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

//! Connectors (architecture v3 §11): bring one app's data into a
//! transactional Space the owner holds.
//!
//! A [`Connector`] describes its tables ([`TableDef`]: columns and the
//! source key) and returns, from [`Connector::pull`], the rows that are new
//! or changed since a cursor, the keys of rows the source deleted, and the
//! cursor to keep for next time. The node applies a pull to the Space in
//! one SQLite transaction together with the new cursor (`oikome-node`), so
//! a failed pull changes nothing. Connectors only move data: reshaping is
//! done with views.
//!
//! Built-in connectors: [`csv::CsvSource`] (a CSV file), the files of a
//! folder as photos or notes ([`folder::PhotosFolder`],
//! [`folder::NotesFolder`]), and flomo through its HTML export
//! ([`flomo_export::FlomoExport`]) or its MCP server ([`flomo_mcp::FlomoMcp`],
//! flomo MAX).

pub mod csv;
pub mod flomo_export;
pub mod flomo_mcp;
pub mod folder;
pub mod mcp;

use async_trait::async_trait;
pub use oikome_engine::value::Value;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// A failed pull. Messages never contain credentials.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("{0}")]
    Io(String),
    /// The source's data is not what the connector expects.
    #[error("{0}")]
    Format(String),
    /// The app's service failed or refused the request.
    #[error("{0}")]
    Remote(String),
    #[error("{0}")]
    Config(String),
}

/// The type of a column, as SQLite declares it (the SQLite engine reads
/// these back as Arrow types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Integer,
    Real,
    Text,
    Boolean,
    /// Bytes, such as an image.
    Blob,
    /// A time as text, `2025-11-07 09:40:00` (or a day, `2025-11-07`).
    Timestamp,
}

impl ColumnKind {
    /// The declared SQLite type.
    pub fn sql_type(self) -> &'static str {
        match self {
            ColumnKind::Integer => "INTEGER",
            ColumnKind::Real => "REAL",
            ColumnKind::Text => "TEXT",
            ColumnKind::Boolean => "BOOLEAN",
            ColumnKind::Blob => "BLOB",
            ColumnKind::Timestamp => "TIMESTAMP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub kind: ColumnKind,
}

/// A table a connector writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    /// The columns that identify a row at the source (its primary key).
    pub key: Vec<String>,
}

impl TableDef {
    /// Positions of the key columns.
    pub fn key_positions(&self) -> Vec<usize> {
        self.key
            .iter()
            .filter_map(|k| self.columns.iter().position(|c| &c.name == k))
            .collect()
    }
}

/// One row: a value per column of its [`TableDef`], in order.
pub type Row = Vec<Value>;

/// What a pull returns.
#[derive(Debug, Default)]
pub struct Pull {
    /// New or changed rows, per table.
    pub upserts: Vec<(String, Vec<Row>)>,
    /// Keys (values of the key columns, in order) of rows the source
    /// reports deleted, per table.
    pub deletes: Vec<(String, Vec<Row>)>,
    /// The cursor to store; opaque to the node.
    pub cursor: Option<String>,
    /// `upserts` lists every row the source has: rows not in it were
    /// deleted at the source.
    pub complete: bool,
}

impl Pull {
    /// Nothing changed since the cursor.
    pub fn unchanged(cursor: Option<String>) -> Self {
        Self {
            cursor,
            ..Default::default()
        }
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    /// `csv`, `flomo-export`, `flomo-mcp`.
    fn kind(&self) -> &'static str;
    /// The tables this connector writes.
    fn tables(&self) -> Vec<TableDef>;
    /// Rows changed since `cursor` (everything when `None`).
    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError>;
}

/// How a source is set up: what the node stores about it, without
/// credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceConfig {
    /// A CSV file with a header row.
    Csv {
        path: String,
        /// The columns and their types, fixed when the source is added.
        table: TableDef,
    },
    /// flomo's HTML export (`flomo_backup.zip`, or its `index.html`).
    FlomoExport { path: String },
    /// The photos of a folder, with its `captions.csv` if any.
    PhotosFolder { path: String },
    /// The Markdown and text files of a folder, such as a notes vault.
    NotesFolder { path: String },
    /// flomo's MCP server (flomo MAX), with a personal token.
    FlomoMcp {
        endpoint: String,
        /// The tool that lists memos, when not the one found by name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        /// Arguments for that tool, as JSON.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<serde_json::Value>,
    },
}

impl SourceConfig {
    /// The connector kind.
    pub fn kind(&self) -> &'static str {
        match self {
            SourceConfig::Csv { .. } => csv::KIND,
            SourceConfig::FlomoExport { .. } => flomo_export::KIND,
            SourceConfig::PhotosFolder { .. } => folder::PHOTOS_KIND,
            SourceConfig::NotesFolder { .. } => folder::NOTES_KIND,
            SourceConfig::FlomoMcp { .. } => flomo_mcp::KIND,
        }
    }

    /// Whether the connector needs a token.
    pub fn needs_token(&self) -> bool {
        matches!(self, SourceConfig::FlomoMcp { .. })
    }

    /// The connector this configuration describes; `token` for those that
    /// need one.
    pub fn connector(
        &self,
        token: Option<Zeroizing<String>>,
    ) -> Result<Box<dyn Connector>, ConnectError> {
        Ok(match self {
            SourceConfig::Csv { path, table } => {
                Box::new(csv::CsvSource::new(path.into(), table.clone()))
            }
            SourceConfig::FlomoExport { path } => {
                Box::new(flomo_export::FlomoExport::new(path.into()))
            }
            SourceConfig::PhotosFolder { path } => Box::new(folder::PhotosFolder::new(path.into())),
            SourceConfig::NotesFolder { path } => Box::new(folder::NotesFolder::new(path.into())),
            SourceConfig::FlomoMcp {
                endpoint,
                tool,
                arguments,
            } => {
                let token = token.ok_or_else(|| {
                    ConnectError::Config("flomo over MCP needs a token".to_string())
                })?;
                Box::new(flomo_mcp::FlomoMcp::new(
                    endpoint.clone(),
                    token,
                    tool.clone(),
                    arguments.clone(),
                )?)
            }
        })
    }
}

/// BLAKE3 of `bytes`, as hex: the cursor of a file read whole.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
