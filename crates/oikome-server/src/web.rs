// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The owner's HTTP API (`oikome serve --web`, architecture v3 §13.4):
//! `POST /api/<operation>` with a JSON request, answered with JSON or, for
//! rows, an Arrow IPC stream.
//!
//! It listens on loopback only and every caller is the node's owner, so
//! getting in is guarded closely:
//! - **Login.** A one-time token, valid for 10 minutes, is written into the
//!   login link in `.oikome/web/login` (readable by the owner only; never
//!   printed or logged). `oikome web open` opens that link. Using it sets an
//!   HttpOnly, SameSite=Strict session cookie and replaces the token.
//! - **Every request.** `Host` must name this server (DNS rebinding).
//! - **API calls.** `Origin` must be this server, `Sec-Fetch-Site` (when
//!   sent) `same-origin`, `Content-Type` JSON, and the session cookie valid.
//!   Bodies are capped at 1 MiB.
//! - **The app.** With a web root (`--web-root`, the `apps/web` build), GET
//!   serves its files under a strict Content-Security-Policy. Paths are
//!   checked segment by segment and must resolve inside the root; paths
//!   without an extension get `index.html` (the app's own routes).

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oikome_api::{
    Api, ApiError, Caller, ContactInput, DidRef, Empty, ErrorCode, ListAccess, ListShares,
    NewSpace, NodeName, ReadRows, Rows, RunSql, ShareRef, ShareSpace, SpaceRef, SqlResult,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;

/// Default address of the web API.
pub const DEFAULT_WEB_LISTEN: &str = "127.0.0.1:7340";
/// How long a login link works.
pub const LOGIN_TTL: Duration = Duration::from_secs(10 * 60);
/// A session ends after this long without a request.
pub const SESSION_IDLE: Duration = Duration::from_secs(12 * 3600);
/// Largest request body.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
const ARROW_STREAM: &str = "application/vnd.apache.arrow.stream";
const TRUNCATED_HEADER: &str = "oikome-truncated";
/// The app loads only its own files and talks only to this server.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
     form-action 'self'";

/// Where the login link is written, under the node's `.oikome/` directory.
pub fn login_file(oikome_dir: &Path) -> PathBuf {
    oikome_dir.join("web").join("login")
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Write `contents` to `path`, readable by the owner only, replacing it at once.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("login file has no directory"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

struct LoginToken {
    value: String,
    expires: Instant,
}

struct WebState {
    api: Api,
    /// `Host` values this server answers to.
    hosts: [String; 3],
    port: u16,
    cookie: String,
    login_file: PathBuf,
    /// The app's files (`apps/web/dist`), canonicalized.
    web_root: Option<PathBuf>,
    login_ttl: Duration,
    login: Mutex<LoginToken>,
    /// Session id -> last request.
    sessions: Mutex<HashMap<String, Instant>>,
}

/// The web API of one node.
#[derive(Clone)]
pub struct WebApp {
    state: Arc<WebState>,
}

impl std::fmt::Debug for WebApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebApp")
            .field("port", &self.state.port)
            .finish_non_exhaustive()
    }
}

impl WebApp {
    /// The web API for `api` at `addr`, which must be a loopback address.
    /// Writes the first login link to `login_file`.
    pub fn new(api: Api, addr: SocketAddr, login_file: PathBuf) -> std::io::Result<Self> {
        Self::with_login_ttl(api, addr, login_file, LOGIN_TTL)
    }

    /// Like [`Self::new`], with another lifetime for login links.
    pub fn with_login_ttl(
        api: Api,
        addr: SocketAddr,
        login_file: PathBuf,
        login_ttl: Duration,
    ) -> std::io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("the web API listens on loopback only, not {addr}"),
            ));
        }
        let port = addr.port();
        let app = Self {
            state: Arc::new(WebState {
                api,
                hosts: [
                    format!("127.0.0.1:{port}"),
                    format!("localhost:{port}"),
                    format!("[::1]:{port}"),
                ],
                port,
                cookie: format!("oikome_session_{port}"),
                login_file,
                web_root: None,
                login_ttl,
                login: Mutex::new(LoginToken {
                    value: String::new(),
                    expires: Instant::now(),
                }),
                sessions: Mutex::new(HashMap::new()),
            }),
        };
        app.state.rotate_login()?;
        Ok(app)
    }

    /// Serve the app's files from `root` (the `apps/web` build), which must
    /// hold an `index.html`.
    pub fn with_web_root(mut self, root: &Path) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.join("index.html").is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "{} has no index.html (build apps/web first)",
                    root.display()
                ),
            ));
        }
        let state = Arc::get_mut(&mut self.state).ok_or_else(|| {
            std::io::Error::other("set the web root before serving or cloning the app")
        })?;
        state.web_root = Some(root);
        Ok(self)
    }

    /// The HTTP routes, for serving or for calling directly in tests.
    pub fn router(&self) -> Router {
        Router::new()
            .fallback(handle)
            .with_state(self.state.clone())
    }

    /// Serve on `listener` until `shutdown` resolves, then delete the login link.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let state = self.state.clone();
        // Replace expired login links and forget idle sessions.
        let housekeeping = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                let expired = state.lock_login().expires <= Instant::now();
                if expired {
                    if let Err(e) = state.rotate_login() {
                        tracing::warn!(error = %e, "could not write the login link");
                    }
                }
                state
                    .lock_sessions()
                    .retain(|_, last| last.elapsed() < SESSION_IDLE);
            }
        });
        let result = axum::serve(listener, self.router())
            .with_graceful_shutdown(shutdown)
            .await;
        housekeeping.abort();
        let _ = std::fs::remove_file(&self.state.login_file);
        result
    }
}

