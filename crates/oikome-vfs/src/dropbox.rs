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

//! Signing in to Dropbox (3.4c, architecture v3 §7.5): OAuth 2.0 with PKCE
//! and a refresh token for a desktop app. Only the file permissions Oikome
//! uses are asked for; whether Oikome sees an app folder or the whole drive
//! is chosen with the app in Dropbox's App Console, not by an OAuth scope.
//! Codes and tokens travel in request bodies, never in an address, and never
//! appear in an error or a log. PKCE needs no App secret.

use std::fmt;
use std::time::Duration;

use zeroize::Zeroizing;

/// The permissions Oikome uses: account room, and reading and writing the
/// files and their metadata within the access the app was given.
pub const FILES_SCOPE: &str =
    "account_info.read files.content.read files.content.write files.metadata.read files.metadata.write";
const TIMEOUT: Duration = Duration::from_secs(30);

/// A failed exchange with Dropbox. Messages never hold a code or token.
#[derive(Debug, thiserror::Error)]
pub enum DropboxError {
    #[error("Dropbox refused: {0}")]
    Refused(String),
    #[error("{0}")]
    Other(String),
}

/// Where sign-in and the account API happen; Dropbox's own unless a test says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth: "https://www.dropbox.com/oauth2/authorize".into(),
            token: "https://api.dropboxapi.com/oauth2/token".into(),
            api: "https://api.dropboxapi.com/2".into(),
        }
    }
}

/// What a sign-in gave.
pub struct SignedIn {
    pub refresh_token: Zeroizing<String>,
}

impl fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedIn")
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

/// The Dropbox app Oikome signs in with, registered by its owner.
#[derive(Clone)]
pub struct DropboxClient {
    client_id: String,
    endpoints: Endpoints,
}

impl fmt::Debug for DropboxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropboxClient")
            .field("client_id", &self.client_id)
            .field("endpoints", &self.endpoints)
            .finish()
    }
}

#[derive(serde::Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(serde::Deserialize)]
struct Refusal {
    error: String,
}

#[derive(serde::Deserialize)]
struct SpaceUsage {
    used: u64,
    allocation: Allocation,
}

#[derive(serde::Deserialize)]
struct Allocation {
    allocated: u64,
}

