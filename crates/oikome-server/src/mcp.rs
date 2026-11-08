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

//! MCP over HTTP for the owner's AI assistants (M3e): `POST /mcp` on the
//! web API's loopback port, MCP's Streamable HTTP transport answered with
//! plain JSON.
//!
//! - **Who.** Every request carries an assistant's token
//!   (`Authorization: Bearer oik_…`); it acts as that assistant, through the
//!   API's assistant calls, so its grants decide what it reads. A request
//!   with an `Origin` is refused (a web page has no business here: DNS
//!   rebinding), as is one whose `Host` isn't this server (checked before).
//! - **What.** Three tools: `list_collections`, `search`, `read_row`. None of
//!   the owner's own operations is offered.
//! - **Which protocol.** Both eras on one endpoint: a request carrying
//!   per-request `_meta` is served as revision 2026-07-28 (its headers
//!   checked against its body, `server/discover` answered); `initialize`
//!   and what follows it as 2025-11-25 and earlier, without sessions. No
//!   GET stream: GET and DELETE are 405.
//! - **How much.** Each assistant may send [`MAX_CALLS`] requests a minute.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch};
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::DataType;
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use oikome_api::{Api, ApiError, Caller, ErrorCode, ReadRow, Search};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The MCP endpoint's path.
pub const PATH: &str = "/mcp";
/// Revisions served per request (`_meta`), newest first.
const MODERN: [&str; 1] = ["2026-07-28"];
/// Revisions served after `initialize`, newest first.
const LEGACY: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Requests an assistant may send in [`CALL_WINDOW`].
const MAX_CALLS: u32 = 120;
const CALL_WINDOW: Duration = Duration::from_secs(60);
/// A picture in a row read is sent as an image up to this size.
const MAX_IMAGE_BYTES: usize = 4 << 20;

const INSTRUCTIONS: &str = "Oikome is the owner's personal data center on this computer. \
You may read only the collections the owner gave you. Call list_collections to see them, \
search to find rows by their words, and read_row to open one by its key. \
Every search and read shows up in the owner's Activity.";

// JSON-RPC and MCP error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_VERSION: i64 = -32022;
/// Not authenticated (an implementation-defined server error).
const UNAUTHORIZED: i64 = -32001;
/// Too many requests (an implementation-defined server error).
const RATE_LIMITED: i64 = -32002;

/// Requests per assistant in the current window.
#[derive(Default)]
pub struct Calls(Mutex<HashMap<String, (Instant, u32)>>);

impl Calls {
    /// Count a request by `who`; false once it is over [`MAX_CALLS`].
    fn allow(&self, who: &str) -> bool {
        let mut calls = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        calls.retain(|_, (start, _)| now.duration_since(*start) < CALL_WINDOW);
        let (_, count) = calls.entry(who.to_string()).or_insert((now, 0));
        *count += 1;
        *count <= MAX_CALLS
    }
}

/// Which era a request is served in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Era {
    Modern,
    Legacy,
}

/// A JSON-RPC reply: its HTTP status and body (`None`: 202, no body).
struct Reply {
    status: StatusCode,
    body: Option<Value>,
}

impl Reply {
    fn result(id: &Value, era: Era, mut result: Value) -> Self {
        if era == Era::Modern {
            if let Some(object) = result.as_object_mut() {
                object.insert("resultType".to_string(), json!("complete"));
            }
        }
        Self {
            status: StatusCode::OK,
            body: Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        }
    }

    fn error(
        status: StatusCode,
        id: Option<&Value>,
        code: i64,
        message: &str,
        data: Option<Value>,
    ) -> Self {
        let mut error = json!({ "code": code, "message": message });
        if let Some(data) = data {
            error["data"] = data;
        }
        let mut body = json!({ "jsonrpc": "2.0", "error": error });
        if let Some(id) = id {
            body["id"] = id.clone();
        }
        Self {
            status,
            body: Some(body),
        }
    }

    fn into_response(self) -> Response {
        let body = self
            .body
            .map(|b| serde_json::to_vec(&b).unwrap_or_default())
            .unwrap_or_default();
        let empty = body.is_empty();
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = self.status;
        let headers = response.headers_mut();
        if !empty {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        response
    }
}

/// Answer one request to [`PATH`]. The caller has checked its `Host`.
pub async fn handle(
    api: &Api,
    calls: &Calls,
    method: &Method,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    if method != Method::POST {
        let mut response = Reply::error(
            StatusCode::METHOD_NOT_ALLOWED,
            None,
            INVALID_REQUEST,
            "MCP here is POST only; there is no stream to open",
            None,
        )
        .into_response();
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("POST"));
        return response;
    }
    serve(api, calls, headers, body).await.into_response()
}

