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

//! Sources (architecture v3 §11): a connector bringing one app's data into
//! a transactional Space of the same name.
//!
//! A pull is applied in one SQLite transaction on a private connection:
//! changed rows are upserted by their source key (rows that did not change
//! are left alone), deleted rows removed, and the connector's cursor stored
//! in the Space's hidden `_oikome_source` table. If anything fails the
//! transaction rolls back, so data and cursor stay as they were and the
//! next pull starts from the same place.
//!
//! The owner adds sources from the apps or the command line; a source's
//! token is kept in the node's secret store (`source/<name>/token`), with
//! `OIKOME_SOURCE_<NAME>_TOKEN` overriding it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use oikome_catalog::{SourceDef, SourceFailure};
use oikome_connect::{ConnectError, Connector, Pull, SourceConfig, TableDef, Value};
use oikome_core::{CatalogError, CreateSpaceRequest, SpaceKind};
use oikome_engine::name::quote_ident;
use oikome_engine::QueryEngine;
use oikome_identity::unix_now_ms;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::error::SessionError;
use crate::result::QueryResult;
use crate::session::OikomeSession;
use crate::subscribe::lock_location;

/// The hidden table holding each source's cursor, in the source's Space.
const CURSOR_TABLE: &str = "_oikome_source";
/// The temporary table of the keys a complete pull listed.
const KEYS_TABLE: &str = "_oikome_keys";

fn token_env(source: &str) -> String {
    format!(
        "OIKOME_SOURCE_{}_TOKEN",
        source.to_ascii_uppercase().replace('-', "_")
    )
}

fn token_account(source: &str) -> String {
    format!("source/{source}/token")
}

fn connect_error(source: &str, e: ConnectError) -> SessionError {
    SessionError::Unsupported(format!("source '{source}': {e}"))
}

/// One table of a [`PullReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulledTable {
    pub table: String,
    /// Rows inserted or changed.
    pub upserted: u64,
    pub deleted: u64,
}

/// What a pull did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullReport {
    pub source: String,
    pub tables: Vec<PulledTable>,
}

impl PullReport {
    /// The report as a result set: `(table, upserted, deleted)`.
    pub fn into_result(self) -> Result<QueryResult, SessionError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("table", DataType::Utf8, false),
            Field::new("upserted", DataType::Int64, false),
            Field::new("deleted", DataType::Int64, false),
        ]));
        let t = &self.tables;
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from_iter_values(t.iter().map(|r| &r.table))),
            Arc::new(Int64Array::from_iter_values(
                t.iter().map(|r| r.upserted as i64),
            )),
            Arc::new(Int64Array::from_iter_values(
                t.iter().map(|r| r.deleted as i64),
            )),
        ];
        let batch = arrow_array::RecordBatch::try_new(schema.clone(), columns)?;
        Ok(QueryResult::Batches {
            schema,
            batches: vec![batch],
        })
    }
}

/// The last pull of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastPull {
    pub pulled_at_ms: u64,
    pub upserted: u64,
    pub deleted: u64,
}

/// `"space"."table"`.
fn qualified(space: &str, table: &str) -> String {
    format!("{}.{}", quote_ident(space), quote_ident(table))
}

/// `INSERT … ON CONFLICT (key) DO UPDATE` that changes a row only when one
/// of its values differs, so unchanged rows are not counted.
fn upsert_sql(space: &str, table: &TableDef) -> String {
    let columns: Vec<String> = table.columns.iter().map(|c| quote_ident(&c.name)).collect();
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
    let key: Vec<String> = table.key.iter().map(|k| quote_ident(k)).collect();
    let rest: Vec<&String> = table
        .columns
        .iter()
        .zip(&columns)
        .filter(|(c, _)| !table.key.contains(&c.name))
        .map(|(_, q)| q)
        .collect();
    let target = qualified(space, &table.name);
    let mut sql = format!(
        "INSERT INTO {target} ({}) VALUES ({}) ON CONFLICT ({}) ",
        columns.join(", "),
        placeholders.join(", "),
        key.join(", ")
    );
    if rest.is_empty() {
        sql.push_str("DO NOTHING");
    } else {
        let set: Vec<String> = rest.iter().map(|c| format!("{c} = excluded.{c}")).collect();
        let changed: Vec<String> = rest
            .iter()
            .map(|c| format!("{target}.{c} IS NOT excluded.{c}"))
            .collect();
        sql.push_str(&format!(
            "DO UPDATE SET {} WHERE {}",
            set.join(", "),
            changed.join(" OR ")
        ));
    }
    sql
}