impl WebState {
    fn lock_login(&self) -> std::sync::MutexGuard<'_, LoginToken> {
        self.login.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Make a new one-time token and write its link.
    fn rotate_login(&self) -> std::io::Result<()> {
        let value = random_token()?;
        let link = format!("http://127.0.0.1:{}/login?token={value}\n", self.port);
        let mut login = self.lock_login();
        write_private(&self.login_file, &link)?;
        *login = LoginToken {
            value,
            expires: Instant::now() + self.login_ttl,
        };
        Ok(())
    }

    /// The `Host` header, if it names this server.
    fn host<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        let host = headers.get(header::HOST)?.to_str().ok()?;
        self.hosts.iter().any(|h| h == host).then_some(host)
    }

    fn session_ok(&self, headers: &HeaderMap) -> bool {
        let Some(id) = cookie(headers, &self.cookie) else {
            return false;
        };
        let mut sessions = self.lock_sessions();
        match sessions.get_mut(id) {
            Some(last) if last.elapsed() < SESSION_IDLE => {
                *last = Instant::now();
                true
            }
            Some(_) => {
                sessions.remove(id);
                false
            }
            None => false,
        }
    }

    fn login(&self, query: Option<&str>) -> Response {
        let token = query.and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
        let valid = {
            let login = self.lock_login();
            token.is_some_and(|t| {
                login.expires > Instant::now()
                    && bool::from(t.as_bytes().ct_eq(login.value.as_bytes()))
            })
        };
        if !valid {
            return plain(
                StatusCode::UNAUTHORIZED,
                "This login link is invalid, expired or already used. Run `oikome web open` for a new one.\n",
            );
        }
        if let Err(e) = self.rotate_login() {
            tracing::warn!(error = %e, "could not write the login link");
            return plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Login failed on the node.\n",
            );
        }
        let session = match random_token() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "could not create a session");
                return plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Login failed on the node.\n",
                );
            }
        };
        self.lock_sessions().insert(session.clone(), Instant::now());
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::SEE_OTHER;
        let headers = response.headers_mut();
        headers.insert(header::LOCATION, HeaderValue::from_static("/"));
        if let Ok(value) = HeaderValue::from_str(&format!(
            "{}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
            self.cookie,
            SESSION_IDLE.as_secs()
        )) {
            headers.insert(header::SET_COOKIE, value);
        }
        secure_headers(headers);
        response
    }

    /// Checks that apply to API calls only, after `Host`.
    fn api_request_ok(&self, headers: &HeaderMap, host: &str) -> Result<(), ApiError> {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
        if origin != Some(format!("http://{host}").as_str()) {
            return Err(ApiError::permission_denied("cross-origin request"));
        }
        if let Some(site) = headers.get("sec-fetch-site") {
            if site.as_bytes() != b"same-origin" {
                return Err(ApiError::permission_denied("cross-site request"));
            }
        }
        let json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.split(';').next().map(str::trim) == Some("application/json"));
        if !json {
            return Err(ApiError::invalid("send the request as application/json"));
        }
        if !self.session_ok(headers) {
            return Err(ApiError::new(
                ErrorCode::Unauthenticated,
                "log in with `oikome web open`",
            ));
        }
        Ok(())
    }

    async fn call(&self, op: &str, body: Bytes) -> Response {
        let caller = Caller::Owner;
        let api = &self.api;
        macro_rules! json_op {
            ($method:ident, $request:ty) => {
                match parse::<$request>(&body) {
                    Ok(request) => json(api.$method(&caller, request).await),
                    Err(e) => error(e),
                }
            };
        }
        match op {
            "node_info" => json_op!(node_info, Empty),
            "rename_node" => json_op!(rename_node, NodeName),
            "recovery_key" => json_op!(recovery_key, Empty),
            "confirm_recovery_key" => json_op!(confirm_recovery_key, Empty),
            "list_spaces" => json_op!(list_spaces, Empty),
            "create_space" => json_op!(create_space, NewSpace),
            "drop_space" => json_op!(drop_space, SpaceRef),
            "list_tables" => json_op!(list_tables, SpaceRef),
            "read_rows" => match parse::<ReadRows>(&body) {
                Ok(request) => match api.read_rows(&caller, request).await {
                    Ok(rows) => arrow(rows),
                    Err(e) => error(e),
                },
                Err(e) => error(e),
            },
            "run_sql" => match parse::<RunSql>(&body) {
                Ok(request) => match api.run_sql(&caller, request).await {
                    Ok(SqlResult::Rows(rows)) => arrow(rows),
                    Ok(SqlResult::Done(done)) => json(Ok(done)),
                    Err(e) => error(e),
                },
                Err(e) => error(e),
            },
            "list_contacts" => json_op!(list_contacts, Empty),
            "add_contact" => json_op!(add_contact, ContactInput),
            "rename_contact" => json_op!(rename_contact, ContactInput),
            "remove_contact" => json_op!(remove_contact, DidRef),
            "share_space" => json_op!(share_space, ShareSpace),
            "list_shares" => json_op!(list_shares, ListShares),
            "revoke_share" => json_op!(revoke_share, ShareRef),
            "list_access" => json_op!(list_access, ListAccess),
            _ => error(ApiError::not_found(format!("no operation named '{op}'"))),
        }
    }
}

