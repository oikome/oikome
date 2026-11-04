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

//! The access log (architecture v3 §9): one row for each request a peer or
//! a browser sent this node, allowed or not, so the owner can see who read
//! what and when.
//!
//! Rows live in `.oikome/access_log.sqlite`. Requests never wait for the
//! log: [`AccessLog::record`] hands the entry to a writer thread through a
//! bounded channel and drops it (counting the drop) when the channel is
//! full. Failed authentications carry no caller and can be sent by anyone
//! who reaches the port, so they are merged into one row per method and
//! minute, and kept under a cap of their own.
//!
//! The names of callers that are gone (a removed contact or plugin) are
//! kept beside the rows, as long as the rows are, so a line still says who
//! it was.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// File name under `.oikome/`.
pub const ACCESS_LOG_FILE: &str = "access_log.sqlite";
/// Entries waiting for the writer before new ones are dropped.
const CHANNEL_CAPACITY: usize = 10_000;
/// Rows older than this are removed.
const RETENTION: Duration = Duration::from_secs(90 * 24 * 3600);
/// Most rows kept in total.
const MAX_ROWS: i64 = 100_000;
/// Most rows kept for failed authentications (rows without a caller).
const MAX_ANONYMOUS_ROWS: i64 = 10_000;
/// The log is pruned after this many writes (and when it is opened).
const PRUNE_EVERY: u64 = 1_000;
/// Longest stored reason, in bytes.
const MAX_REASON_BYTES: usize = 300;
/// Most rows one [`AccessLog::list`] returns.
pub const MAX_LIST_ROWS: u32 = 500;

/// How a request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Served.
    Allowed,
    /// Refused: authentication or authorization failed.
    Denied,
    /// Authorized, but failed for another reason (a bad query, a missing
    /// table, an internal error).
    Failed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
            Outcome::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "allowed" => Outcome::Allowed,
            "denied" => Outcome::Denied,
            _ => Outcome::Failed,
        }
    }
}

/// One request, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessEntry {
    pub at_ms: u64,
    /// The authenticated caller's DID; `None` when authentication failed.
    pub caller: Option<String>,
    /// The protocol method (`do_get`, `oikome.snapshot`, ...).
    pub method: String,
    /// Spaces the request named.
    pub spaces: Vec<String>,
    /// Id of the UCAN that authorized it.
    pub token_id: Option<String>,
    pub outcome: Outcome,
    /// Why it was refused or failed; never shown to the caller.
    pub reason: Option<String>,
}

/// A stored row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRecord {
    pub id: i64,
    /// When it happened; for merged failed authentications, the first one.
    pub at_ms: u64,
    pub caller: Option<String>,
    pub method: String,
    pub spaces: Vec<String>,
    pub token_id: Option<String>,
    pub outcome: Outcome,
    pub reason: Option<String>,
    /// Requests merged into this row (failed authentications); 1 otherwise.
    pub count: u64,
}

/// Which rows [`AccessLog::list`] returns, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessQuery {
    /// Only rows older than this id (the last id of the previous page).
    pub before_id: Option<i64>,
    /// At most this many rows; capped at [`MAX_LIST_ROWS`].
    pub limit: u32,
    /// Only requests by this DID.
    pub caller: Option<String>,
    /// Only requests naming this Space.
    pub space: Option<String>,
}

