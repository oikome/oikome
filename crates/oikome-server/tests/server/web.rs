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

//! The owner's web API over HTTP (M1b): the one-time login link, the
//! session cookie and the checks every request goes through.

use crate::common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use axum::Router;
use common::Node;
use http_body_util::BodyExt;
use oikome_api::Api;
use oikome_node::SecretKind;
use oikome_server::web::{login_file, WebApp, LOGIN_TTL, MAX_BODY_BYTES};
use oikome_vfs::google::{challenge_of, Endpoints, DRIVE_SCOPE};
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:7999";
const ORIGIN: &str = "http://127.0.0.1:7999";

struct Web {
    node: Node,
    router: Router,
    login: PathBuf,
}

fn web(ttl: Duration) -> Web {
    web_with(ttl, Endpoints::default())
}

/// With Google's sign-in at `google`.
fn web_with(ttl: Duration, google: Endpoints) -> Web {
    let node = Node::new(41);
    let login = login_file(&node.catalog.oikome_dir());
    let api = Api::new(node.catalog.clone(), node.engines.clone())
        .with_access_log(node.access_log.clone())
        .with_google_endpoints(google);
    let addr: SocketAddr = HOST.parse().unwrap();
    let app = WebApp::with_login_ttl(api, addr, login.clone(), ttl).unwrap();
    Web {
        router: app.router(),
        node,
        login,
    }
}

impl Web {
    fn link(&self) -> String {
        let link = std::fs::read_to_string(&self.login).unwrap();
        link.trim()
            .strip_prefix("http://127.0.0.1:7999")
            .unwrap()
            .to_string()
    }