/// The value of cookie `name` in the request's `Cookie` headers.
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == name).then_some(v)
        })
}

fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    let body: &[u8] = if body.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        body
    };
    serde_json::from_slice(body).map_err(|e| ApiError::invalid(format!("request: {e}")))
}

fn secure_headers(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

fn respond(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    secure_headers(headers);
    response
}

fn plain(status: StatusCode, text: &str) -> Response {
    respond(
        status,
        "text/plain; charset=utf-8",
        text.as_bytes().to_vec(),
    )
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The file under `root` that `path` names, or `None`. Paths with escapes,
/// backslashes, empty, dot or hidden segments are refused before touching
/// the disk, and the file found must still lie inside `root` once symbolic
/// links are resolved. A path without an extension is one of the app's own
/// routes and gets `index.html`.
fn app_file(root: &Path, path: &str) -> Option<PathBuf> {
    if path.contains(['%', '\\', '\0']) {
        return None;
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.iter().any(|s| s.starts_with('.')) {
        return None;
    }
    let file = match segments.last() {
        Some(last) if last.contains('.') => {
            segments.iter().fold(root.to_path_buf(), |p, s| p.join(s))
        }
        _ => root.join("index.html"),
    };
    let file = file.canonicalize().ok()?;
    (file.starts_with(root) && file.is_file()).then_some(file)
}

async fn serve_app_file(root: &Path, path: &str) -> Response {
    let Some(file) = app_file(root, path) else {
        return error(ApiError::not_found("not found"));
    };
    let body = match tokio::fs::read(&file).await {
        Ok(body) => body,
        Err(_) => return error(ApiError::not_found("not found")),
    };
    let mut response = respond(StatusCode::OK, content_type(&file), body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    // Built assets are named by their content; everything else may change.
    if path.starts_with("/assets/") {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response
}

fn status_of(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Invalid => StatusCode::BAD_REQUEST,
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn error(e: ApiError) -> Response {
    let body = serde_json::to_vec(&e).unwrap_or_default();
    respond(status_of(e.code), "application/json", body)
}

fn json<T: Serialize>(result: Result<T, ApiError>) -> Response {
    match result.and_then(|value| serde_json::to_vec(&value).map_err(ApiError::internal)) {
        Ok(body) => respond(StatusCode::OK, "application/json", body),
        Err(e) => error(e),
    }
}

fn arrow(rows: Rows) -> Response {
    let mut response = respond(StatusCode::OK, ARROW_STREAM, rows.ipc);
    if rows.truncated {
        response
            .headers_mut()
            .insert(TRUNCATED_HEADER, HeaderValue::from_static("true"));
    }
    response
}

async fn handle(State(state): State<Arc<WebState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Some(host) = state.host(&parts.headers) else {
        return error(ApiError::permission_denied("unexpected Host"));
    };
    let path = parts.uri.path();
    match (&parts.method, path) {
        (&Method::GET, "/login") => state.login(parts.uri.query()),
        (&Method::GET, _) if path.starts_with("/api/") => {
            error(ApiError::not_found("API calls use POST"))
        }
        (&Method::GET, _) if state.web_root.is_some() => {
            let root = state
                .web_root
                .as_deref()
                .unwrap_or(Path::new("/nonexistent"));
            serve_app_file(root, path).await
        }
        (&Method::GET, "/") => plain(
            StatusCode::OK,
            "Oikome is running without its app (start it with --web-root apps/web/dist); \
             the API is at POST /api/<operation>.\n",
        ),
        (&Method::POST, _) if path.starts_with("/api/") => {
            if let Err(e) = state.api_request_ok(&parts.headers, host) {
                return error(e);
            }
            let body = match to_bytes(body, MAX_BODY_BYTES).await {
                Ok(body) => body,
                Err(_) => {
                    return error(ApiError::invalid(format!(
                        "request bodies are limited to {} KiB",
                        MAX_BODY_BYTES / 1024
                    )))
                }
            };
            state.call(&path["/api/".len()..], body).await
        }
        _ => error(ApiError::not_found("not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_are_found_by_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; oikome_session_7340=abc; b=2"),
        );
        assert_eq!(cookie(&headers, "oikome_session_7340"), Some("abc"));
        assert_eq!(cookie(&headers, "oikome_session_7341"), None);
    }

    #[test]
    fn app_files_stay_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("index.html"), "<!doctype html>").unwrap();
        std::fs::create_dir(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/app-1a2b.js"), "").unwrap();
        std::fs::write(dir.path().parent().unwrap().join("secret.txt"), "").ok();
        assert_eq!(app_file(&root, "/").unwrap(), root.join("index.html"));
        assert_eq!(
            app_file(&root, "/sharing").unwrap(),
            root.join("index.html")
        );
        assert_eq!(
            app_file(&root, "/assets/app-1a2b.js").unwrap(),
            root.join("assets/app-1a2b.js")
        );
        for bad in [
            "/../secret.txt",
            "/assets/../../secret.txt",
            "/%2e%2e/secret.txt",
            "/.hidden.txt",
            "/assets\\..\\x.js",
            "/missing.js",
        ] {
            assert!(app_file(&root, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn empty_bodies_parse_as_empty_objects() {
        let empty: Empty = parse(&Bytes::from_static(b"")).unwrap();
        assert_eq!(empty, Empty {});
        assert!(parse::<SpaceRef>(&Bytes::from_static(b"{}")).is_err());
    }
}
