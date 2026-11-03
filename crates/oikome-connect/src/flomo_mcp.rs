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

//! flomo through its MCP server (`https://flomoapp.com/mcp`, flomo MAX,
//! a personal token).
//!
//! flomo does not publish its tools, so the connector finds the tool that
//! lists or searches memos by name (or uses the one configured), pages
//! through it and reads memos from its answer, matching field names
//! loosely. Not yet run against flomo itself: when the tool or its answer
//! is not what this module expects, the error says what the server offered,
//! so this module is the one place to adjust. A search may not list every
//! memo, so pulls are not `complete` and never delete.

use async_trait::async_trait;
use serde_json::{json, Value as Json};
use zeroize::Zeroizing;

use crate::mcp::{McpClient, Tool, ToolResult};
use crate::{ColumnDef, ColumnKind, ConnectError, Connector, Pull, Row, TableDef, Value};

pub const KIND: &str = "flomo-mcp";
pub const DEFAULT_ENDPOINT: &str = "https://flomoapp.com/mcp";
/// The table memos land in.
pub const TABLE: &str = "memos";
/// Pages read at most in one pull.
const MAX_PAGES: u64 = 1000;
/// Memos asked for per page.
const PAGE_SIZE: u64 = 100;

/// Tool names tried first, in order.
const KNOWN_TOOLS: [&str; 7] = [
    "list_memos",
    "memo_list",
    "get_memos",
    "list_notes",
    "search_memos",
    "memo_search",
    "search_notes",
];

/// The memos table.
pub fn memos_table() -> TableDef {
    let text = |name: &str| ColumnDef {
        name: name.to_string(),
        kind: ColumnKind::Text,
    };
    TableDef {
        name: TABLE.to_string(),
        columns: vec![
            text("id"),
            text("created_at"),
            text("updated_at"),
            text("content"),
            text("tags"),
        ],
        key: vec!["id".to_string()],
    }
}

pub struct FlomoMcp {
    client: McpClient,
    tool: Option<String>,
    arguments: Option<Json>,
}

impl FlomoMcp {
    pub fn new(
        endpoint: String,
        token: Zeroizing<String>,
        tool: Option<String>,
        arguments: Option<Json>,
    ) -> Result<Self, ConnectError> {
        Ok(Self {
            client: McpClient::new(endpoint, token)?,
            tool,
            arguments,
        })
    }
}

/// The tool that lists memos: `configured`, else a known name, else one
/// whose name speaks of memos or notes and of listing or searching.
fn choose_tool<'a>(tools: &'a [Tool], configured: Option<&str>) -> Result<&'a Tool, ConnectError> {
    let names = || {
        tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(name) = configured {
        return tools.iter().find(|t| t.name == name).ok_or_else(|| {
            ConnectError::Config(format!(
                "flomo offers no tool '{name}'; it offers: {}",
                names()
            ))
        });
    }
    KNOWN_TOOLS
        .iter()
        .find_map(|k| tools.iter().find(|t| t.name == *k))
        .or_else(|| {
            tools.iter().find(|t| {
                let n = t.name.to_ascii_lowercase();
                (n.contains("memo") || n.contains("note"))
                    && ["list", "search", "get", "query", "recent"]
                        .iter()
                        .any(|v| n.contains(v))
            })
        })
        .ok_or_else(|| {
            ConnectError::Remote(format!(
                "flomo's MCP server offers no tool that lists memos; it offers: {}. \
                 Name one when adding the source",
                names()
            ))
        })
}

/// Arguments for `tool` when none are configured: an empty query and the
/// page size, where its input schema has them.
fn default_arguments(tool: &Tool) -> Json {
    let mut args = serde_json::Map::new();
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Json::as_object);
    for (name, _) in properties.into_iter().flatten() {
        match name.as_str() {
            "query" | "keyword" | "keywords" | "q" | "text" => {
                args.insert(name.clone(), json!(""));
            }
            "limit" | "page_size" | "pageSize" | "size" | "count" => {
                args.insert(name.clone(), json!(PAGE_SIZE));
            }
            _ => {}
        }
    }
    Json::Object(args)
}

/// The name of the page parameter of `tool`, if it has one.
fn page_parameter(tool: &Tool) -> Option<&'static str> {
    let properties = tool.input_schema.get("properties")?.as_object()?;
    ["page", "page_no", "pageNo"]
        .into_iter()
        .find(|p| properties.contains_key(*p))
}

