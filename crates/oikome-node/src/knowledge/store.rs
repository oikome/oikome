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

//! A collection's chunks in Lance (docs/knowledge-schema.md §3.1, §5.1):
//! `.oikome/knowledge/<space>/lance/chunks.lance`, written and searched
//! through DuckDB's `lance` extension, as Lance Spaces are. Each statement
//! commits on its own, so a conversion first adds its rows and then removes
//! the rows of the version they replace; the control plane says which
//! conversion is current, and readers keep only its rows.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use oikome_engine::name::{quote_ident, quote_literal};
use oikome_engine::{QualifiedName, QueryEngine, SpaceSource, TableSource, Value};

use crate::error::SessionError;

const TABLE: &str = "chunks";
/// The full-text index on `text`: `ngram` splits any script, Chinese
/// included, with no dictionary to download; two-character words match.
const INDEX: &str = "chunks_text";
const INDEX_WITH: &str =
    "base_tokenizer = 'ngram', min_ngram_length = 2, max_ngram_length = 3, lower_case = true";

/// A chunk as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChunk {
    pub chunk_id: String,
    pub asset_id: String,
    pub conversion_id: String,
    pub path: String,
    pub seq: i32,
    pub heading: Vec<String>,
    pub text: String,
    pub page: Option<i32>,
    pub char_start: i64,
    pub char_end: i64,
    pub line_start: i32,
    pub line_end: i32,
    /// Its full-text score, for a search hit.
    pub score: Option<f64>,
}

pub(crate) fn schema() -> SchemaRef {
    let text = |name: &str| Field::new(name, DataType::Utf8, false);
    Arc::new(Schema::new(vec![
        text("chunk_id"),
        text("asset_id"),
        text("asset_blake3"),
        text("conversion_id"),
        text("channel"),
        text("path"),
        Field::new("seq", DataType::Int32, false),
        // The heading path as a JSON array of strings.
        text("heading"),
        text("text"),
        text("content_hash"),
        Field::new("page", DataType::Int32, true),
        Field::new("char_start", DataType::Int64, false),
        Field::new("char_end", DataType::Int64, false),
        Field::new("line_start", DataType::Int32, false),
        Field::new("line_end", DataType::Int32, false),
    ]))
}

const COLUMNS: &str = "chunk_id VARCHAR, asset_id VARCHAR, asset_blake3 VARCHAR, \
    conversion_id VARCHAR, channel VARCHAR, path VARCHAR, seq INTEGER, heading VARCHAR, \
    text VARCHAR, content_hash VARCHAR, page INTEGER, char_start BIGINT, char_end BIGINT, \
    line_start INTEGER, line_end INTEGER";

const SELECT: &str = "chunk_id, asset_id, conversion_id, path, seq, heading, text, page, \
    char_start, char_end, line_start, line_end";

fn engine_err(e: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("knowledge: {e}"))
}

pub(crate) struct Store {
    engine: Arc<dyn QueryEngine>,
    namespace: String,
    dataset: PathBuf,
}

impl Store {
    /// The chunks of the collection whose knowledge is in `root`, made if
    /// missing. Fails with how to install it when the `lance` extension is
    /// not on this node.
    pub(crate) async fn open(
        engine: Arc<dyn QueryEngine>,
        space: &str,
        root: &Path,
    ) -> Result<Self, SessionError> {
        // A name no Space can have (Space names have no ':').
        let namespace = format!("oikome-knowledge:{space}");
        engine
            .attach_space(
                &namespace,
                SpaceSource::LanceNamespace {
                    root: root.to_path_buf(),
                },
            )
            .await?;
        let store = Self {
            dataset: root.join(format!("{TABLE}.lance")),
            namespace,
            engine,
        };
        if !store
            .engine
            .table_names(&store.namespace)
            .await?
            .iter()
            .any(|t| t == TABLE)
        {
            store
                .engine
                .execute(&format!("CREATE TABLE {} ({COLUMNS})", store.table()), &[])
                .await?;
        }
        Ok(store)
    }

