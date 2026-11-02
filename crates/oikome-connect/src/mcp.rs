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

//! A minimal Model Context Protocol client over streamable HTTP: it
//! initializes a session, lists the server's tools and calls one. Answers
//! may come as JSON or as a server-sent event stream. Only what connectors
//! need; the token is sent as a bearer token and never shown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value as Json};
use zeroize::Zeroizing;

use crate::ConnectError;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SESSION_HEADER: &str = "mcp-session-id";
/// Pages of a tool listing read at most.
const MAX_TOOL_PAGES: usize = 20;

/// A tool the server offers.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Json,
}

/// What a tool call returned.
#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    /// Content items (`{"type": "text", "text": ...}` and others).
    pub content: Vec<Json>,
    pub structured: Option<Json>,
    pub is_error: bool,
}

impl ToolResult {
    /// The text of the text items.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.content
            .iter()
            .filter(|c| c.get("type").and_then(Json::as_str) == Some("text"))
            .filter_map(|c| c.get("text").and_then(Json::as_str))
    }
}

pub struct McpClient {
    http: reqwest::Client,
    endpoint: String,
    token: Zeroizing<String>,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

fn remote(e: impl std::fmt::Display) -> ConnectError {
    ConnectError::Remote(e.to_string())
}

impl McpClient {
    /// A client of the server at `endpoint` (`https://…`).
    pub fn new(endpoint: String, token: Zeroizing<String>) -> Result<Self, ConnectError> {
        if !endpoint.starts_with("https://") {
            return Err(ConnectError::Config(format!(
                "'{endpoint}' is not an https:// address"
            )));
        }
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| ConnectError::Config(format!("HTTP client: {e}")))?;
        Ok(Self {
            http,
            endpoint,
            token,
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
        })
    }

    fn post(&self, body: &Json) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(&self.endpoint)
            .bearer_auth(self.token.as_str())
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .header("mcp-protocol-version", PROTOCOL_VERSION)
            .json(body);
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            request = request.header(SESSION_HEADER, session);
        }
        request
    }

    /// Send a JSON-RPC request and return its result.
    async fn request(&self, method: &str, params: Json) -> Result<Json, ConnectError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = self
            .post(&body)
            .send()
            .await
            .map_err(|e| remote(format!("{method}: {}", e.without_url())))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(remote("the server refused the token"));
        }
        if !status.is_success() {
            return Err(remote(format!("{method}: HTTP {status}")));
        }
        if let Some(session) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(session.to_string());
        }
        let event_stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let text = response
            .text()
            .await
            .map_err(|e| remote(format!("{method}: {}", e.without_url())))?;
        let message = if event_stream {
            answer_in_events(&text, id)?
        } else {
            serde_json::from_str(&text).map_err(|e| remote(format!("{method}: {e}")))?
        };
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Json::as_str)
                .unwrap_or("error");
            return Err(remote(format!("{method}: {text}")));
        }
        message
            .get("result")
            .cloned()
            .ok_or_else(|| remote(format!("{method}: an answer without a result")))
    }

    /// Open the session.
    pub async fn initialize(&self) -> Result<(), ConnectError> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "oikome", "version": env!("CARGO_PKG_VERSION")},
            }),
        )
        .await?;
        let notice = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        self.post(&notice)
            .send()
            .await
            .map_err(|e| remote(format!("initialized: {}", e.without_url())))?;
        Ok(())
    }

    /// The tools the server offers.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, ConnectError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            for tool in result
                .get("tools")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(name) = tool.get("name").and_then(Json::as_str) {
                    tools.push(Tool {
                        name: name.to_string(),
                        description: tool
                            .get("description")
                            .and_then(Json::as_str)
                            .map(str::to_string),
                        input_schema: tool.get("inputSchema").cloned().unwrap_or(Json::Null),
                    });
                }
            }
            cursor = result
                .get("nextCursor")
                .and_then(Json::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Call the tool `name` with `arguments`.
    pub async fn call_tool(&self, name: &str, arguments: Json) -> Result<ToolResult, ConnectError> {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await?;
        Ok(ToolResult {
            content: result
                .get("content")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default(),
            structured: result.get("structuredContent").cloned(),
            is_error: result
                .get("isError")
                .and_then(Json::as_bool)
                .unwrap_or(false),
        })
    }
}

/// The JSON-RPC message answering request `id` in a server-sent event
/// stream: each event's `data:` lines joined.
fn answer_in_events(stream: &str, id: u64) -> Result<Json, ConnectError> {
    let mut data = String::new();
    let mut answer = None;
    let mut finish = |data: &mut String| {
        if !data.is_empty() {
            if let Ok(message) = serde_json::from_str::<Json>(data) {
                if message.get("id").and_then(Json::as_u64) == Some(id) {
                    answer = Some(message);
                }
            }
            data.clear();
        }
    };
    for line in stream.lines() {
        if line.is_empty() {
            finish(&mut data);
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    finish(&mut data);
    answer.ok_or_else(|| remote("the event stream held no answer"))
}
