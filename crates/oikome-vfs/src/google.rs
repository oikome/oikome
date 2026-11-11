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

//! Signing in to Google Drive (architecture v3 §7.5): OAuth 2.0 for an app
//! on the owner's computer (RFC 8252), with the code sent back to this
//! computer's loopback address and PKCE (RFC 7636) binding it to the node
//! that asked.
//!
//! Only the scope `drive.file` is asked for: Oikome sees the files it made,
//! never the rest of the drive. The refresh token Google returns is the
//! storage's secret; it, the code and the client secret travel in request
//! bodies, never in an address, and never appear in an error or a log.
//!
//! Nothing here revokes a token: Google revokes every token of the project
//! with it, the owner's other computers' too. A node forgets its own; the
//! owner takes Oikome's access back in their Google Account.

use std::fmt;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// The scope always asked for: the files Oikome makes itself.
pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
/// Asked for besides when the owner wants to view their other files in
/// place (§7.7): every file of the drive, read only.
pub const DRIVE_READ_ALL_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
/// How long a request to Google may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A failed exchange with Google. Messages never hold a code, token or the
/// client secret.
#[derive(Debug, thiserror::Error)]
pub enum GoogleError {
    /// Google turned the request down: the code expired or was used, the
    /// client is not the one the code was for, or the owner said no.
    #[error("Google refused: {0}")]
    Refused(String),
    /// The owner unticked the permission Oikome needs on Google's page.
    #[error("Oikome was not allowed to keep files on Google Drive")]
    ScopeMissing,
    #[error("{0}")]
    Other(String),
}

/// Where the sign-in happens; Google's own unless a test says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    /// The Drive API, for the drive's room.
    pub drive: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            drive: "https://www.googleapis.com/drive/v3".into(),
        }
    }
}

/// What a sign-in gave: the refresh token, and whether the owner let Oikome
/// read the rest of the drive too.
pub struct SignedIn {
    pub refresh_token: Zeroizing<String>,
    pub reads_all: bool,
}

impl fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedIn")
            .field("refresh_token", &"<redacted>")
            .field("reads_all", &self.reads_all)
            .finish()
    }
}

/// The OAuth client Oikome signs in with: a "Desktop app" client the
/// owner registered in a Google Cloud project of their own, and entered in
/// the apps. Google says such a secret can't be kept secret; it is kept
/// like one all the same, and out of `Debug`.
#[derive(Clone)]
pub struct GoogleClient {
    client_id: String,
    client_secret: Zeroizing<String>,
    endpoints: Endpoints,
}

impl fmt::Debug for GoogleClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("endpoints", &self.endpoints)
            .finish()
    }
}

/// A code verifier (kept by the node) and its S256 challenge (sent to
/// Google with the sign-in).
pub struct Pkce {
    pub verifier: Zeroizing<String>,
    pub challenge: String,
}

