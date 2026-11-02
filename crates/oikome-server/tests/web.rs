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

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use axum::Router;
use common::Node;
use http_body_util::BodyExt;
use oikome_api::Api;
use oikome_server::web::{login_file, WebApp, LOGIN_TTL, MAX_BODY_BYTES};
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:7999";
const ORIGIN: &str = "http://127.0.0.1:7999";

struct Web {
    _node: Node,
    router: Router,
    login: PathBuf,
}

fn web(ttl: Duration) -> Web {
    let node = Node::new(41);
    let login = login_file(&node.catalog.oikome_dir());
    let api = Api::new(node.catalog.clone(), node.engines.clone())
        .with_access_log(node.access_log.clone());
    let addr: SocketAddr = HOST.parse().unwrap();
    let app = WebApp::with_login_ttl(api, addr, login.clone(), ttl).unwrap();
    Web {
        router: app.router(),
        _node: node,
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