/// The memo objects in a tool's answer.
fn memo_objects(result: &ToolResult) -> Result<Vec<Json>, ConnectError> {
    fn array_in(value: &Json) -> Option<&Vec<Json>> {
        if let Some(array) = value.as_array() {
            return Some(array);
        }
        let object = value.as_object()?;
        for key in ["memos", "notes", "items", "results", "list", "data"] {
            if let Some(found) = object.get(key).and_then(array_in) {
                return Some(found);
            }
        }
        None
    }
    if let Some(found) = result.structured.as_ref().and_then(array_in) {
        return Ok(found.clone());
    }
    let mut memos = Vec::new();
    let mut unread = None;
    for text in result.texts() {
        match serde_json::from_str::<Json>(text) {
            Ok(value) => match array_in(&value) {
                Some(found) => memos.extend(found.iter().cloned()),
                None if value.is_object() => memos.push(value),
                None => unread = Some(text),
            },
            Err(_) => unread = Some(text),
        }
    }
    if memos.is_empty() {
        if let Some(text) = unread {
            let shown: String = text.chars().take(200).collect();
            return Err(ConnectError::Format(format!(
                "flomo answered in a form this connector does not read yet: {shown}"
            )));
        }
    }
    Ok(memos)
}

fn field<'a>(memo: &'a Json, names: &[&str]) -> Option<&'a Json> {
    names
        .iter()
        .find_map(|n| memo.get(*n))
        .filter(|v| !v.is_null())
}

fn text_of(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A memo object as a row of [`memos_table`].
fn memo_row(memo: &Json) -> Result<Row, ConnectError> {
    let id = field(memo, &["id", "slug", "memo_id", "memoId", "uuid"])
        .map(text_of)
        .ok_or_else(|| ConnectError::Format("a memo without an id".to_string()))?;
    let text = |names: &[&str]| field(memo, names).map(text_of).map(Value::Text);
    let content = text(&["content", "text", "body", "markdown"]).unwrap_or(Value::Null);
    let tags = match field(memo, &["tags", "tag_list"]) {
        Some(Json::Array(tags)) => {
            let tags: Vec<String> = tags.iter().map(text_of).collect();
            Value::Text(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()))
        }
        Some(other) => Value::Text(text_of(other)),
        None => match &content {
            Value::Text(c) => Value::Text(
                serde_json::to_string(&crate::flomo_export::tags_in(c))
                    .unwrap_or_else(|_| "[]".into()),
            ),
            _ => Value::Null,
        },
    };
    Ok(vec![
        Value::Text(id),
        text(&["created_at", "createdAt", "created", "create_time"]).unwrap_or(Value::Null),
        text(&["updated_at", "updatedAt", "updated", "update_time"]).unwrap_or(Value::Null),
        content,
        tags,
    ])
}

#[async_trait]
impl Connector for FlomoMcp {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn tables(&self) -> Vec<TableDef> {
        vec![memos_table()]
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError> {
        self.client.initialize().await?;
        let tools = self.client.list_tools().await?;
        let tool = choose_tool(&tools, self.tool.as_deref())?;
        let base = self
            .arguments
            .clone()
            .unwrap_or_else(|| default_arguments(tool));
        let page = page_parameter(tool);
        let mut rows: Vec<Row> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for n in 1..=MAX_PAGES {
            let mut args = base.clone();
            if let (Some(page), Some(object)) = (page, args.as_object_mut()) {
                object.insert(page.to_string(), json!(n));
            }
            let result = self.client.call_tool(&tool.name, args).await?;
            if result.is_error {
                let shown: String = result.texts().collect::<Vec<_>>().join(" ");
                return Err(ConnectError::Remote(format!(
                    "flomo's {}: {}",
                    tool.name,
                    shown.chars().take(200).collect::<String>()
                )));
            }
            let memos = memo_objects(&result)?;
            let before = rows.len();
            for memo in &memos {
                let row = memo_row(memo)?;
                if seen.insert(row[0].clone().to_string_key()) {
                    rows.push(row);
                }
            }
            // One page only, or a page that brought nothing new.
            if page.is_none() || rows.len() == before {
                break;
            }
        }
        // The newest change seen; kept for a tool that can filter by it.
        let newest = rows
            .iter()
            .filter_map(|r| match &r[2] {
                Value::Text(t) => Some(t.clone()),
                _ => None,
            })
            .max()
            .or_else(|| cursor.map(str::to_string));
        Ok(Pull {
            upserts: vec![(TABLE.to_string(), rows)],
            deletes: Vec::new(),
            cursor: newest,
            complete: false,
        })
    }
}

/// A row value as a set key.
trait StringKey {
    fn to_string_key(self) -> String;
}

impl StringKey for Value {
    fn to_string_key(self) -> String {
        match self {
            Value::Text(t) => t,
            other => format!("{other:?}"),
        }
    }
}