/// Apply `pull` inside the open transaction of `engine`.
async fn apply(
    engine: &Arc<dyn QueryEngine>,
    source: &str,
    space: &str,
    tables: &HashMap<String, TableDef>,
    pull: Pull,
) -> Result<Vec<PulledTable>, SessionError> {
    let mut report: Vec<PulledTable> = Vec::new();
    let entry = |report: &mut Vec<PulledTable>, table: &str| -> usize {
        match report.iter().position(|r| r.table == table) {
            Some(i) => i,
            None => {
                report.push(PulledTable {
                    table: table.to_string(),
                    upserted: 0,
                    deleted: 0,
                });
                report.len() - 1
            }
        }
    };
    let def = |table: &str| {
        tables.get(table).ok_or_else(|| {
            SessionError::Unsupported(format!(
                "source '{source}' wrote an unknown table '{table}'"
            ))
        })
    };
    let mut listed: HashMap<String, Vec<Vec<Value>>> = HashMap::new();
    for (table, rows) in &pull.upserts {
        let def = def(table)?;
        let sql = upsert_sql(space, def);
        let key = def.key_positions();
        let i = entry(&mut report, table);
        for row in rows {
            if row.len() != def.columns.len() {
                return Err(SessionError::Unsupported(format!(
                    "source '{source}': a row of {table} has {} values for {} columns",
                    row.len(),
                    def.columns.len()
                )));
            }
            report[i].upserted += engine.execute(&sql, row).await?;
            if pull.complete {
                listed
                    .entry(table.clone())
                    .or_default()
                    .push(key.iter().map(|&k| row[k].clone()).collect());
            }
        }
    }
    for (table, keys) in &pull.deletes {
        let def = def(table)?;
        let filter: Vec<String> = def
            .key
            .iter()
            .enumerate()
            .map(|(n, k)| format!("{} = ?{}", quote_ident(k), n + 1))
            .collect();
        let sql = format!(
            "DELETE FROM {} WHERE {}",
            qualified(space, table),
            filter.join(" AND ")
        );
        let i = entry(&mut report, table);
        for key in keys {
            report[i].deleted += engine.execute(&sql, key).await?;
        }
    }
    if pull.complete {
        // Rows the source no longer lists were deleted there.
        for (table, _) in &pull.upserts {
            let def = def(table)?;
            let columns: Vec<String> = (0..def.key.len()).map(|n| format!("k{n}")).collect();
            engine
                .execute(&format!("DROP TABLE IF EXISTS temp.{KEYS_TABLE}"), &[])
                .await?;
            engine
                .execute(
                    &format!("CREATE TEMP TABLE {KEYS_TABLE} ({})", columns.join(", ")),
                    &[],
                )
                .await?;
            let insert = format!(
                "INSERT INTO temp.{KEYS_TABLE} VALUES ({})",
                (1..=columns.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for key in listed.get(table).into_iter().flatten() {
                engine.execute(&insert, key).await?;
            }
            let target = qualified(space, table);
            let matches: Vec<String> = def
                .key
                .iter()
                .zip(&columns)
                .map(|(k, c)| format!("listed.{c} IS {target}.{}", quote_ident(k)))
                .collect();
            let i = entry(&mut report, table);
            report[i].deleted += engine
                .execute(
                    &format!(
                        "DELETE FROM {target} WHERE NOT EXISTS \
                         (SELECT 1 FROM temp.{KEYS_TABLE} AS listed WHERE {})",
                        matches.join(" AND ")
                    ),
                    &[],
                )
                .await?;
            engine
                .execute(&format!("DROP TABLE temp.{KEYS_TABLE}"), &[])
                .await?;
        }
    }
    let upserted = report.iter().map(|r| r.upserted).sum::<u64>();
    let deleted = report.iter().map(|r| r.deleted).sum::<u64>();
    engine
        .execute(
            &format!(
                "INSERT INTO {} (source, cursor, pulled_at_ms, upserted, deleted) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (source) DO UPDATE SET \
                 cursor = excluded.cursor, pulled_at_ms = excluded.pulled_at_ms, \
                 upserted = excluded.upserted, deleted = excluded.deleted",
                qualified(space, CURSOR_TABLE)
            ),
            &[
                Value::Text(source.to_string()),
                pull.cursor.map(Value::Text).unwrap_or(Value::Null),
                Value::Int64(unix_now_ms() as i64),
                Value::Int64(upserted as i64),
                Value::Int64(deleted as i64),
            ],
        )
        .await?;
    Ok(report)
}

impl OikomeSession {
    /// The configuration of the source `name`.
    fn source_config(&self, name: &str) -> Result<(SourceDef, SourceConfig), SessionError> {
        let def = self.catalog.get_source(name)?;
        let config: SourceConfig = serde_json::from_value(def.config.clone()).map_err(|e| {
            CatalogError::Registry(format!("source '{name}' has unreadable settings: {e}"))
        })?;
        Ok((def, config))
    }

    /// The connector of the source `name`, with its token.
    fn connector(
        &self,
        name: &str,
        config: &SourceConfig,
    ) -> Result<Box<dyn Connector>, SessionError> {
        let token = if config.needs_token() {
            self.engines()
                .secret(&token_env(name), &token_account(name))?
        } else {
            None
        };
        config.connector(token).map_err(|e| connect_error(name, e))
    }

    /// Create the tables of `tables` and the cursor table in the Space
    /// `space`, unless they exist with the same columns.
    async fn ensure_tables(&self, space: &str, tables: &[TableDef]) -> Result<(), SessionError> {
        let engine = self.engines().sqlite_for(&self.space(space)?)?;
        let existing = engine.table_names(space).await?;
        for table in tables {
            if existing.contains(&table.name) {
                let schema = engine
                    .table_schema(&oikome_engine::QualifiedName::new(space, &table.name))
                    .await?;
                let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
                let wanted: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
                if names != wanted {
                    return Err(SessionError::AlreadyExists(format!(
                        "{space}.{} exists with the columns {names:?}, not {wanted:?}",
                        table.name
                    )));
                }
                continue;
            }
            let mut columns: Vec<String> = table
                .columns
                .iter()
                .map(|c| format!("{} {}", quote_ident(&c.name), c.kind.sql_type()))
                .collect();
            let key: Vec<String> = table.key.iter().map(|k| quote_ident(k)).collect();
            columns.push(format!("PRIMARY KEY ({})", key.join(", ")));
            engine
                .execute(
                    &format!(
                        "CREATE TABLE {} ({})",
                        qualified(space, &table.name),
                        columns.join(", ")
                    ),
                    &[],
                )
                .await?;
        }
        engine
            .execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {} (source TEXT PRIMARY KEY, cursor TEXT, \
                     pulled_at_ms INTEGER NOT NULL, upserted INTEGER NOT NULL, \
                     deleted INTEGER NOT NULL)",
                    qualified(space, CURSOR_TABLE)
                ),
                &[],
            )
            .await?;
        Ok(())
    }

    /// Add the source `name`: register it, keep its token in the node's
    /// secret store, create its transactional Space (or use the one of that
    /// name) and its tables, and pull it once. If anything fails nothing is
    /// kept, except the Space's file if it already existed.
    pub async fn add_source(
        &self,
        name: &str,
        config: SourceConfig,
        every: Option<Duration>,
        token: Option<Zeroizing<String>>,
    ) -> Result<PullReport, SessionError> {
        self.deny_principal("adding a source")?;
        let settings = serde_json::to_value(&config)
            .map_err(|e| CatalogError::Registry(format!("source settings: {e}")))?;
        self.catalog
            .add_source(name, settings, every.map(|e| e.as_secs().max(60)))?;
        let mut created_space = false;
        let added = async {
            if let Some(token) = &token {
                self.engines()
                    .secret_backend()?
                    .set(&token_account(name), token)?;
            }
            match self.catalog.get_space(name) {
                Ok(space) if space.kind == SpaceKind::Transactional => {}
                Ok(_) => {
                    return Err(SessionError::Unsupported(format!(
                        "space '{name}' exists and is not transactional; choose another source name"
                    )))
                }
                Err(_) => {
                    let mut request = CreateSpaceRequest::new(name);
                    request.kind = Some(SpaceKind::Transactional);
                    self.catalog.create_space(request).await?;
                    created_space = true;
                }
            }
            let connector = self.connector(name, &config)?;
            self.ensure_tables(name, &connector.tables()).await?;
            self.pull_with(name, connector.as_ref()).await
        }
        .await;
        if added.is_err() {
            let _ = self.catalog.remove_source(name);
            if let Ok(backend) = self.engines().secret_backend() {
                let _ = backend.delete(&token_account(name));
            }
            if created_space {
                let _ = self.catalog.drop_space(name);
            }
        }
        added
    }

    /// Pull the source `name` now. A failure is kept with the source (see
    /// [`SourceDef::failure`]) until a pull succeeds.
    pub async fn pull_source(&self, name: &str) -> Result<PullReport, SessionError> {
        self.deny_principal("pulling a source")?;
        let pulled = async {
            let (_, config) = self.source_config(name)?;
            let connector = self.connector(name, &config)?;
            self.pull_with(name, connector.as_ref()).await
        }
        .await;
        let failure = pulled.as_ref().err().map(|e| SourceFailure {
            at_ms: unix_now_ms(),
            message: e.to_string(),
        });
        let had = self
            .catalog
            .get_source(name)
            .is_ok_and(|def| def.failure.is_some());
        if failure.is_some() || had {
            if let Err(e) = self.catalog.set_source_failure(name, failure) {
                tracing::warn!(source = %name, error = %e, "keeping the pull's outcome failed");
            }
        }
        pulled
    }

    /// Pull the source `name` with `connector` in place of its own, e.g. a
    /// connector built elsewhere.
    pub async fn pull_source_with(
        &self,
        name: &str,
        connector: &dyn Connector,
    ) -> Result<PullReport, SessionError> {
        self.deny_principal("pulling a source")?;
        self.catalog.get_source(name)?;
        self.pull_with(name, connector).await
    }

    async fn pull_with(
        &self,
        name: &str,
        connector: &dyn Connector,
    ) -> Result<PullReport, SessionError> {
        let space = self.space(name)?;
        if space.kind != SpaceKind::Transactional {
            return Err(SessionError::Unsupported(format!(
                "source '{name}' writes space '{name}', which is no longer transactional"
            )));
        }
        let _lock = lock_location(&self.catalog, &format!("source-{name}")).await?;
        let shared = self.engines().sqlite_for(&space)?;
        let cursor = {
            let (_, stream) = shared
                .query(
                    &format!(
                        "SELECT cursor FROM {} WHERE source = ?1",
                        qualified(name, CURSOR_TABLE)
                    ),
                    &[Value::Text(name.to_string())],
                )
                .await?;
            let batches: Vec<arrow_array::RecordBatch> = stream.try_collect().await?;
            batches.iter().filter(|b| b.num_rows() > 0).find_map(|b| {
                b.column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .filter(|a| !a.is_null(0))
                    .map(|a| a.value(0).to_string())
            })
        };
        let pull = connector
            .pull(cursor.as_deref())
            .await
            .map_err(|e| connect_error(name, e))?;
        let tables: HashMap<String, TableDef> = connector
            .tables()
            .into_iter()
            .map(|t| (t.name.clone(), t))
            .collect();

        let engine = self.engines().sqlite_private_for(&space)?;
        let tx = engine.begin().await?;
        match apply(&engine, name, name, &tables, pull).await {
            Ok(tables) => {
                engine.commit(tx).await?;
                Ok(PullReport {
                    source: name.to_string(),
                    tables,
                })
            }
            Err(e) => {
                if let Err(undo) = engine.rollback(tx).await {
                    tracing::warn!(source = %name, error = %undo, "rolling back a failed pull failed");
                }
                Err(e)
            }
        }
    }

    /// When the source `name` was last pulled, and what that pull did.
    pub async fn last_pull(&self, name: &str) -> Result<Option<LastPull>, SessionError> {
        let engine = self.engines().sqlite_for(&self.space(name)?)?;
        let (_, stream) = engine
            .query(
                &format!(
                    "SELECT pulled_at_ms, upserted, deleted FROM {} WHERE source = ?1",
                    qualified(name, CURSOR_TABLE)
                ),
                &[Value::Text(name.to_string())],
            )
            .await?;
        let batches: Vec<arrow_array::RecordBatch> = stream.try_collect().await?;
        let Some(batch) = batches.into_iter().find(|b| b.num_rows() > 0) else {
            return Ok(None);
        };
        let int = |i: usize| {
            batch
                .column(i)
                .as_any()
                .downcast_ref::<Int64Array>()
                .map(|a| a.value(0).max(0) as u64)
                .unwrap_or(0)
        };
        Ok(Some(LastPull {
            pulled_at_ms: int(0),
            upserted: int(1),
            deleted: int(2),
        }))
    }

    /// Unregister the source `name` and forget its token; its Space and
    /// data stay (`DROP SPACE` removes them).
    pub fn remove_source(&self, name: &str) -> Result<(), SessionError> {
        self.deny_principal("removing a source")?;
        self.catalog.remove_source(name)?;
        self.engines()
            .secret_backend()?
            .delete(&token_account(name))
    }

    /// Pull every source whose interval has passed since its last pull.
    pub async fn pull_due_sources(&self) {
        let now = unix_now_ms();
        for def in self.catalog.list_sources() {
            let Some(every) = def.every_secs else {
                continue;
            };
            let due = match self.last_pull(&def.name).await {
                Ok(Some(last)) => now.saturating_sub(last.pulled_at_ms) >= every * 1000,
                Ok(None) => true,
                Err(e) => {
                    tracing::warn!(source = %def.name, error = %e, "reading the last pull failed");
                    continue;
                }
            };
            if due {
                if let Err(e) = self.pull_source(&def.name).await {
                    tracing::warn!(source = %def.name, error = %e, "scheduled pull failed");
                }
            }
        }
    }
}

/// Pull sources on their schedule on a background task, checking every
/// `check_every`. Failures are logged; the source's data and cursor stay
/// as they were. Abort the returned handle to stop.
pub fn spawn_source_sync(session: Arc<OikomeSession>, check_every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(check_every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = session.catalog.refresh() {
                tracing::warn!(error = %e, "reloading the catalog failed");
            }
            session.pull_due_sources().await;
        }
    })
}