impl DropboxClient {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            endpoints: Endpoints::default(),
        }
    }

    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Dropbox's authorization page, with offline access and PKCE. With no
    /// redirect URI, Dropbox shows the authorization code for the owner to
    /// paste into Oikome. App folder or full Dropbox access is set in its
    /// console, so it is not a scope added here.
    pub fn sign_in_url(&self, challenge: &str) -> String {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("response_type", "code"),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("token_access_type", "offline"),
            ("scope", FILES_SCOPE),
        ];
        match url::Url::parse_with_params(&self.endpoints.auth, params) {
            Ok(url) => url.into(),
            Err(_) => self.endpoints.auth.clone(),
        }
    }

    fn http() -> Result<reqwest::Client, DropboxError> {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| DropboxError::Other(e.without_url().to_string()))
    }

    async fn token(&self, form: &[(&str, &str)]) -> Result<TokenAnswer, DropboxError> {
        let response = Self::http()?
            .post(&self.endpoints.token)
            .form(form)
            .send()
            .await
            .map_err(|e| DropboxError::Other(format!("reaching Dropbox: {}", e.without_url())))?;
        let status = response.status();
        let body = Zeroizing::new(
            response
                .bytes()
                .await
                .map_err(|e| {
                    DropboxError::Other(format!("reading Dropbox's answer: {}", e.without_url()))
                })?
                .to_vec(),
        );
        if !status.is_success() {
            return Err(match serde_json::from_slice::<Refusal>(&body) {
                Ok(refusal) => DropboxError::Refused(refusal.error),
                Err(_) => DropboxError::Other(format!("Dropbox answered HTTP {status}")),
            });
        }
        serde_json::from_slice(&body)
            .map_err(|_| DropboxError::Other("Dropbox's answer was not understood".into()))
    }

    /// Trade the pasted code for a refresh token, with the verifier only this
    /// node holds. No redirect URI was used, so none is sent here either.
    pub async fn exchange(&self, code: &str, verifier: &str) -> Result<SignedIn, DropboxError> {
        let answer = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("client_id", self.client_id.as_str()),
            ])
            .await?;
        let refresh_token = answer
            .refresh_token
            .map(Zeroizing::new)
            .ok_or_else(|| DropboxError::Other("Dropbox sent no refresh token".into()))?;
        Ok(SignedIn { refresh_token })
    }

    /// The account's room, with an access token made from the refresh token.
    pub async fn room(&self, refresh_token: &str) -> Result<(u64, u64), DropboxError> {
        let answer = self
            .token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", self.client_id.as_str()),
            ])
            .await?;
        let access = Zeroizing::new(
            answer
                .access_token
                .ok_or_else(|| DropboxError::Other("Dropbox sent no access token".into()))?,
        );
        let response = Self::http()?
            .post(format!("{}/users/get_space_usage", self.endpoints.api))
            .bearer_auth(access.as_str())
            .send()
            .await
            .map_err(|e| DropboxError::Other(format!("reaching Dropbox: {}", e.without_url())))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DropboxError::Other(format!(
                "Dropbox answered HTTP {status}"
            )));
        }
        let body = response.bytes().await.map_err(|e| {
            DropboxError::Other(format!("reading Dropbox's answer: {}", e.without_url()))
        })?;
        let usage: SpaceUsage = serde_json::from_slice(&body)
            .map_err(|_| DropboxError::Other("Dropbox's answer was not understood".into()))?;
        Ok((usage.used, usage.allocation.allocated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn sign_in_asks_for_a_code_with_offline_file_access_and_pkce() {
        let client = DropboxClient::new("app-key");
        let url = url::Url::parse(&client.sign_in_url("the-challenge")).unwrap();
        assert_eq!(url.host_str(), Some("www.dropbox.com"));
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["scope"], FILES_SCOPE);
        assert!(!params.contains_key("state"));
        assert!(!params.contains_key("redirect_uri"));
        assert_eq!(params["code_challenge"], "the-challenge");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["token_access_type"], "offline");
        assert_eq!(params["response_type"], "code");
        assert!(!url.as_str().contains("app-secret"));
        assert!(!format!("{client:?}").contains("app-secret"));
    }

    async fn token_endpoint(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let answer = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(answer.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, served)
    }

    fn client_at(token: &str) -> DropboxClient {
        DropboxClient::new("the-client").with_endpoints(Endpoints {
            auth: "http://127.0.0.1/auth".into(),
            token: token.into(),
            api: "http://127.0.0.1:1/2".into(),
        })
    }

    #[tokio::test]
    async fn the_code_is_exchanged_in_the_body() {
        let (url, served) = token_endpoint(
            200,
            r#"{"access_token":"at","refresh_token":"rt-1","token_type":"Bearer","expires_in":3599}"#,
        )
        .await;
        let signed = client_at(&url)
            .exchange("the-code", "the-verifier")
            .await
            .unwrap();
        assert_eq!(signed.refresh_token.as_str(), "rt-1");
        let request = served.await.unwrap();
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("POST /token HTTP/1.1"), "{head}");
        let line = head.lines().next().unwrap();
        assert!(!line.contains("the-code") && !line.contains("the-secret"));
        let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "the-code");
        assert_eq!(form["code_verifier"], "the-verifier");
        assert!(!form.contains_key("client_secret"));
        assert!(!form.contains_key("redirect_uri"));
    }

    #[tokio::test]
    async fn a_refusal_says_so_without_the_secrets() {
        let (url, _) = token_endpoint(
            400,
            r#"{"error":"invalid_grant","error_description":"Bad Request"}"#,
        )
        .await;
        let refused = client_at(&url)
            .exchange("the-code", "the-verifier")
            .await
            .unwrap_err();
        assert!(matches!(&refused, DropboxError::Refused(e) if e == "invalid_grant"));
        let shown = refused.to_string();
        assert!(!shown.contains("the-code") && !shown.contains("the-secret"));
    }

    async fn answering(
        answers: Vec<&'static str>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let mut lines = Vec::new();
            for body in answers {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                lines.push(request.lines().next().unwrap_or_default().to_string());
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(answer.as_bytes()).await.unwrap();
            }
            lines
        });
        (base, served)
    }

    #[tokio::test]
    async fn the_room_is_read_with_an_access_token_from_the_refresh_token() {
        let (base, served) = answering(vec![
            r#"{"access_token":"at-1","expires_in":3599,"token_type":"Bearer"}"#,
            r#"{"used":5368709120,"allocation":{"allocated":16106127360}}"#,
        ])
        .await;
        let client = DropboxClient::new("the-client").with_endpoints(Endpoints {
            auth: format!("{base}/auth"),
            token: format!("{base}/token"),
            api: format!("{base}/2"),
        });
        assert_eq!(
            client.room("rt-1").await.unwrap(),
            (5_368_709_120, 16_106_127_360)
        );
        let lines = served.await.unwrap();
        assert!(lines[0].starts_with("POST /token "), "{lines:?}");
        assert!(
            lines[1].starts_with("POST /2/users/get_space_usage "),
            "{lines:?}"
        );
        assert!(!lines
            .iter()
            .any(|l| l.contains("rt-1") || l.contains("at-1")));
    }
}