async fn serve(api: &Api, calls: &Calls, headers: &HeaderMap, body: Body) -> Reply {
    if headers.contains_key(header::ORIGIN) {
        return Reply::error(
            StatusCode::FORBIDDEN,
            None,
            INVALID_REQUEST,
            "requests from web pages are refused",
            None,
        );
    }
    let Some((token, caller)) = bearer(headers).and_then(|t| Some((t, api.assistant_by_token(t)?)))
    else {
        return Reply::error(
            StatusCode::UNAUTHORIZED,
            None,
            UNAUTHORIZED,
            "this assistant isn't connected to Oikome: connect it again on the AI Assistants page",
            None,
        );
    };
    // Counted by the token's hash: the token itself is kept nowhere.
    if !calls.allow(blake3::hash(token.as_bytes()).to_hex().as_str()) {
        return Reply::error(
            StatusCode::TOO_MANY_REQUESTS,
            None,
            RATE_LIMITED,
            &format!("at most {MAX_CALLS} requests a minute; wait a moment"),
            None,
        );
    }
    let Ok(body) = to_bytes(body, MAX_BODY_BYTES).await else {
        return Reply::error(
            StatusCode::PAYLOAD_TOO_LARGE,
            None,
            INVALID_REQUEST,
            &format!("requests are limited to {} KiB", MAX_BODY_BYTES / 1024),
            None,
        );
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return Reply::error(
            StatusCode::BAD_REQUEST,
            None,
            PARSE_ERROR,
            "the body isn't JSON",
            None,
        );
    };
    let Some(message) = message.as_object() else {
        return Reply::error(
            StatusCode::BAD_REQUEST,
            None,
            INVALID_REQUEST,
            "send one JSON-RPC message per request",
            None,
        );
    };
    let Some(name) = message.get("method").and_then(Value::as_str) else {
        // A response, or nothing: clients send neither.
        return Reply::error(
            StatusCode::BAD_REQUEST,
            None,
            INVALID_REQUEST,
            "no method",
            None,
        );
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(id) = message.get("id") else {
        // A notification (`notifications/initialized`, `…/cancelled`).
        return Reply {
            status: StatusCode::ACCEPTED,
            body: None,
        };
    };
    let era = match era_of(headers, name, &params) {
        Ok(era) => era,
        Err(reply) => return reply.with_id(id),
    };
    match name {
        "initialize" => initialize(id, &params),
        "server/discover" => Reply::result(
            id,
            era,
            json!({
                "supportedVersions": supported(),
                "capabilities": { "tools": {} },
                "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
                "instructions": INSTRUCTIONS,
            }),
        ),
        "ping" => Reply::result(id, era, json!({})),
        "tools/list" => Reply::result(id, era, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(api, &caller, name, arguments).await {
                Some(result) => Reply::result(id, era, result),
                None => Reply::error(
                    StatusCode::OK,
                    Some(id),
                    INVALID_PARAMS,
                    &format!("there is no tool named {}", clip(name)),
                    None,
                ),
            }
        }
        _ => Reply::error(
            // A modern server says so with 404, so a client tells it from
            // an old endpoint; within a legacy session it is an ordinary error.
            if era == Era::Modern {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            },
            Some(id),
            METHOD_NOT_FOUND,
            "method not found",
            None,
        ),
    }
}

impl Reply {
    fn with_id(mut self, id: &Value) -> Self {
        if let Some(body) = self.body.as_mut() {
            body["id"] = id.clone();
        }
        self
    }
}

/// The bearer token of `Authorization`, if it has one.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

/// Every revision served, newest first.
fn supported() -> Vec<&'static str> {
    MODERN.iter().chain(LEGACY.iter()).copied().collect()
}

fn server_info() -> Value {
    json!({ "name": "oikome", "title": "Oikome", "version": env!("CARGO_PKG_VERSION") })
}

