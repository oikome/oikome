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

//! Searching what a collection holds (M3a), for the node's owner.
//!
//! The collections sources fill are transactional Spaces, and the node keeps
//! a full-text index for each of their tables: an FTS5 table over the text
//! columns, hidden (`_oikome_fts_<table>`), reading the rows of the table it
//! indexes (external content) and kept in step by triggers. Every write, a
//! pull or the owner's own SQL, updates it in the same transaction. The
//! `trigram` tokenizer matches any three or more characters inside a word,
//! so words of languages written without spaces (Chinese, Japanese) are
//! found too; shorter terms are matched with `LIKE`.
//!
//! Other collections (analytical, or a table with no index) are read up to
//! [`MAX_SCAN_ROWS`] rows and matched here. Collections shared with you are
//! not searched: a search runs only where the data is.
//!
//! What the owner types never becomes SQL text: each term is an FTS5 string
//! (quotes doubled) or a `LIKE` pattern, both bound as parameters; names come
//! from the catalog and are quoted.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Array, Float64Array, RecordBatch, StringArray};
use arrow_schema::DataType;
use futures::TryStreamExt;
use oikome_core::SpaceKind;
use oikome_engine::name::quote_ident;
use oikome_engine::{QueryEngine, Value};

use crate::error::SessionError;
use crate::result::QueryResult;
use crate::session::OikomeSession;

/// Names of the full-text index of a table, and of its triggers, start so.
pub const INDEX_PREFIX: &str = "_oikome_fts_";
/// Rows of a collection with no index that a search reads.
pub const MAX_SCAN_ROWS: usize = 10_000;
/// The FTS5 tokenizer: any three or more characters, case and accents aside.
const TOKENIZER: &str = "trigram remove_diacritics 1";
/// A term shorter than this (in characters) cannot use a trigram index.
pub(crate) const MIN_INDEXED_CHARS: usize = 3;

/// A row a search found.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    pub table: String,
    /// The row's primary key, each column as text, when the table has one.
    pub key: Option<Vec<String>>,
    /// The row's text columns, by name, in the table's order.
    pub texts: Vec<(String, String)>,
    /// How well it matched (BM25; lower is better), when an index ranked it.
    pub score: Option<f64>,
}

fn index_name(table: &str) -> String {
    format!("{INDEX_PREFIX}{table}")
}

fn qualified(space: &str, table: &str) -> String {
    format!("{}.{}", quote_ident(space), quote_ident(table))
}