    async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(request).await.unwrap()
    }

    async fn get(&self, path: &str) -> Response<Body> {
        self.send(
            Request::get(path)
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    /// Log in with the current link; returns the `name=value` cookie.
    async fn login(&self) -> String {
        let response = self.get(&self.link()).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let set_cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        set_cookie.split(';').next().unwrap().to_string()
    }

    /// A well-formed API call, without its cookie.
    fn api(&self, op: &str) -> axum::http::request::Builder {
        Request::post(format!("/api/{op}"))
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
    }
}

async fn json(response: Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn a_login_link_works_once_and_opens_a_session() {
    let w = web(LOGIN_TTL);
    let first = w.link();
    let response = w.get(&first).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("oikome_session_7999="));

    // Used: refused, and the file holds a new link.
    assert_eq!(w.get(&first).await.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(w.link(), first);

    let response = w
        .send(
            w.api("node_info")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(json(response).await["did"]
        .as_str()
        .unwrap()
        .starts_with("did:key:z"));
}

#[tokio::test]
async fn expired_and_forged_links_are_refused() {
    let w = web(Duration::ZERO);
    assert_eq!(w.get(&w.link()).await.status(), StatusCode::UNAUTHORIZED);
    let w = web(LOGIN_TTL);
    assert_eq!(
        w.get("/login?token=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(w.get("/login").await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_calls_need_this_host_this_origin_json_and_a_session() {
    let w = web(LOGIN_TTL);
    let cookie = w.login().await;
    let web = &w;
    let call = move |builder: axum::http::request::Builder, body: Body| {
        let request = builder.body(body).unwrap();
        async move { web.send(request).await.status() }
    };

    // Another Host (DNS rebinding).
    let request = Request::post("/api/list_spaces")
        .header(header::HOST, "attacker.example:7999")
        .header(header::ORIGIN, "http://attacker.example:7999")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &cookie);
    assert_eq!(call(request, Body::from("{}")).await, StatusCode::FORBIDDEN);
    // No session.
    assert_eq!(
        call(w.api("list_spaces"), Body::from("{}")).await,
        StatusCode::UNAUTHORIZED
    );
    // Another origin, or none.
    let request = Request::post("/api/list_spaces")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, "http://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &cookie);
    assert_eq!(call(request, Body::from("{}")).await, StatusCode::FORBIDDEN);
    let request = Request::post("/api/list_spaces")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &cookie);
    assert_eq!(call(request, Body::from("{}")).await, StatusCode::FORBIDDEN);
    // Cross-site fetch metadata.
    let request = w
        .api("list_spaces")
        .header(header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-site");
    assert_eq!(call(request, Body::from("{}")).await, StatusCode::FORBIDDEN);
    // Not JSON (what a plain HTML form would send).
    let request = Request::post("/api/list_spaces")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::COOKIE, &cookie);
    assert_eq!(
        call(request, Body::from("{}")).await,
        StatusCode::BAD_REQUEST
    );
    // Too large.
    let big = vec![b' '; MAX_BODY_BYTES + 1];
    assert_eq!(
        call(
            w.api("list_spaces").header(header::COOKIE, &cookie),
            Body::from(big)
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // With everything in place.
    assert_eq!(
        call(
            w.api("list_spaces").header(header::COOKIE, &cookie),
            Body::from("{}")
        )
        .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn errors_come_back_as_json_with_a_code() {
    let w = web(LOGIN_TTL);
    let cookie = w.login().await;
    let response = w
        .send(
            w.api("create_space")
                .header(header::COOKIE, &cookie)
                .body(Body::from(r#"{"name":"../escape"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json(response).await["code"], "invalid");

    let response = w
        .send(
            w.api("no_such_operation")
                .header(header::COOKIE, &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json(response).await["code"], "not_found");

    let response = w
        .send(
            w.api("drop_space")
                .header(header::COOKIE, &cookie)
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rows_come_back_as_arrow() {
    let w = web(LOGIN_TTL);
    let cookie = w.login().await;
    for (op, body) in [
        ("create_space", r#"{"name":"notes"}"#),
        (
            "run_sql",
            r#"{"sql":"CREATE TABLE notes.memos (id BIGINT NOT NULL); INSERT INTO notes.memos VALUES (1), (2)"}"#,
        ),
    ] {
        let response = w
            .send(
                w.api(op)
                    .header(header::COOKIE, &cookie)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{op}");
    }
    let response = w
        .send(
            w.api("read_rows")
                .header(header::COOKIE, &cookie)
                .body(Body::from(r#"{"space":"notes","table":"memos"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/vnd.apache.arrow.stream"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let rows: usize = arrow_ipc::reader::StreamReader::try_new(bytes.as_ref(), None)
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn the_web_api_listens_on_loopback_only() {
    let node = Node::new(42);
    let api = Api::new(node.catalog.clone(), node.engines.clone());
    let login = login_file(&node.catalog.oikome_dir());
    assert!(WebApp::new(api, "0.0.0.0:7340".parse().unwrap(), login).is_err());
}

#[tokio::test]
async fn the_app_is_served_with_a_strict_policy() {
    let node = Node::new(43);
    let pages = tempfile::tempdir().unwrap();
    std::fs::write(
        pages.path().join("index.html"),
        "<!doctype html><title>Oikome</title>",
    )
    .unwrap();
    std::fs::create_dir(pages.path().join("assets")).unwrap();
    std::fs::write(pages.path().join("assets/app-1a2b.js"), "export {}").unwrap();
    let api = Api::new(node.catalog.clone(), node.engines.clone());
    let app = WebApp::new(
        api,
        HOST.parse().unwrap(),
        login_file(&node.catalog.oikome_dir()),
    )
    .unwrap()
    .with_web_root(pages.path())
    .unwrap();
    let router = app.router();
    let get = |path: &str| {
        let request = Request::get(path)
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        router.clone().oneshot(request)
    };

    // The app's own routes get index.html.
    for path in ["/", "/sharing", "/collections/notes"] {
        let response = get(path).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("default-src 'self'"));
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let asset = get("/assets/app-1a2b.js").await.unwrap();
    assert_eq!(asset.status(), StatusCode::OK);
    assert!(asset.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
    assert!(asset.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("immutable"));
    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/missing.js",
        "/api/node_info",
    ] {
        assert_eq!(
            get(path).await.unwrap().status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }

    // A directory without index.html is refused up front.
    let empty = tempfile::tempdir().unwrap();
    let api = Api::new(node.catalog.clone(), node.engines.clone());
    assert!(WebApp::new(
        api,
        HOST.parse().unwrap(),
        login_file(&node.catalog.oikome_dir())
    )
    .unwrap()
    .with_web_root(empty.path())
    .is_err());
}

// ----- Connecting Google Drive (3.4c) -----

/// A token endpoint standing in for Google's: it keeps each form it gets
/// and answers for `drive.file`, without a refresh token, so no drive is
/// reached from here.
async fn google_token_endpoint() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use axum::routing::post;
    let forms = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = forms.clone();
    let router = Router::new().route(
        "/token",
        post(move |body: axum::body::Bytes| {
            let kept = kept.clone();
            async move {
                kept.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&body).into_owned());
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    format!(
                        r#"{{"access_token":"at","expires_in":3599,"scope":"{DRIVE_SCOPE}","token_type":"Bearer"}}"#
                    ),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });
    (url, forms)
}

/// The parameters of an address's query, decoded.
fn query_of(url: &str) -> std::collections::HashMap<String, String> {
    let decode = |text: &str| {
        let bytes = text.replace('+', " ").into_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    };
    url.split_once('?')
        .map(|(_, q)| {
            q.split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (decode(k), decode(v)))
                .collect()
        })
        .unwrap_or_default()
}

impl Web {
    async fn start_google_drive(&self, cookie: &str, body: &str) -> Response<Body> {
        self.send(
            self.api("start_google_drive")
                .header(header::COOKIE, cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    /// Where Google's way back with `query` sends the owner.
    async fn back_from_google(&self, query: &str) -> String {
        let response = self.get(&format!("/oauth/google/callback?{query}")).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }
}

#[tokio::test]
async fn google_drive_needs_the_owners_own_google_app() {
    let web = web(LOGIN_TTL);
    let cookie = web.login().await;
    for body in [
        r#"{"returnTo":"settings"}"#,
        r#"{"returnTo":"settings","clientId":"","clientSecret":""}"#,
        r#"{"returnTo":"settings","clientId":"id.apps.googleusercontent.com","clientSecret":" "}"#,
    ] {
        let response = web.start_google_drive(&cookie, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json(response).await["reason"], "google_client_incomplete");
    }
}

#[tokio::test]
async fn a_google_sign_in_comes_back_once_by_its_state_and_leaves_nothing_when_it_fails() {
    let (token_url, forms) = google_token_endpoint().await;
    let web = web_with(
        LOGIN_TTL,
        Endpoints {
            token: token_url,
            ..Endpoints::default()
        },
    );
    let cookie = web.login().await;
    let app = r#""clientId":"the-client.apps.googleusercontent.com","clientSecret":"the-secret""#;

    // Only the owner starts one.
    let response = web
        .send(
            web.api("start_google_drive")
                .body(Body::from(format!(r#"{{"returnTo":"settings",{app}}}"#)))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let start = |return_to: &'static str| {
        let web = &web;
        let cookie = cookie.clone();
        async move {
            let body = format!(r#"{{"returnTo":"{return_to}","makeDefault":true,{app}}}"#);
            let response = web.start_google_drive(&cookie, &body).await;
            assert_eq!(response.status(), StatusCode::OK);
            json(response).await["url"].as_str().unwrap().to_string()
        }
    };
    let url = start("onboarding").await;
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    let params = query_of(&url);
    // The owner's own app; back to this server, on loopback; only the
    // files Oikome makes.
    assert_eq!(params["client_id"], "the-client.apps.googleusercontent.com");
    assert_eq!(
        params["redirect_uri"],
        "http://127.0.0.1:7999/oauth/google/callback"
    );
    assert_eq!(params["scope"], DRIVE_SCOPE);
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(!url.contains("the-secret"));
    let state = params["state"].clone();
    assert!(state.len() >= 43);

    // A state the node never made, or none.
    assert_eq!(
        web.back_from_google("state=made-up&code=x").await,
        "/settings?google=failed"
    );
    assert_eq!(
        web.back_from_google("code=x").await,
        "/settings?google=failed"
    );
    // The owner said no; the state is used up then.
    assert_eq!(
        web.back_from_google(&format!("state={state}&error=access_denied"))
            .await,
        "/onboarding?google=denied"
    );
    assert_eq!(
        web.back_from_google(&format!("state={state}&code=x")).await,
        "/settings?google=failed"
    );
    assert!(forms.lock().unwrap().is_empty());

    // A code: traded with the verifier only the node holds, in the body,
    // with the owner's app.
    let url = start("settings").await;
    let params = query_of(&url);
    let back = web
        .back_from_google(&format!(
            "state={}&code=4%2F0the-code&scope={}",
            params["state"], DRIVE_SCOPE
        ))
        .await;
    let forms = forms.lock().unwrap().clone();
    assert_eq!(forms.len(), 1);
    let form = query_of(&format!("?{}", forms[0]));
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "4/0the-code");
    assert_eq!(
        challenge_of(&form["code_verifier"]),
        params["code_challenge"]
    );
    assert_eq!(
        form["redirect_uri"],
        "http://127.0.0.1:7999/oauth/google/callback"
    );
    assert_eq!(form["client_id"], "the-client.apps.googleusercontent.com");
    assert_eq!(form["client_secret"], "the-secret");
    // Google sent no refresh token: nothing is kept, the app's secret neither.
    assert_eq!(back, "/settings?google=failed");
    assert!(web.node.catalog.list_storages().is_empty());
    let secrets = web.node.engines.secret_backend().unwrap();
    for kind in [SecretKind::Token, SecretKind::Password] {
        assert!(secrets
            .get(&kind.account("google_drive"))
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn the_google_app_is_remembered_so_the_next_drive_asks_for_nothing() {
    let web = web(LOGIN_TTL);
    let cookie = web.login().await;
    let google_app = || async {
        json(
            web.send(
                web.api("google_app")
                    .header(header::COOKIE, &cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await,
        )
        .await
    };
    assert_eq!(google_app().await["clientId"], serde_json::Value::Null);
    // Remembered after a drive connected, or given on the command line.
    let session = oikome_node::OikomeSession::new(
        web.node.catalog.clone(),
        web.node.engines.clone(),
        oikome_node::SessionConfig::default(),
    );
    session
        .set_google_app("kept.apps.googleusercontent.com", "kept-secret")
        .unwrap();
    // Only its ID comes back, never the secret.
    let app = google_app().await;
    assert_eq!(app["clientId"], "kept.apps.googleusercontent.com");
    assert!(!app.to_string().contains("kept-secret"));
    let response = web
        .start_google_drive(&cookie, r#"{"returnTo":"settings"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let url = json(response).await["url"].as_str().unwrap().to_string();
    assert_eq!(
        query_of(&url)["client_id"],
        "kept.apps.googleusercontent.com"
    );
    assert_eq!(query_of(&url)["scope"], DRIVE_SCOPE);
    // Reading the rest of the drive, to view folders there, only when asked.
    let response = web
        .start_google_drive(&cookie, r#"{"returnTo":"settings","readAll":true}"#)
        .await;
    let url = json(response).await["url"].as_str().unwrap().to_string();
    assert_eq!(
        query_of(&url)["scope"],
        format!("{DRIVE_SCOPE} {}", oikome_vfs::google::DRIVE_READ_ALL_SCOPE)
    );
    // An ID without its secret is not an app.
    let response = web
        .start_google_drive(&cookie, r#"{"returnTo":"settings","clientId":"other"}"#)
        .await;
    assert_eq!(json(response).await["reason"], "google_client_incomplete");
}

#[tokio::test]
async fn googles_way_back_answers_this_host_only() {
    let web = web(LOGIN_TTL);
    let response = web
        .send(
            Request::get("/oauth/google/callback?state=x&code=y")
                .header(header::HOST, "evil.example:7999")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