/// Errors reading or opening the log.
#[derive(Debug, thiserror::Error)]
pub enum AccessLogError {
    #[error("access log: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("access log: {0}")]
    Io(#[from] std::io::Error),
}

/// The node's access log. Cloning shares the writer.
#[derive(Debug, Clone)]
pub struct AccessLog {
    path: PathBuf,
    tx: SyncSender<AccessEntry>,
    dropped: Arc<AtomicU64>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn open_connection(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS access (
             id       INTEGER PRIMARY KEY AUTOINCREMENT,
             at_ms    INTEGER NOT NULL,
             caller   TEXT,
             method   TEXT NOT NULL,
             spaces   TEXT NOT NULL,
             token_id TEXT,
             outcome  TEXT NOT NULL,
             reason   TEXT,
             count    INTEGER NOT NULL DEFAULT 1
         );
         CREATE INDEX IF NOT EXISTS access_caller ON access (caller, id);
         CREATE TABLE IF NOT EXISTS names (
             caller TEXT PRIMARY KEY,
             name   TEXT NOT NULL,
             at_ms  INTEGER NOT NULL
         );",
    )?;
    Ok(conn)
}

/// Spaces stored as `,a,b,` so one can be found with `instr(spaces, ',a,')`
/// (`LIKE` would treat `_` in a name as a wildcard).
fn encode_spaces(spaces: &[String]) -> String {
    if spaces.is_empty() {
        String::new()
    } else {
        format!(",{},", spaces.join(","))
    }
}

fn decode_spaces(text: &str) -> Vec<String> {
    text.split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn clip(reason: Option<String>) -> Option<String> {
    reason.map(|mut r| {
        if r.len() > MAX_REASON_BYTES {
            let mut end = MAX_REASON_BYTES;
            while !r.is_char_boundary(end) {
                end -= 1;
            }
            r.truncate(end);
        }
        r
    })
}

fn prune(conn: &Connection) -> Result<(), rusqlite::Error> {
    let cutoff = now_ms().saturating_sub(RETENTION.as_millis() as u64) as i64;
    // The setting up of the data center stays; everything else goes in time.
    conn.execute(
        "DELETE FROM access WHERE at_ms < ?1 AND method <> 'owner_setup'",
        params![cutoff],
    )?;
    // A name is kept as long as the lines it was remembered for.
    conn.execute("DELETE FROM names WHERE at_ms < ?1", params![cutoff])?;
    conn.execute(
        "DELETE FROM access WHERE caller IS NULL AND id <= (
             SELECT id FROM access WHERE caller IS NULL
             ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        params![MAX_ANONYMOUS_ROWS],
    )?;
    conn.execute(
        "DELETE FROM access WHERE id <= (
             SELECT id FROM access ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        params![MAX_ROWS],
    )?;
    Ok(())
}

/// The writer thread: insert entries until every sender is gone.
fn write_loop(conn: Connection, rx: Receiver<AccessEntry>) {
    // Failed authentications of the current minute: method -> (row, minute).
    let mut merged: std::collections::HashMap<String, (i64, u64)> = Default::default();
    let mut writes = 0u64;
    for entry in rx {
        let result = (|| -> Result<(), rusqlite::Error> {
            if entry.caller.is_none() {
                let minute = entry.at_ms / 60_000;
                if let Some(&(id, m)) = merged.get(&entry.method) {
                    if m == minute {
                        let changed = conn.execute(
                            "UPDATE access SET count = count + 1 WHERE id = ?1",
                            params![id],
                        )?;
                        if changed == 1 {
                            return Ok(());
                        }
                    }
                }
            }
            conn.execute(
                "INSERT INTO access (at_ms, caller, method, spaces, token_id, outcome, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.at_ms as i64,
                    entry.caller,
                    entry.method,
                    encode_spaces(&entry.spaces),
                    entry.token_id,
                    entry.outcome.as_str(),
                    clip(entry.reason.clone()),
                ],
            )?;
            if entry.caller.is_none() {
                merged.insert(
                    entry.method.clone(),
                    (conn.last_insert_rowid(), entry.at_ms / 60_000),
                );
            }
            writes += 1;
            if writes.is_multiple_of(PRUNE_EVERY) {
                prune(&conn)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not write the access log");
        }
    }
}

impl AccessLog {
    /// Open (or create) the log in `oikome_dir` and start its writer.
    pub fn open(oikome_dir: &Path) -> Result<Self, AccessLogError> {
        std::fs::create_dir_all(oikome_dir)?;
        let path = oikome_dir.join(ACCESS_LOG_FILE);
        let conn = open_connection(&path)?;
        prune(&conn)?;
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        std::thread::Builder::new()
            .name("oikome-access-log".to_string())
            .spawn(move || write_loop(conn, rx))?;
        Ok(Self {
            path,
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Record a request without waiting. When the writer is behind, the
    /// entry is dropped and counted (see [`Self::dropped`]).
    pub fn record(&self, entry: AccessEntry) {
        match self.tx.try_send(entry) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                if self.dropped.fetch_add(1, Ordering::Relaxed) == 0 {
                    tracing::warn!("access log is behind; entries are being dropped");
                }
            }
        }
    }

    /// Keep `name` for `caller`, which is no longer a contact or a plugin
    /// here, so its lines still say who it was. Blocking.
    pub fn remember_name(&self, caller: &str, name: &str) -> Result<(), AccessLogError> {
        let conn = open_connection(&self.path)?;
        conn.execute(
            "INSERT INTO names (caller, name, at_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT (caller) DO UPDATE SET name = excluded.name, at_ms = excluded.at_ms",
            params![caller, clip(Some(name.to_string())), now_ms() as i64],
        )?;
        Ok(())
    }

    /// The names kept by [`Self::remember_name`], by caller. Blocking.
    pub fn names(&self) -> Result<std::collections::HashMap<String, String>, AccessLogError> {
        let conn = Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let mut stmt = conn.prepare("SELECT caller, name FROM names")?;
        let names = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(names)
    }

    /// Entries dropped because the writer was behind.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Rows matching `query`, newest first. Blocking: call it off the
    /// async runtime's worker threads (`spawn_blocking`).
    pub fn list(&self, query: &AccessQuery) -> Result<Vec<AccessRecord>, AccessLogError> {
        let conn = Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let limit = query.limit.clamp(1, MAX_LIST_ROWS);
        let space = query.space.as_ref().map(|s| format!(",{s},"));
        let mut stmt = conn.prepare(
            "SELECT id, at_ms, caller, method, spaces, token_id, outcome, reason, count
             FROM access
             WHERE (?1 IS NULL OR id < ?1)
               AND (?2 IS NULL OR caller = ?2)
               AND (?3 IS NULL OR instr(spaces, ?3) > 0)
             ORDER BY id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![query.before_id, query.caller, space, limit],
            |row| {
                Ok(AccessRecord {
                    id: row.get(0)?,
                    at_ms: row.get::<_, i64>(1)?.max(0) as u64,
                    caller: row.get(2)?,
                    method: row.get(3)?,
                    spaces: decode_spaces(&row.get::<_, String>(4)?),
                    token_id: row.get(5)?,
                    outcome: Outcome::parse(&row.get::<_, String>(6)?),
                    reason: row.get(7)?,
                    count: row.get::<_, i64>(8)?.max(1) as u64,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest row id, or `None` for an empty log.
    pub fn last_id(&self) -> Result<Option<i64>, AccessLogError> {
        let conn = Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(conn
            .query_row("SELECT max(id) FROM access", [], |row| row.get(0))
            .optional()?
            .flatten())
    }
}

/// An entry for a request that happened now.
pub fn entry_now(method: &str) -> AccessEntry {
    AccessEntry {
        at_ms: now_ms(),
        caller: None,
        method: method.to_string(),
        spaces: Vec::new(),
        token_id: None,
        outcome: Outcome::Failed,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(log: &AccessLog, rows: usize) -> Vec<AccessRecord> {
        for _ in 0..200 {
            let found = log
                .list(&AccessQuery {
                    limit: MAX_LIST_ROWS,
                    ..Default::default()
                })
                .unwrap();
            if found.len() >= rows {
                return found;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the access log never reached {rows} rows");
    }

    fn entry(caller: Option<&str>, spaces: &[&str], outcome: Outcome) -> AccessEntry {
        AccessEntry {
            caller: caller.map(str::to_string),
            spaces: spaces.iter().map(|s| s.to_string()).collect(),
            outcome,
            ..entry_now("do_get")
        }
    }

    #[test]
    fn records_filters_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let log = AccessLog::open(dir.path()).unwrap();
        log.record(entry(Some("did:key:a"), &["blog"], Outcome::Allowed));
        log.record(entry(Some("did:key:b"), &["my_notes"], Outcome::Denied));
        log.record(entry(
            Some("did:key:a"),
            &["blog", "orders"],
            Outcome::Failed,
        ));
        let all = wait_for(&log, 3);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].spaces, vec!["blog", "orders"]);

        let by_a = log
            .list(&AccessQuery {
                limit: 10,
                caller: Some("did:key:a".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_a.len(), 2);

        // `_` is not a wildcard: "myXnotes" would not match "my_notes" either.
        let notes = log
            .list(&AccessQuery {
                limit: 10,
                space: Some("my_notes".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].outcome, Outcome::Denied);

        let page = log
            .list(&AccessQuery {
                limit: 10,
                before_id: Some(all[0].id),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn failed_authentications_are_merged_per_minute() {
        let dir = tempfile::tempdir().unwrap();
        let log = AccessLog::open(dir.path()).unwrap();
        let at_ms = 60_000 * 1_000_000;
        for _ in 0..5 {
            log.record(AccessEntry {
                at_ms,
                ..entry(None, &[], Outcome::Denied)
            });
        }
        log.record(entry(Some("did:key:a"), &["blog"], Outcome::Allowed));
        let rows = wait_for(&log, 2);
        let anonymous: Vec<_> = rows.iter().filter(|r| r.caller.is_none()).collect();
        assert_eq!(anonymous.len(), 1);
        // The writer may still be merging the last ones.
        for _ in 0..200 {
            let rows = log
                .list(&AccessQuery {
                    limit: 10,
                    ..Default::default()
                })
                .unwrap();
            if rows.iter().any(|r| r.caller.is_none() && r.count == 5) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("failed authentications were not merged into one row");
    }

    #[test]
    fn long_reasons_are_clipped_on_a_char_boundary() {
        let reason = "é".repeat(MAX_REASON_BYTES);
        let clipped = clip(Some(reason)).unwrap();
        assert!(clipped.len() <= MAX_REASON_BYTES);
        assert!(clipped.chars().all(|c| c == 'é'));
    }

    #[test]
    fn names_are_remembered_and_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let log = AccessLog::open(dir.path()).unwrap();
        assert!(log.names().unwrap().is_empty());
        log.remember_name("did:key:a", "Sam").unwrap();
        log.remember_name("did:key:b", "Spending by category")
            .unwrap();
        log.remember_name("did:key:a", "Sam Lee").unwrap();
        let names = log.names().unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names["did:key:a"], "Sam Lee");
        assert_eq!(names["did:key:b"], "Spending by category");
    }
}