/// An SQL string literal of a name that comes from the catalog.
fn literal(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

async fn rows(
    engine: &Arc<dyn QueryEngine>,
    sql: &str,
    params: &[Value],
) -> Result<Vec<RecordBatch>, SessionError> {
    let (_, stream) = engine.query(sql, params).await?;
    Ok(stream.try_collect().await?)
}

/// The first column of every row, as text.
async fn names(
    engine: &Arc<dyn QueryEngine>,
    sql: &str,
    params: &[Value],
) -> Result<Vec<String>, SessionError> {
    let mut out = Vec::new();
    for batch in rows(engine, sql, params).await? {
        out.extend(texts_of(batch.column(0)).into_iter().flatten());
    }
    Ok(out)
}

/// A column's values as text (`None` for null).
fn texts_of(array: &dyn Array) -> Vec<Option<String>> {
    let cast = arrow_cast::cast(array, &DataType::Utf8).ok();
    match cast
        .as_ref()
        .and_then(|a| a.as_any().downcast_ref::<StringArray>())
    {
        Some(strings) => (0..strings.len())
            .map(|i| (!strings.is_null(i)).then(|| strings.value(i).to_string()))
            .collect(),
        None => vec![None; array.len()],
    }
}

/// The columns of `table` in `space`, as SQLite lists them.
async fn columns(
    engine: &Arc<dyn QueryEngine>,
    space: &str,
    table: &str,
) -> Result<Vec<String>, SessionError> {
    names(
        engine,
        "SELECT name FROM pragma_table_info(?1, ?2) ORDER BY cid",
        &[
            Value::Text(table.to_string()),
            Value::Text(space.to_string()),
        ],
    )
    .await
}

/// The primary key columns of `table` in `space`, in key order.
async fn key_columns(
    engine: &Arc<dyn QueryEngine>,
    space: &str,
    table: &str,
) -> Result<Vec<String>, SessionError> {
    names(
        engine,
        "SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk",
        &[
            Value::Text(table.to_string()),
            Value::Text(space.to_string()),
        ],
    )
    .await
}

/// Make sure `table` in `space` has its full-text index over `text_columns`,
/// built from the rows already there when it is new or its columns changed.
/// Run it inside the transaction that writes the table.
pub(crate) async fn ensure_index(
    engine: &Arc<dyn QueryEngine>,
    space: &str,
    table: &str,
    text_columns: &[String],
) -> Result<(), SessionError> {
    // A table the connector names but has not made yet is indexed later.
    if columns(engine, space, table).await?.is_empty() {
        return Ok(());
    }
    let index = index_name(table);
    let indexed = columns(engine, space, &index).await?;
    if indexed == text_columns {
        return Ok(());
    }
    drop_index(engine, space, table).await?;
    if text_columns.is_empty() {
        return Ok(());
    }
    let cols: Vec<String> = text_columns.iter().map(|c| quote_ident(c)).collect();
    let list = cols.join(", ");
    let new: Vec<String> = cols.iter().map(|c| format!("new.{c}")).collect();
    let old: Vec<String> = cols.iter().map(|c| format!("old.{c}")).collect();
    let (new, old) = (new.join(", "), old.join(", "));
    let target = quote_ident(&index);
    let base = quote_ident(table);
    let insert = format!("INSERT INTO {target}(rowid, {list}) VALUES (new.rowid, {new});");
    let remove = format!(
        "INSERT INTO {target}({target}, rowid, {list}) VALUES ('delete', old.rowid, {old});"
    );
    let statements = [
        format!(
            "CREATE VIRTUAL TABLE {} USING fts5({list}, content={}, content_rowid='rowid', tokenize='{TOKENIZER}')",
            qualified(space, &index),
            literal(table),
        ),
        format!(
            "CREATE TRIGGER {} AFTER INSERT ON {base} BEGIN {insert} END",
            qualified(space, &format!("{index}_ai"))
        ),
        format!(
            "CREATE TRIGGER {} AFTER DELETE ON {base} BEGIN {remove} END",
            qualified(space, &format!("{index}_ad"))
        ),
        format!(
            "CREATE TRIGGER {} AFTER UPDATE ON {base} BEGIN {remove} {insert} END",
            qualified(space, &format!("{index}_au"))
        ),
        format!(
            "INSERT INTO {}({target}) VALUES ('rebuild')",
            qualified(space, &index)
        ),
    ];
    for sql in statements {
        engine.execute(&sql, &[]).await?;
    }
    Ok(())
}

/// Remove the full-text index of `table` and its triggers, if there is one.
pub(crate) async fn drop_index(
    engine: &Arc<dyn QueryEngine>,
    space: &str,
    table: &str,
) -> Result<(), SessionError> {
    let index = index_name(table);
    for trigger in ["_ai", "_ad", "_au"] {
        engine
            .execute(
                &format!(
                    "DROP TRIGGER IF EXISTS {}",
                    qualified(space, &format!("{index}{trigger}"))
                ),
                &[],
            )
            .await?;
    }
    engine
        .execute(
            &format!("DROP TABLE IF EXISTS {}", qualified(space, &index)),
            &[],
        )
        .await?;
    Ok(())
}

/// A `LIKE` pattern matching `term` anywhere, with `\` as its escape.
pub(crate) fn like_pattern(term: &str) -> String {
    let mut out = String::from("%");
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// An FTS5 query matching every term in `terms` (each a string, quotes doubled).
pub(crate) fn fts_query(terms: &[&str]) -> String {
    terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Whether every term is in one of `texts`, case aside.
fn matches_all(terms: &[String], texts: &[(String, String)]) -> bool {
    let lowered: Vec<String> = texts.iter().map(|(_, t)| t.to_lowercase()).collect();
    terms
        .iter()
        .all(|term| lowered.iter().any(|t| t.contains(&term.to_lowercase())))
}

/// Rows of `batches` as matches: `key` and `text` columns by name, the
/// score from a `score` column when there is one.
fn to_matches(
    table: &str,
    batches: &[RecordBatch],
    key: &[String],
    text: &[String],
) -> Vec<SearchMatch> {
    let mut out = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        let column = |name: &str| {
            schema
                .index_of(name)
                .ok()
                .map(|i| texts_of(batch.column(i)))
        };
        let keys: Vec<Vec<Option<String>>> = key.iter().filter_map(|k| column(k)).collect();
        let texts: Vec<(String, Vec<Option<String>>)> = text
            .iter()
            .filter_map(|t| column(t).map(|v| (t.clone(), v)))
            .collect();
        let scores = schema
            .index_of("score")
            .ok()
            .and_then(|i| arrow_cast::cast(batch.column(i), &DataType::Float64).ok());
        let scores = scores
            .as_ref()
            .and_then(|a| a.as_any().downcast_ref::<Float64Array>().cloned());
        for row in 0..batch.num_rows() {
            let key = (!key.is_empty() && keys.len() == key.len()).then(|| {
                keys.iter()
                    .map(|k| k[row].clone().unwrap_or_default())
                    .collect()
            });
            out.push(SearchMatch {
                table: table.to_string(),
                key,
                texts: texts
                    .iter()
                    .filter_map(|(name, values)| values[row].clone().map(|v| (name.clone(), v)))
                    .collect(),
                score: scores
                    .as_ref()
                    .filter(|s| !s.is_null(row))
                    .map(|s| s.value(row)),
            });
        }
    }
    out
}

impl OikomeSession {
    /// Rows of `space` with every one of `terms` in their text, at most
    /// `limit` per table: ranked by the full-text index where there is one,
    /// newest first otherwise. A collection shared with you cannot be
    /// searched here.
    ///
    /// A principal (an AI assistant) searches only what it may read: the
    /// index only finds rows, which are then read again as the principal,
    /// by their key, so its column grants and row policies decide what it
    /// gets, and a row whose words are only in columns it may not see is
    /// left out. A table with no index or no key is read as the principal.
    pub async fn search(
        &self,
        space: &str,
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<SearchMatch>, SessionError> {
        let def = self.space(space)?;
        if def.kind == SpaceKind::Remote {
            return Err(SessionError::Unsupported(format!(
                "{space} is shared with you; it is searched where it is kept"
            )));
        }
        let terms: Vec<String> = terms.iter().filter(|t| !t.is_empty()).cloned().collect();
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        if let Some(location) = self.in_place_location(space)? {
            // A folder viewed in place: its index finds files, read again as
            // a principal like any table's rows; its data files are read.
            let readable = self.readable_relations(space).await?;
            if readable.iter().any(|t| t == crate::inplace::FILES) {
                let found = crate::inplace::search(&self.catalog, &location, &terms, limit)?;
                // (The index is read synchronously; nothing large is awaited.)
                out.extend(if self.principal().is_none() {
                    found
                } else {
                    self.as_principal(
                        space,
                        crate::inplace::FILES,
                        &["path".to_string()],
                        &terms,
                        found,
                    )
                    .await?
                });
            }
            // Its data files are read where they are: one that can't be read
            // now (the place is offline) doesn't stop the search.
            for table in readable.iter().filter(|t| *t != crate::inplace::FILES) {
                match self.scan(space, table, &terms, limit, &[]).await {
                    Ok(found) => out.extend(found),
                    Err(e) => tracing::debug!(%space, %table, error = %e, "not searched"),
                }
            }
            return Ok(out);
        }
        for table in self.readable_relations(space).await? {
            let found = if def.kind == SpaceKind::Transactional {
                let engine = self.engines().sqlite_for(&def)?;
                if self.principal().is_none() {
                    self.search_sqlite(&engine, space, &table, &terms, limit)
                        .await?
                } else {
                    let key = key_columns(&engine, space, &table).await?;
                    let indexed = !columns(&engine, space, &index_name(&table))
                        .await?
                        .is_empty();
                    if indexed && !key.is_empty() {
                        let found = self
                            .search_sqlite(&engine, space, &table, &terms, limit)
                            .await?;
                        self.as_principal(space, &table, &key, &terms, found)
                            .await?
                    } else {
                        self.scan(space, &table, &terms, limit, &key).await?
                    }
                }
            } else {
                self.scan(space, &table, &terms, limit, &[]).await?
            };
            out.extend(found);
        }
        Ok(out)
    }

    /// Search one table of a transactional Space: through its index when it
    /// has one, else by reading it.
    async fn search_sqlite(
        &self,
        engine: &Arc<dyn QueryEngine>,
        space: &str,
        table: &str,
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<SearchMatch>, SessionError> {
        let key = key_columns(engine, space, table).await?;
        let index = index_name(table);
        let text = columns(engine, space, &index).await?;
        if text.is_empty() {
            return self.scan(space, table, terms, limit, &key).await;
        }
        let (long, short): (Vec<&str>, Vec<&str>) = terms
            .iter()
            .map(String::as_str)
            .partition(|t| t.chars().count() >= MIN_INDEXED_CHARS);
        let mut params: Vec<Value> = Vec::new();
        let mut conditions: Vec<String> = Vec::new();
        let target = quote_ident(&index);
        if !long.is_empty() {
            params.push(Value::Text(fts_query(&long)));
            conditions.push(format!("{target} MATCH ?{}", params.len()));
        }
        for term in short {
            params.push(Value::Text(like_pattern(term)));
            let n = params.len();
            let any: Vec<String> = text
                .iter()
                .map(|c| format!("t.{} LIKE ?{n} ESCAPE '\\'", quote_ident(c)))
                .collect();
            conditions.push(format!("({})", any.join(" OR ")));
        }
        let wanted: Vec<String> = key
            .iter()
            .chain(text.iter())
            .map(|c| format!("t.{}", quote_ident(c)))
            .collect();
        let sql = if long.is_empty() {
            format!(
                "SELECT {} FROM {} AS t WHERE {} ORDER BY t.rowid DESC LIMIT {limit}",
                wanted.join(", "),
                qualified(space, table),
                conditions.join(" AND "),
            )
        } else {
            format!(
                "SELECT {}, bm25({target}) AS score FROM {} JOIN {} AS t ON t.rowid = {target}.rowid \
                 WHERE {} ORDER BY score LIMIT {limit}",
                wanted.join(", "),
                qualified(space, &index),
                qualified(space, table),
                conditions.join(" AND "),
            )
        };
        let batches = rows(engine, &sql, &params).await?;
        Ok(to_matches(table, &batches, &key, &text))
    }

    /// Matches the index found, read again as this session's principal by
    /// their keys (`columns`): only what it may see, and only rows whose
    /// words are in what it sees.
    async fn as_principal(
        &self,
        space: &str,
        table: &str,
        columns: &[String],
        terms: &[String],
        found: Vec<SearchMatch>,
    ) -> Result<Vec<SearchMatch>, SessionError> {
        let mut out = Vec::with_capacity(found.len());
        for m in found {
            let Some(key) = m.key.clone().filter(|k| k.len() == columns.len()) else {
                continue;
            };
            let conditions: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} = ?{}", quote_ident(c), i + 1))
                .collect();
            // Names from the catalog, quoted; the key's values are bound.
            let sql = format!(
                "SELECT * FROM {} WHERE {} LIMIT 1",
                qualified(space, table),
                conditions.join(" AND ")
            );
            let params: Vec<Value> = key.iter().cloned().map(Value::Text).collect();
            let batches = match self.query_params(&sql, &params).await? {
                QueryResult::Batches { batches, .. } => batches,
                _ => continue,
            };
            let Some(batch) = batches.into_iter().find(|b| b.num_rows() > 0) else {
                continue;
            };
            let text: Vec<String> = batch
                .schema()
                .fields()
                .iter()
                .filter(|f| {
                    matches!(
                        f.data_type(),
                        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                    )
                })
                .map(|f| f.name().clone())
                .collect();
            let Some(seen) = to_matches(table, &[batch], &[], &text).into_iter().next() else {
                continue;
            };
            if matches_all(terms, &seen.texts) {
                out.push(SearchMatch {
                    texts: seen.texts,
                    ..m
                });
            }
        }
        Ok(out)
    }

    /// Search a table with no index by reading up to [`MAX_SCAN_ROWS`] rows
    /// and matching their text columns here.
    async fn scan(
        &self,
        space: &str,
        table: &str,
        terms: &[String],
        limit: usize,
        key: &[String],
    ) -> Result<Vec<SearchMatch>, SessionError> {
        // Both names come from the catalog and are quoted.
        let sql = format!(
            "SELECT * FROM {} LIMIT {MAX_SCAN_ROWS}",
            qualified(space, table)
        );
        let batches = match self.sql(&sql).await? {
            QueryResult::Batches { batches, .. } => batches,
            _ => return Ok(Vec::new()),
        };
        let Some(first) = batches.first() else {
            return Ok(Vec::new());
        };
        let text: Vec<String> = first
            .schema()
            .fields()
            .iter()
            .filter(|f| {
                matches!(
                    f.data_type(),
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                )
            })
            .map(|f| f.name().clone())
            .collect();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<SearchMatch> = to_matches(table, &batches, key, &text)
            .into_iter()
            .filter(|m| matches_all(terms, &m.texts))
            .collect();
        out.truncate(limit);
        Ok(out)
    }

    /// The columns of `table`'s primary key, in key order: what names one
    /// of its rows for `row_by_key`. Empty for a table with none, a view, or
    /// a Space other than a transactional one. The owner's only.
    pub async fn row_key(&self, space: &str, table: &str) -> Result<Vec<String>, SessionError> {
        self.deny_principal("reading a table's key")?;
        let def = self.space(space)?;
        if table == crate::inplace::FILES && self.in_place_location(space)?.is_some() {
            return Ok(vec!["path".to_string()]);
        }
        if def.kind != SpaceKind::Transactional {
            return Ok(Vec::new());
        }
        let engine = self.engines().sqlite_for(&def)?;
        key_columns(&engine, space, table).await
    }

    /// One row of a transactional Space's table, by its primary key (each
    /// column as text, in key order). A principal reads it as itself: its
    /// column grants and row policies decide what it gets, and a row they
    /// leave out reads as no row.
    pub async fn row_by_key(
        &self,
        space: &str,
        table: &str,
        key: &[String],
    ) -> Result<QueryResult, SessionError> {
        let def = self.space(space)?;
        if table == crate::inplace::FILES && self.in_place_location(space)?.is_some() {
            // A file of a folder viewed in place, by its path.
            if key.len() != 1 {
                return Err(SessionError::Unsupported(format!(
                    "{space}.{table} is read by a file's path"
                )));
            }
            let sql = format!(
                "SELECT * FROM {} WHERE path = ?1 LIMIT 1",
                qualified(space, table)
            );
            return self
                .query_params(&sql, &[Value::Text(key[0].clone())])
                .await;
        }
        if def.kind != SpaceKind::Transactional {
            return Err(SessionError::Unsupported(format!(
                "rows of {space} are not read by key"
            )));
        }
        if !self
            .readable_relations(space)
            .await?
            .iter()
            .any(|t| t == table)
        {
            return Err(SessionError::Engine(
                oikome_engine::EngineError::TableNotFound(format!("{space}.{table}")),
            ));
        }
        let engine = self.engines().sqlite_for(&def)?;
        let columns = key_columns(&engine, space, table).await?;
        if columns.is_empty() || columns.len() != key.len() {
            return Err(SessionError::Unsupported(format!(
                "{space}.{table} is read by a key of {} columns",
                columns.len()
            )));
        }
        let conditions: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} = ?{}", quote_ident(c), i + 1))
            .collect();
        let sql = format!(
            "SELECT * FROM {} WHERE {} LIMIT 1",
            qualified(space, table),
            conditions.join(" AND ")
        );
        let params: Vec<Value> = key.iter().map(|k| Value::Text(k.clone())).collect();
        if self.principal().is_some() {
            return self.query_params(&sql, &params).await;
        }
        let (schema, stream) = engine.query(&sql, &params).await?;
        let batches: Vec<RecordBatch> = stream.try_collect().await?;
        Ok(QueryResult::Batches { schema, batches })
    }
}

/// The text columns of each table a connector writes, by table name.
pub(crate) fn text_columns(
    tables: &HashMap<String, oikome_connect::TableDef>,
) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = tables
        .values()
        .map(|t| {
            (
                t.name.clone(),
                t.columns
                    .iter()
                    .filter(|c| c.kind == oikome_connect::ColumnKind::Text)
                    .map(|c| c.name.clone())
                    .collect(),
            )
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_become_strings_and_patterns_not_syntax() {
        assert_eq!(fts_query(&["pessoa"]), "\"pessoa\"");
        assert_eq!(fts_query(&["a\"b", "NEAR("]), "\"a\"\"b\" AND \"NEAR(\"");
        assert_eq!(like_pattern("50%_\\"), "%50\\%\\_\\\\%");
    }

    #[test]
    fn every_term_must_be_somewhere() {
        let texts = vec![
            ("title".to_string(), "Pastéis de Belém".to_string()),
            ("place".to_string(), "Belém".to_string()),
        ];
        assert!(matches_all(&["belém".into(), "PASTÉIS".into()], &texts));
        assert!(!matches_all(&["belém".into(), "tram".into()], &texts));
    }
}