    fn table(&self) -> String {
        format!("{}.{}", quote_ident(&self.namespace), quote_ident(TABLE))
    }

    fn dataset_literal(&self) -> String {
        quote_literal(&self.dataset.to_string_lossy())
    }

    /// Add `batch` as the rows of `conversion_id`, then remove the asset's
    /// rows of any other conversion.
    pub(crate) async fn replace(
        &self,
        asset_id: &str,
        conversion_id: &str,
        batch: RecordBatch,
    ) -> Result<(), SessionError> {
        if batch.num_rows() > 0 {
            let load = QualifiedName::new("oikome_knowledge_load", conversion_id);
            self.engine
                .attach(
                    &load,
                    TableSource::ArrowBatches {
                        schema: batch.schema(),
                        batches: vec![batch],
                    },
                )
                .await?;
            let added = self
                .engine
                .execute(
                    &format!(
                        "INSERT INTO {} SELECT * FROM {}",
                        self.table(),
                        load.quoted()
                    ),
                    &[],
                )
                .await;
            self.engine.detach(&load).await?;
            added?;
        }
        self.engine
            .execute(
                &format!(
                    "DELETE FROM {} WHERE asset_id = ? AND conversion_id <> ?",
                    self.table()
                ),
                &[
                    Value::Text(asset_id.into()),
                    Value::Text(conversion_id.into()),
                ],
            )
            .await?;
        Ok(())
    }

    /// Remove every row of an asset that went.
    pub(crate) async fn remove(&self, asset_id: &str) -> Result<(), SessionError> {
        self.engine
            .execute(
                &format!("DELETE FROM {} WHERE asset_id = ?", self.table()),
                &[Value::Text(asset_id.into())],
            )
            .await?;
        Ok(())
    }

    /// Build the full-text index again, so rows added since are in it.
    pub(crate) async fn reindex(&self) -> Result<(), SessionError> {
        // There is none the first time.
        let _ = self
            .engine
            .execute(
                &format!("DROP INDEX {INDEX} ON {}", self.dataset_literal()),
                &[],
            )
            .await;
        let rows = self
            .query(&format!("SELECT count(*) AS n FROM {}", self.table()), &[])
            .await?;
        let empty = rows
            .first()
            .and_then(|b| {
                b.column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .map(|a| a.value(0))
            })
            .unwrap_or(0)
            == 0;
        if !empty {
            self.engine
                .execute(
                    &format!(
                        "CREATE INDEX {INDEX} ON {} (text) USING INVERTED WITH ({INDEX_WITH})",
                        self.dataset_literal()
                    ),
                    &[],
                )
                .await?;
        }
        Ok(())
    }

    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<RecordBatch>, SessionError> {
        let (_, stream) = self.engine.query(sql, params).await?;
        Ok(stream.try_collect().await?)
    }

    /// Up to `k` chunks matching `query` by full text, best first. The query
    /// is bound as a parameter, never written into the SQL.
    pub(crate) async fn search(
        &self,
        query: &str,
        k: u32,
    ) -> Result<Vec<StoredChunk>, SessionError> {
        let batches = self
            .query(
                &format!(
                    "SELECT {SELECT}, _score FROM lance_fts({}, 'text', ?, k => {k}) \
                     ORDER BY _score DESC, chunk_id",
                    self.dataset_literal()
                ),
                &[Value::Text(query.into())],
            )
            .await?;
        chunks_of(&batches, true)
    }

    /// The chunks of one conversion, in order.
    pub(crate) async fn conversion(
        &self,
        conversion_id: &str,
    ) -> Result<Vec<StoredChunk>, SessionError> {
        let batches = self
            .query(
                &format!(
                    "SELECT {SELECT} FROM {} WHERE conversion_id = ? ORDER BY seq",
                    self.table()
                ),
                &[Value::Text(conversion_id.into())],
            )
            .await?;
        chunks_of(&batches, false)
    }