/// Which era `method` is served in, checking what the request's headers
/// say against its body; a refusal as an error reply (its id added later).
fn era_of(headers: &HeaderMap, method: &str, params: &Value) -> Result<Era, Reply> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let mismatch = |what: &str| {
        Reply::error(
            StatusCode::BAD_REQUEST,
            None,
            HEADER_MISMATCH,
            &format!("header mismatch: {what}"),
            None,
        )
    };
    let declared = params
        .get("_meta")
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str);
    let Some(version) = declared else {
        // Legacy: `initialize`, and the requests after it.
        return match header("mcp-protocol-version") {
            None => Ok(Era::Legacy),
            Some(v) if LEGACY.contains(&v) || method == "initialize" => Ok(Era::Legacy),
            Some(v) if MODERN.contains(&v) => {
                Err(mismatch("the request has no protocol version in _meta"))
            }
            Some(v) => Err(unsupported(v)),
        };
    };
    if header("mcp-protocol-version") != Some(version) {
        return Err(mismatch("MCP-Protocol-Version differs from the body's"));
    }
    if !MODERN.contains(&version) {
        return Err(unsupported(version));
    }
    if header("mcp-method") != Some(method) {
        return Err(mismatch("Mcp-Method differs from the body's method"));
    }
    if method == "tools/call" {
        let named = params.get("name").and_then(Value::as_str);
        let sent = header("mcp-name").and_then(decode_header);
        if named.is_none() || sent.as_deref() != named {
            return Err(mismatch("Mcp-Name differs from the tool called"));
        }
    }
    Ok(Era::Modern)
}

fn unsupported(requested: &str) -> Reply {
    Reply::error(
        StatusCode::BAD_REQUEST,
        None,
        UNSUPPORTED_VERSION,
        "Unsupported protocol version",
        Some(json!({ "supported": supported(), "requested": clip(requested) })),
    )
}

/// A header value as sent: plain, or `=?base64?…?=` for one that isn't.
fn decode_header(value: &str) -> Option<String> {
    match value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    {
        Some(encoded) => String::from_utf8(STANDARD.decode(encoded).ok()?).ok(),
        None => Some(value.to_string()),
    }
}

/// The legacy handshake: the client's revision when it is one served here,
/// else the newest legacy one; no session is kept.
fn initialize(id: &Value, params: &Value) -> Reply {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = LEGACY
        .iter()
        .find(|v| **v == asked)
        .copied()
        .unwrap_or(LEGACY[0]);
    Reply::result(
        id,
        Era::Legacy,
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS,
        }),
    )
}