/// A new verifier: 32 random bytes, 43 characters (RFC 7636 §4.1).
pub fn pkce() -> Result<Pkce, GoogleError> {
    let verifier = Zeroizing::new(random_text()?);
    let challenge = challenge_of(&verifier);
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// The S256 challenge of `verifier`: BASE64URL(SHA256(verifier)).
pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 32 random bytes as URL-safe text, for a verifier or a sign-in's state.
pub fn random_text() -> Result<String, GoogleError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| GoogleError::Other(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// What Google's token endpoint answers.
#[derive(serde::Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

/// The drive's `about`, with only its room asked for.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct About {
    storage_quota: Quota,
}

/// Bytes, as Google writes 64-bit numbers: in strings. No `limit` is a
/// drive without one.
#[derive(serde::Deserialize)]
struct Quota {
    limit: Option<String>,
    usage: String,
}

/// What it answers when it refuses.
#[derive(serde::Deserialize)]
struct Refusal {
    error: String,
}

impl GoogleClient {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: Zeroizing::new(client_secret.into()),
            endpoints: Endpoints::default(),
        }
    }

    /// Sign in somewhere else than Google: for tests.
    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub(crate) fn client_secret(&self) -> &str {
        &self.client_secret
    }

    /// The address of Google's page that asks the owner to let Oikome keep
    /// files on their drive (and, with `read_all`, read the rest of it), and
    /// then sends them to `redirect_uri` with a code and `state`.
    pub fn sign_in_url(
        &self,
        redirect_uri: &str,
        state: &str,
        challenge: &str,
        read_all: bool,
    ) -> String {
        let scope = if read_all {
            format!("{DRIVE_SCOPE} {DRIVE_READ_ALL_SCOPE}")
        } else {
            DRIVE_SCOPE.to_string()
        };
        let params = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            // A refresh token, and a new one each time: a second sign-in
            // must not come back without one.
            ("access_type", "offline"),
            ("prompt", "consent"),
        ];
        match url::Url::parse_with_params(&self.endpoints.auth, params) {
            Ok(url) => url.into(),
            // The endpoint is Google's, or a test's: always an address.
            Err(_) => self.endpoints.auth.clone(),
        }
    }

    fn http() -> Result<reqwest::Client, GoogleError> {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| GoogleError::Other(e.without_url().to_string()))
    }

    /// A request to Google's token endpoint with `form`, answered.
    async fn token(&self, form: &[(&str, &str)]) -> Result<TokenAnswer, GoogleError> {
        let response = Self::http()?
            .post(&self.endpoints.token)
            .form(form)
            .send()
            .await
            .map_err(|e| GoogleError::Other(format!("reaching Google: {}", e.without_url())))?;
        let status = response.status();
        let body = Zeroizing::new(
            response
                .bytes()
                .await
                .map_err(|e| {
                    GoogleError::Other(format!("reading Google's answer: {}", e.without_url()))
                })?
                .to_vec(),
        );
        if !status.is_success() {
            // Only the error's name: its description may quote the request.
            return Err(match serde_json::from_slice::<Refusal>(&body) {
                Ok(refusal) => GoogleError::Refused(refusal.error),
                Err(_) => GoogleError::Other(format!("Google answered HTTP {status}")),
            });
        }
        serde_json::from_slice(&body)
            .map_err(|_| GoogleError::Other("Google's answer was not understood".into()))
    }

    /// Trade the `code` Google sent back for a refresh token, with the
    /// verifier only this node holds. Everything goes in the request body.
    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<SignedIn, GoogleError> {
        let answer = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
            ])
            .await?;
        // The owner may untick a permission on Google's page.
        let granted: Vec<&str> = answer.scope.split(' ').collect();
        if !granted.contains(&DRIVE_SCOPE) {
            return Err(GoogleError::ScopeMissing);
        }
        let reads_all = granted.contains(&DRIVE_READ_ALL_SCOPE);
        let refresh_token = answer
            .refresh_token
            .map(Zeroizing::new)
            .ok_or_else(|| GoogleError::Other("Google sent no refresh token".into()))?;
        Ok(SignedIn {
            refresh_token,
            reads_all,
        })
    }

    /// The drive's room: the bytes used and, unless it has no limit, how
    /// many it holds. Every file of the account counts, as Google counts.
    pub async fn room(&self, refresh_token: &str) -> Result<(u64, Option<u64>), GoogleError> {
        let answer = self
            .token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
            ])
            .await?;
        let access = Zeroizing::new(
            answer
                .access_token
                .ok_or_else(|| GoogleError::Other("Google sent no access token".into()))?,
        );
        let response = Self::http()?
            .get(format!("{}/about", self.endpoints.drive))
            .query(&[("fields", "storageQuota(limit,usage)")])
            .bearer_auth(access.as_str())
            .send()
            .await
            .map_err(|e| GoogleError::Other(format!("reaching Google: {}", e.without_url())))?;
        let status = response.status();
        if !status.is_success() {
            return Err(GoogleError::Other(format!("Google answered HTTP {status}")));
        }
        let body = response.bytes().await.map_err(|e| {
            GoogleError::Other(format!("reading Google's answer: {}", e.without_url()))
        })?;
        let about: About = serde_json::from_slice(&body)
            .map_err(|_| GoogleError::Other("Google's answer was not understood".into()))?;
        let number = |text: &str| {
            text.parse::<u64>()
                .map_err(|_| GoogleError::Other("Google's answer was not understood".into()))
        };
        let used = number(&about.storage_quota.usage)?;
        let limit = about
            .storage_quota
            .limit
            .as_deref()
            .map(number)
            .transpose()?;
        Ok((used, limit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_challenge_is_rfc_7636s() {
        // RFC 7636, Appendix B.
        assert_eq!(
            challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let pkce = pkce().unwrap();
        assert_eq!(pkce.verifier.len(), 43);
        assert_eq!(pkce.challenge, challenge_of(&pkce.verifier));
        assert_ne!(
            pkce.verifier.as_str(),
            self::pkce().unwrap().verifier.as_str()
        );
    }

    #[test]
    fn the_sign_in_asks_for_drive_file_only() {
        let client = GoogleClient::new("id.apps.googleusercontent.com", "shh");
        let url = url::Url::parse(&client.sign_in_url(
            "http://127.0.0.1:7340/oauth/google/callback",
            "the-state",
            "the-challenge",
            false,
        ))
        .unwrap();
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["scope"], DRIVE_SCOPE);
        assert_eq!(
            params["redirect_uri"],
            "http://127.0.0.1:7340/oauth/google/callback"
        );
        assert_eq!(params["state"], "the-state");
        assert_eq!(params["code_challenge"], "the-challenge");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["access_type"], "offline");
        assert_eq!(params["response_type"], "code");
        // The secret never goes to the browser.
        assert!(!url.as_str().contains("shh"));
        assert!(!format!("{client:?}").contains("shh"));
        // Reading the rest of the drive is asked for only when wanted.
        let wide =
            url::Url::parse(&client.sign_in_url("http://127.0.0.1:1/cb", "s", "c", true)).unwrap();
        let scope = wide
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(scope, format!("{DRIVE_SCOPE} {DRIVE_READ_ALL_SCOPE}"));
    }

    /// A token endpoint on loopback that answers `status` and `body` once,
    /// and hands back the request it got.
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

    fn client_at(token: &str) -> GoogleClient {
        GoogleClient::new("the-client", "the-secret").with_endpoints(Endpoints {
            auth: "http://127.0.0.1/auth".into(),
            token: token.into(),
            drive: "http://127.0.0.1:1/drive".into(),
        })
    }

    #[tokio::test]
    async fn the_code_is_exchanged_in_the_body() {
        let (url, served) = token_endpoint(
            200,
            r#"{"access_token":"at","refresh_token":"rt-1","scope":"https://www.googleapis.com/auth/drive.file","token_type":"Bearer","expires_in":3599}"#,
        )
        .await;
        let signed = client_at(&url)
            .exchange("the-code", "the-verifier", "http://127.0.0.1:1/cb")
            .await
            .unwrap();
        assert_eq!(signed.refresh_token.as_str(), "rt-1");
        assert!(!signed.reads_all);
        let request = served.await.unwrap();
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("POST /token HTTP/1.1"), "{head}");
        // Nothing secret in the address.
        let line = head.lines().next().unwrap();
        assert!(!line.contains("the-code") && !line.contains("the-secret"));
        let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "the-code");
        assert_eq!(form["code_verifier"], "the-verifier");
        assert_eq!(form["client_secret"], "the-secret");
        assert_eq!(form["redirect_uri"], "http://127.0.0.1:1/cb");
    }

    /// A loopback server answering each of `answers` in turn, one request
    /// a connection; it hands back the requests' first lines.
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
            r#"{"storageQuota":{"limit":"16106127360","usage":"5368709120"}}"#,
        ])
        .await;
        let client = GoogleClient::new("the-client", "the-secret").with_endpoints(Endpoints {
            auth: format!("{base}/auth"),
            token: format!("{base}/token"),
            drive: format!("{base}/drive/v3"),
        });
        assert_eq!(
            client.room("rt-1").await.unwrap(),
            (5_368_709_120, Some(16_106_127_360))
        );
        let lines = served.await.unwrap();
        assert!(lines[0].starts_with("POST /token "), "{lines:?}");
        assert!(
            lines[1].starts_with("GET /drive/v3/about?fields="),
            "{lines:?}"
        );
        // No token in an address.
        assert!(!lines
            .iter()
            .any(|l| l.contains("rt-1") || l.contains("at-1")));
    }

    #[tokio::test]
    async fn a_refusal_or_a_missing_permission_says_so_without_the_secrets() {
        let (url, _) = token_endpoint(
            400,
            r#"{"error":"invalid_grant","error_description":"Bad Request"}"#,
        )
        .await;
        let refused = client_at(&url)
            .exchange("the-code", "the-verifier", "http://127.0.0.1:1/cb")
            .await
            .unwrap_err();
        assert!(matches!(&refused, GoogleError::Refused(e) if e == "invalid_grant"));
        let (url, _) = token_endpoint(
            200,
            r#"{"access_token":"at","refresh_token":"rt-1","scope":"openid","token_type":"Bearer"}"#,
        )
        .await;
        let unticked = client_at(&url)
            .exchange("the-code", "the-verifier", "http://127.0.0.1:1/cb")
            .await
            .unwrap_err();
        assert!(matches!(unticked, GoogleError::ScopeMissing));
        for e in [refused, unticked] {
            let shown = e.to_string();
            assert!(
                !shown.contains("the-code")
                    && !shown.contains("the-secret")
                    && !shown.contains("rt-1")
            );
        }
    }
}