    /// One chunk, by id.
    pub(crate) async fn chunk(&self, chunk_id: &str) -> Result<Option<StoredChunk>, SessionError> {
        let batches = self
            .query(
                &format!("SELECT {SELECT} FROM {} WHERE chunk_id = ?", self.table()),
                &[Value::Text(chunk_id.into())],
            )
            .await?;
        Ok(chunks_of(&batches, false)?.into_iter().next())
    }
}

/// Strings, whatever string type the engine handed back.
fn strings(batch: &RecordBatch, name: &str) -> Result<StringArray, SessionError> {
    let array = batch
        .column_by_name(name)
        .ok_or_else(|| engine_err(format!("the chunks have no {name} column")))?;
    let cast: ArrayRef = arrow_cast::cast(array, &DataType::Utf8)?;
    cast.as_any()
        .downcast_ref::<StringArray>()
        .cloned()
        .ok_or_else(|| engine_err(format!("{name} is not text")))
}

fn ints(batch: &RecordBatch, name: &str, ty: DataType) -> Result<ArrayRef, SessionError> {
    let array = batch
        .column_by_name(name)
        .ok_or_else(|| engine_err(format!("the chunks have no {name} column")))?;
    Ok(arrow_cast::cast(array, &ty)?)
}

fn chunks_of(batches: &[RecordBatch], scored: bool) -> Result<Vec<StoredChunk>, SessionError> {
    let mut out = Vec::new();
    for batch in batches {
        let ids = strings(batch, "chunk_id")?;
        let assets = strings(batch, "asset_id")?;
        let conversions = strings(batch, "conversion_id")?;
        let paths = strings(batch, "path")?;
        let headings = strings(batch, "heading")?;
        let texts = strings(batch, "text")?;
        let seq = ints(batch, "seq", DataType::Int32)?;
        let page = ints(batch, "page", DataType::Int32)?;
        let line_start = ints(batch, "line_start", DataType::Int32)?;
        let line_end = ints(batch, "line_end", DataType::Int32)?;
        let char_start = ints(batch, "char_start", DataType::Int64)?;
        let char_end = ints(batch, "char_end", DataType::Int64)?;
        let i32s = |a: &ArrayRef| a.as_any().downcast_ref::<Int32Array>().cloned();
        let i64s = |a: &ArrayRef| a.as_any().downcast_ref::<Int64Array>().cloned();
        let (seq, page, line_start, line_end) = (
            i32s(&seq).ok_or_else(|| engine_err("seq"))?,
            i32s(&page).ok_or_else(|| engine_err("page"))?,
            i32s(&line_start).ok_or_else(|| engine_err("line_start"))?,
            i32s(&line_end).ok_or_else(|| engine_err("line_end"))?,
        );
        let (char_start, char_end) = (
            i64s(&char_start).ok_or_else(|| engine_err("char_start"))?,
            i64s(&char_end).ok_or_else(|| engine_err("char_end"))?,
        );
        let scores = if scored {
            let array = batch
                .column_by_name("_score")
                .ok_or_else(|| engine_err("a search returned no _score"))?;
            Some(arrow_cast::cast(array, &DataType::Float64)?)
        } else {
            None
        };
        let scores = scores
            .as_ref()
            .and_then(|a| a.as_any().downcast_ref::<Float64Array>());
        for i in 0..batch.num_rows() {
            out.push(StoredChunk {
                chunk_id: ids.value(i).to_string(),
                asset_id: assets.value(i).to_string(),
                conversion_id: conversions.value(i).to_string(),
                path: paths.value(i).to_string(),
                seq: seq.value(i),
                heading: serde_json::from_str(headings.value(i)).unwrap_or_default(),
                text: texts.value(i).to_string(),
                page: (!page.is_null(i)).then(|| page.value(i)),
                char_start: char_start.value(i),
                char_end: char_end.value(i),
                line_start: line_start.value(i),
                line_end: line_end.value(i),
                score: scores.map(|s| s.value(i)),
            });
        }
    }
    Ok(out)
}