/// The tools an assistant has: reading only.
fn tools() -> Value {
    let read_only =
        json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false });
    json!([
        {
            "name": "list_collections",
            "title": "List collections",
            "description": "The collections in the owner's Oikome data center that you may read: each table's columns, how many rows it has, and the key columns that name one of its rows (for read_row).",
            "inputSchema": { "type": "object", "additionalProperties": false },
            "annotations": read_only,
        },
        {
            "name": "search",
            "title": "Search collections",
            "description": "Find rows with every word of the query in their text: words of three or more characters match inside words, case and accents aside, in any language (Chinese and Japanese too). Searches all the collections you may read, or one. Each hit names its collection, table and key (to open it with read_row), the column that matched, and an excerpt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Plain words, up to 200 characters.", "maxLength": 200 },
                    "collection": { "type": "string", "description": "Only this collection." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "At most this many hits; 30 when left out." }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_only,
        },
        {
            "name": "read_row",
            "title": "Read a row",
            "description": "One row of a table, by its key: the values of the table's key columns, in order, as list_collections and search give them. Text, numbers and dates come as fields; a photo comes as an image.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": { "type": "string" },
                    "table": { "type": "string" },
                    "key": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 8 }
                },
                "required": ["collection", "table", "key"],
                "additionalProperties": false
            },
            "annotations": read_only,
        }
    ])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    collection: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRowArgs {
    collection: String,
    table: String,
    key: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// Run tool `name` as `caller`: its result, a failure the model can act on
/// as an error result, or `None` for a tool there isn't.
async fn call(api: &Api, caller: &Caller, name: &str, arguments: Value) -> Option<Value> {
    let done = match name {
        "list_collections" => match args::<NoArgs>(arguments) {
            Ok(NoArgs {}) => api
                .assistant_collections(caller, oikome_api::Empty {})
                .await
                .map(|c| structured(json!({ "collections": c }))),
            Err(e) => Err(e),
        },
        "search" => match args::<SearchArgs>(arguments) {
            Ok(a) => api
                .assistant_search(
                    caller,
                    Search {
                        query: a.query,
                        collection: a.collection,
                        limit: a.limit,
                    },
                )
                .await
                .map(|found| {
                    // Where the words are is for the owner's page; the
                    // model reads the excerpt.
                    let hits: Vec<Value> = found
                        .hits
                        .iter()
                        .map(|h| {
                            json!({
                                "collection": h.collection,
                                "table": h.table,
                                "key": h.key,
                                "column": h.column,
                                "excerpt": h.excerpt,
                            })
                        })
                        .collect();
                    let skipped: Vec<&String> =
                        found.skipped.iter().map(|s| &s.collection).collect();
                    structured(json!({ "hits": hits, "notSearched": skipped }))
                }),
            Err(e) => Err(e),
        },
        "read_row" => match args::<ReadRowArgs>(arguments) {
            Ok(a) => {
                let (collection, table, key) = (a.collection, a.table, a.key);
                api.assistant_read_row(
                    caller,
                    ReadRow {
                        space: collection.clone(),
                        table: table.clone(),
                        key: key.clone(),
                    },
                )
                .await
                .and_then(|rows| row_result(&collection, &table, &key, &rows.ipc))
            }
            Err(e) => Err(e),
        },
        _ => return None,
    };
    Some(match done {
        Ok(result) => result,
        Err(e) => tool_error(&e),
    })
}

fn args<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ApiError> {
    serde_json::from_value(arguments).map_err(|e| ApiError::invalid(format!("arguments: {e}")))
}

/// A result as structured content, with the same JSON as text for clients
/// that read only text.
fn structured(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    json!({ "content": [{ "type": "text", "text": text }], "structuredContent": value })
}

/// A failure as a tool result the model can read and act on; what went
/// wrong inside stays in the node's log.
fn tool_error(e: &ApiError) -> Value {
    let text = match e.code {
        ErrorCode::Internal => "Something went wrong reading this; try again.".to_string(),
        _ => e.message.clone(),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// One row read: its fields as JSON, and a picture in it as an image.
fn row_result(
    collection: &str,
    table: &str,
    key: &[String],
    ipc: &[u8],
) -> Result<Value, ApiError> {
    let batches: Vec<RecordBatch> =
        arrow_ipc::reader::StreamReader::try_new(Cursor::new(ipc), None)
            .map_err(ApiError::internal)?
            .collect::<Result<_, _>>()
            .map_err(ApiError::internal)?;
    let Some(batch) = batches.iter().find(|b| b.num_rows() > 0) else {
        return Err(ApiError::not_found(format!(
            "No such row in {collection}.{table}"
        )));
    };
    let mut fields = Map::new();
    let mut images = Vec::new();
    for (column, field) in batch.columns().iter().zip(batch.schema().fields()) {
        let value = match value_at(column.as_ref(), 0) {
            Cell::Json(v) => v,
            Cell::Bytes(bytes) => match image_type(&bytes) {
                Some(mime) if bytes.len() <= MAX_IMAGE_BYTES => {
                    images.push(json!({ "type": "image", "data": STANDARD.encode(&bytes), "mimeType": mime }));
                    json!(format!("{mime} image, shown below"))
                }
                Some(mime) => json!(format!(
                    "{mime} image of {} bytes, too large to send",
                    bytes.len()
                )),
                None => json!(format!("{} bytes", bytes.len())),
            },
        };
        fields.insert(field.name().clone(), value);
    }
    let value = json!({ "collection": collection, "table": table, "key": key, "fields": fields });
    let mut result = structured(value);
    if let Some(content) = result["content"].as_array_mut() {
        content.extend(images);
    }
    Ok(result)
}

enum Cell {
    Json(Value),
    Bytes(Vec<u8>),
}

/// The value at `row` of `array`: numbers and booleans as JSON, bytes as
/// they are, anything else (text, dates, times) as its text.
fn value_at(array: &dyn Array, row: usize) -> Cell {
    if array.is_null(row) {
        return Cell::Json(Value::Null);
    }
    match array.data_type() {
        DataType::Binary => return Cell::Bytes(array.as_binary::<i32>().value(row).to_vec()),
        DataType::LargeBinary => return Cell::Bytes(array.as_binary::<i64>().value(row).to_vec()),
        DataType::BinaryView => return Cell::Bytes(array.as_binary_view().value(row).to_vec()),
        DataType::Boolean => return Cell::Json(json!(array.as_boolean().value(row))),
        _ => {}
    }
    let text = ArrayFormatter::try_new(array, &FormatOptions::default())
        .map(|f| f.value(row).to_string())
        .unwrap_or_default();
    if array.data_type().is_integer() || array.data_type().is_floating() {
        if let Ok(number) = serde_json::from_str::<serde_json::Number>(&text) {
            return Cell::Json(Value::Number(number));
        }
    }
    Cell::Json(Value::String(text))
}

/// The image type of `bytes`, from their first bytes.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// At most 64 characters of something a client sent, for a message.
fn clip(text: &str) -> String {
    text.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    /// A modern request's params: `extra` and its `_meta`.
    fn modern(extra: Value) -> Value {
        let mut params = json!({ "_meta": { META_VERSION: "2026-07-28" } });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        params
    }

    #[test]
    fn a_modern_request_must_say_the_same_in_its_headers() {
        let params = modern(json!({ "name": "search" }));
        let ok = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "search"),
        ]);
        assert_eq!(era_of(&ok, "tools/call", &params).ok(), Some(Era::Modern));
        // The name may come encoded.
        let encoded = format!("=?base64?{}?=", STANDARD.encode("search"));
        let ok = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", &encoded),
        ]);
        assert_eq!(era_of(&ok, "tools/call", &params).ok(), Some(Era::Modern));
        for bad in [
            headers(&[
                ("mcp-protocol-version", "2026-07-28"),
                ("mcp-method", "tools/call"),
            ]),
            headers(&[
                ("mcp-protocol-version", "2026-07-28"),
                ("mcp-method", "tools/list"),
                ("mcp-name", "search"),
            ]),
            headers(&[
                ("mcp-protocol-version", "2026-07-28"),
                ("mcp-method", "tools/call"),
                ("mcp-name", "read_row"),
            ]),
            headers(&[("mcp-method", "tools/call"), ("mcp-name", "search")]),
        ] {
            let reply = era_of(&bad, "tools/call", &params).expect_err("refused");
            assert_eq!(reply.status, StatusCode::BAD_REQUEST);
            assert_eq!(reply.body.unwrap()["error"]["code"], HEADER_MISMATCH);
        }
    }

    #[test]
    fn versions_not_served_are_named_with_those_that_are() {
        let params = json!({ "_meta": { META_VERSION: "1900-01-01" } });
        let sent = headers(&[
            ("mcp-protocol-version", "1900-01-01"),
            ("mcp-method", "tools/list"),
        ]);
        let reply = era_of(&sent, "tools/list", &params).expect_err("refused");
        let body = reply.body.unwrap();
        assert_eq!(body["error"]["code"], UNSUPPORTED_VERSION);
        assert_eq!(body["error"]["data"]["supported"][0], "2026-07-28");
        // A legacy client: no `_meta`, an old header or none.
        for sent in [
            headers(&[]),
            headers(&[("mcp-protocol-version", "2025-06-18")]),
        ] {
            assert_eq!(
                era_of(&sent, "tools/list", &json!({})).ok(),
                Some(Era::Legacy)
            );
        }
        let old = headers(&[("mcp-protocol-version", "2024-01-01")]);
        assert!(era_of(&old, "tools/list", &json!({})).is_err());
    }

    #[test]
    fn initialize_settles_on_a_legacy_revision() {
        let id = json!(1);
        let reply = initialize(&id, &json!({ "protocolVersion": "2025-06-18" }));
        assert_eq!(
            reply.body.unwrap()["result"]["protocolVersion"],
            "2025-06-18"
        );
        let reply = initialize(&id, &json!({ "protocolVersion": "2099-01-01" }));
        assert_eq!(reply.body.unwrap()["result"]["protocolVersion"], LEGACY[0]);
    }

    #[test]
    fn only_reading_tools_are_offered() {
        let names: Vec<String> = tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["list_collections", "search", "read_row"]);
        assert!(tools()
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true));
    }

    #[test]
    fn bearer_tokens_are_read_from_authorization() {
        assert_eq!(
            bearer(&headers(&[("authorization", "Bearer oik_x")])),
            Some("oik_x")
        );
        assert_eq!(
            bearer(&headers(&[("authorization", "bearer  oik_x ")])),
            Some("oik_x")
        );
        assert_eq!(bearer(&headers(&[("authorization", "Basic abc")])), None);
        assert_eq!(bearer(&headers(&[("authorization", "Bearer ")])), None);
        assert_eq!(bearer(&headers(&[])), None);
    }

    #[test]
    fn requests_beyond_the_limit_wait() {
        let calls = Calls::default();
        for _ in 0..MAX_CALLS {
            assert!(calls.allow("a"));
        }
        assert!(!calls.allow("a"));
        assert!(calls.allow("b"));
    }

    #[test]
    fn pictures_are_known_by_their_first_bytes() {
        assert_eq!(image_type(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(image_type(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"hello"), None);
    }
}
