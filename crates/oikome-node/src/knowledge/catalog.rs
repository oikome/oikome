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

//! The control plane of a collection's knowledge (docs/knowledge-schema.md
//! §4): `.oikome/knowledge/<space>/catalog.sqlite`. Assets are the originals,
//! conversions record each one's preparation and are also the queue: a
//! conversion waits as `queued` until the node takes it. The chunks
//! themselves are in Lance (store.rs); this file says which of them are
//! current and which the owner turned off.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::SessionError;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS facts(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS assets(
  asset_id TEXT PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  size INTEGER NOT NULL,
  modified_ms INTEGER NOT NULL,
  blake3 TEXT,
  state TEXT NOT NULL DEFAULT 'present',
  first_seen_ms INTEGER NOT NULL,
  last_seen_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS conversions(
  conversion_id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL,
  asset_blake3 TEXT,
  target TEXT NOT NULL,
  converter TEXT NOT NULL,
  converter_version TEXT NOT NULL,
  status TEXT NOT NULL,
  current INTEGER NOT NULL DEFAULT 0,
  stage_ms TEXT NOT NULL DEFAULT '{}',
  chunks INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  attempts INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS conversions_asset ON conversions(asset_id, created_ms);
CREATE TABLE IF NOT EXISTS chunk_overrides(
  chunk_id TEXT PRIMARY KEY,
  enabled INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
";

/// A conversion is tried this many times before it stays `failed`.
pub(crate) const ATTEMPTS: i64 = 3;

fn sql(e: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("knowledge catalog: {e}"))
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Where a conversion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Queued,
    Reading,
    Splitting,
    Ready,
    Failed,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Queued => "queued",
            Stage::Reading => "reading",
            Stage::Splitting => "splitting",
            Stage::Ready => "ready",
            Stage::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Stage {
        match s {
            "reading" => Stage::Reading,
            "splitting" => Stage::Splitting,
            "ready" => Stage::Ready,
            "failed" => Stage::Failed,
            _ => Stage::Queued,
        }
    }
}

/// An original and its latest conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetState {
    pub asset_id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub stage: Stage,
    pub chunks: u64,
    pub error: Option<String>,
    pub updated_ms: i64,
}

/// A conversion the node takes next.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub conversion_id: String,
    pub asset_id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub attempts: i64,
}

/// The connection is behind a lock so a catalog can be held across the
/// awaits of a conversion; no lock is held across one.
pub(crate) struct Catalog {
    conn: Mutex<Connection>,
}

impl Catalog {
    /// Open, or make, the catalog in `dir`.
    pub(crate) fn open(dir: &Path) -> Result<Self, SessionError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| SessionError::Unsupported(format!("knowledge catalog: {e}")))?;
        let conn = Connection::open(dir.join("catalog.sqlite")).map_err(sql)?;
        conn.busy_timeout(Duration::from_secs(10)).map_err(sql)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sql)?;
        conn.execute_batch(SCHEMA).map_err(sql)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether the owner prepared the collection, and when.
    pub(crate) fn prepared_ms(&self) -> Result<Option<i64>, SessionError> {
        self.conn()
            .query_row(
                "SELECT value FROM facts WHERE key = 'prepared_ms'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(sql)
            .map(|v| v.and_then(|v| v.parse().ok()))
    }

    pub(crate) fn mark_prepared(&self) -> Result<(), SessionError> {
        self.conn()
            .execute(
                "INSERT INTO facts(key, value) VALUES ('prepared_ms', ?1) \
                 ON CONFLICT(key) DO NOTHING",
                [now_ms().to_string()],
            )
            .map_err(sql)?;
        Ok(())
    }

    /// Bring the assets in line with the files listed now: a new or changed
    /// file is queued, a file that went is `missing`. Returns the assets
    /// that went missing, whose chunks must go.
    pub(crate) fn reconcile(
        &self,
        space: &str,
        files: &[(String, String, u64, i64)],
        converter: impl Fn(&str) -> Option<&'static str>,
        version: &str,
    ) -> Result<Vec<String>, SessionError> {
        let now = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(sql)?;
        let mut known: HashMap<String, (String, u64, i64, String)> = HashMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT path, asset_id, size, modified_ms, state FROM assets")
                .map_err(sql)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (
                            r.get::<_, String>(1)?,
                            u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                            r.get::<_, i64>(3)?,
                            r.get::<_, String>(4)?,
                        ),
                    ))
                })
                .map_err(sql)?;
            for row in rows {
                let (path, rest) = row.map_err(sql)?;
                known.insert(path, rest);
            }
        }
        let mut seen = HashSet::new();
        for (path, name, size, modified_ms) in files {
            let Some(conv) = converter(name) else {
                continue;
            };
            seen.insert(path.clone());
            let size_i = i64::try_from(*size).unwrap_or(i64::MAX);
            let changed = match known.get(path) {
                Some((_, s, m, state)) => *s != *size || *m != *modified_ms || state != "present",
                None => true,
            };
            let asset_id = asset_id(space, path);
            tx.execute(
                "INSERT INTO assets(asset_id, path, name, size, modified_ms, first_seen_ms, last_seen_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
                 ON CONFLICT(asset_id) DO UPDATE SET name = ?3, size = ?4, modified_ms = ?5, \
                 state = 'present', last_seen_ms = ?6",
                params![asset_id, path, name, size_i, modified_ms, now],
            )
            .map_err(sql)?;
            let waiting: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversions WHERE asset_id = ?1 \
                     AND status IN ('queued', 'reading', 'splitting'))",
                    [&asset_id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            let converted: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversions WHERE asset_id = ?1 \
                     AND converter_version = ?2 AND status IN ('ready', 'failed'))",
                    params![asset_id, version],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if !waiting && (changed || !converted) {
                queue(&tx, &asset_id, conv, version, now)?;
            }
        }
        let mut gone = Vec::new();
        for (path, (asset_id, _, _, state)) in &known {
            if !seen.contains(path) && state == "present" {
                tx.execute(
                    "UPDATE assets SET state = 'missing', last_seen_ms = ?2 WHERE asset_id = ?1",
                    params![asset_id, now],
                )
                .map_err(sql)?;
                tx.execute(
                    "UPDATE conversions SET current = 0 WHERE asset_id = ?1",
                    [asset_id],
                )
                .map_err(sql)?;
                gone.push(asset_id.clone());
            }
        }
        tx.commit().map_err(sql)?;
        Ok(gone)
    }

    /// Queue every failed conversion again (Retry).
    pub(crate) fn retry_failed(&self, version: &str) -> Result<u64, SessionError> {
        let now = now_ms();
        let failed: Vec<(String, String)> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT c.asset_id, c.converter FROM conversions c JOIN assets a USING (asset_id) \
                     WHERE a.state = 'present' AND c.status = 'failed' AND c.created_ms = \
                     (SELECT max(created_ms) FROM conversions WHERE asset_id = c.asset_id)",
                )
                .map_err(sql)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(sql)?;
            rows.collect::<Result<_, _>>().map_err(sql)?
        };
        for (asset_id, converter) in &failed {
            queue(&self.conn(), asset_id, converter, version, now)?;
        }
        Ok(failed.len() as u64)
    }

    /// The oldest queued conversion that may still be tried.
    pub(crate) fn next_job(&self) -> Result<Option<Job>, SessionError> {
        self.conn()
            .query_row(
                "SELECT c.conversion_id, c.asset_id, a.path, a.name, a.size, c.attempts \
                 FROM conversions c JOIN assets a USING (asset_id) \
                 WHERE c.status = 'queued' AND a.state = 'present' \
                 ORDER BY c.created_ms, a.path LIMIT 1",
                [],
                |r| {
                    Ok(Job {
                        conversion_id: r.get(0)?,
                        asset_id: r.get(1)?,
                        path: r.get(2)?,
                        name: r.get(3)?,
                        size: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                        attempts: r.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(sql)
    }

    /// Move a conversion to `stage`, noting when.
    pub(crate) fn stage(&self, conversion_id: &str, stage: Stage) -> Result<(), SessionError> {
        let now = now_ms();
        self.conn()
            .execute(
                "UPDATE conversions SET status = ?2, updated_ms = ?3, \
                 stage_ms = json_set(stage_ms, '$.' || ?2, ?3) WHERE conversion_id = ?1",
                params![conversion_id, stage.as_str(), now],
            )
            .map_err(sql)?;
        Ok(())
    }

    /// The asset's content version, read now.
    pub(crate) fn read_version(
        &self,
        conversion_id: &str,
        asset_id: &str,
        blake3: &str,
    ) -> Result<(), SessionError> {
        self.conn()
            .execute(
                "UPDATE conversions SET asset_blake3 = ?2 WHERE conversion_id = ?1",
                params![conversion_id, blake3],
            )
            .map_err(sql)?;
        self.conn()
            .execute(
                "UPDATE assets SET blake3 = ?2 WHERE asset_id = ?1",
                params![asset_id, blake3],
            )
            .map_err(sql)?;
        Ok(())
    }

    /// A failed try: queued again until [`ATTEMPTS`], then `failed`.
    pub(crate) fn fail(&self, job: &Job, error: &str) -> Result<(), SessionError> {
        let attempts = job.attempts + 1;
        let stage = if attempts >= ATTEMPTS {
            Stage::Failed
        } else {
            Stage::Queued
        };
        self.conn()
            .execute(
                "UPDATE conversions SET attempts = ?2, error = ?3 WHERE conversion_id = ?1",
                params![job.conversion_id, attempts, error],
            )
            .map_err(sql)?;
        self.stage(&job.conversion_id, stage)
    }

    /// Its chunks are committed: this conversion is now the asset's current one.
    pub(crate) fn finish(&self, job: &Job, chunks: u64) -> Result<(), SessionError> {
        self.conn()
            .execute(
                "UPDATE conversions SET current = (conversion_id = ?2), \
                 chunks = CASE WHEN conversion_id = ?2 THEN ?3 ELSE chunks END, \
                 error = CASE WHEN conversion_id = ?2 THEN NULL ELSE error END \
                 WHERE asset_id = ?1",
                params![
                    job.asset_id,
                    job.conversion_id,
                    i64::try_from(chunks).unwrap_or(0)
                ],
            )
            .map_err(sql)?;
        self.stage(&job.conversion_id, Stage::Ready)
    }

    /// The current conversion of each present asset.
    pub(crate) fn current(&self) -> Result<HashMap<String, String>, SessionError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT c.asset_id, c.conversion_id FROM conversions c JOIN assets a USING (asset_id) \
                 WHERE c.current = 1 AND a.state = 'present'",
            )
            .map_err(sql)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    /// Chunks the owner turned off.
    pub(crate) fn disabled(&self) -> Result<HashSet<String>, SessionError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT chunk_id FROM chunk_overrides WHERE enabled = 0")
            .map_err(sql)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    pub(crate) fn set_enabled(&self, chunk_id: &str, enabled: bool) -> Result<(), SessionError> {
        self.conn()
            .execute(
                "INSERT INTO chunk_overrides(chunk_id, enabled, updated_ms) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(chunk_id) DO UPDATE SET enabled = ?2, updated_ms = ?3",
                params![chunk_id, enabled, now_ms()],
            )
            .map_err(sql)?;
        Ok(())
    }

    /// Every present original with its latest conversion, by path.
    pub(crate) fn states(&self) -> Result<Vec<AssetState>, SessionError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT a.asset_id, a.path, a.name, a.size, \
                 coalesce(c.status, 'queued'), \
                 coalesce((SELECT chunks FROM conversions WHERE asset_id = a.asset_id AND current = 1), 0), \
                 c.error, coalesce(c.updated_ms, a.last_seen_ms) \
                 FROM assets a LEFT JOIN conversions c ON c.asset_id = a.asset_id AND c.created_ms = \
                 (SELECT max(created_ms) FROM conversions WHERE asset_id = a.asset_id) \
                 WHERE a.state = 'present' ORDER BY a.path",
            )
            .map_err(sql)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AssetState {
                    asset_id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    size: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                    stage: Stage::parse(&r.get::<_, String>(4)?),
                    chunks: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
                    error: r.get(6)?,
                    updated_ms: r.get(7)?,
                })
            })
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    /// The asset at `path`, if present.
    pub(crate) fn asset_at(&self, path: &str) -> Result<Option<String>, SessionError> {
        self.conn()
            .query_row(
                "SELECT asset_id FROM assets WHERE path = ?1 AND state = 'present'",
                [path],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)
    }
}

fn queue(
    conn: &Connection,
    asset_id: &str,
    converter: &str,
    version: &str,
    now: i64,
) -> Result<(), SessionError> {
    let id = blake3::hash(format!("{asset_id}\0{now}\0{}", uuid_like()).as_bytes()).to_hex()[..32]
        .to_string();
    conn.execute(
        "INSERT INTO conversions(conversion_id, asset_id, target, converter, converter_version, \
         status, created_ms, updated_ms) VALUES (?1, ?2, 'knowledge', ?3, ?4, 'queued', ?5, ?5)",
        params![id, asset_id, converter, version, now],
    )
    .map_err(sql)?;
    Ok(())
}

/// Enough to tell apart two conversions queued in the same millisecond.
fn uuid_like() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A file's stable identity in its collection: it keeps it across edits.
pub(crate) fn asset_id(space: &str, path: &str) -> String {
    blake3::hash(format!("{space}\0file\0{path}").as_bytes()).to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(name: &str) -> Option<&'static str> {
        name.ends_with(".md").then_some("markdown")
    }

    fn files(list: &[(&str, u64, i64)]) -> Vec<(String, String, u64, i64)> {
        list.iter()
            .map(|(p, s, m)| {
                (
                    p.to_string(),
                    p.rsplit('/').next().unwrap().to_string(),
                    *s,
                    *m,
                )
            })
            .collect()
    }

    #[test]
    fn new_changed_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::open(dir.path()).unwrap();
        let gone = cat
            .reconcile(
                "docs",
                &files(&[("a.md", 10, 1), ("b.md", 5, 1), ("c.png", 9, 1)]),
                conv,
                "v1",
            )
            .unwrap();
        assert!(gone.is_empty());
        let states = cat.states().unwrap();
        assert_eq!(states.len(), 2, "the PNG is not knowledge");
        assert!(states.iter().all(|s| s.stage == Stage::Queued));

        // Both are converted.
        while let Some(job) = cat.next_job().unwrap() {
            cat.stage(&job.conversion_id, Stage::Reading).unwrap();
            cat.finish(&job, 3).unwrap();
        }
        assert!(cat
            .states()
            .unwrap()
            .iter()
            .all(|s| s.stage == Stage::Ready && s.chunks == 3));
        assert_eq!(cat.current().unwrap().len(), 2);

        // Nothing changed: nothing queued.
        cat.reconcile(
            "docs",
            &files(&[("a.md", 10, 1), ("b.md", 5, 1)]),
            conv,
            "v1",
        )
        .unwrap();
        assert!(cat.next_job().unwrap().is_none());

        // a.md changed, b.md went.
        let gone = cat
            .reconcile("docs", &files(&[("a.md", 11, 2)]), conv, "v1")
            .unwrap();
        assert_eq!(gone, vec![asset_id("docs", "b.md")]);
        let job = cat.next_job().unwrap().unwrap();
        assert_eq!(job.path, "a.md");
        let states = cat.states().unwrap();
        assert_eq!(states.len(), 1);
        // The old chunks keep serving until the new conversion is ready.
        assert_eq!(states[0].chunks, 3);
        assert_eq!(cat.current().unwrap().len(), 1);
    }

    #[test]
    fn failures_are_retried_then_kept() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::open(dir.path()).unwrap();
        cat.reconcile("docs", &files(&[("a.md", 10, 1)]), conv, "v1")
            .unwrap();
        for _ in 0..ATTEMPTS {
            let job = cat.next_job().unwrap().unwrap();
            cat.fail(&job, "not UTF-8 text").unwrap();
        }
        assert!(cat.next_job().unwrap().is_none());
        let state = &cat.states().unwrap()[0];
        assert_eq!(state.stage, Stage::Failed);
        assert_eq!(state.error.as_deref(), Some("not UTF-8 text"));
        assert_eq!(cat.retry_failed("v1").unwrap(), 1);
        assert!(cat.next_job().unwrap().is_some());
    }

    #[test]
    fn a_new_converter_version_converts_again() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::open(dir.path()).unwrap();
        cat.reconcile("docs", &files(&[("a.md", 10, 1)]), conv, "v1")
            .unwrap();
        let job = cat.next_job().unwrap().unwrap();
        cat.finish(&job, 1).unwrap();
        cat.reconcile("docs", &files(&[("a.md", 10, 1)]), conv, "v2")
            .unwrap();
        assert!(cat.next_job().unwrap().is_some());
    }

    #[test]
    fn chunks_can_be_turned_off_and_on() {
        let dir = tempfile::tempdir().unwrap();
        let cat = Catalog::open(dir.path()).unwrap();
        cat.set_enabled("c1", false).unwrap();
        assert!(cat.disabled().unwrap().contains("c1"));
        cat.set_enabled("c1", true).unwrap();
        assert!(cat.disabled().unwrap().is_empty());
    }
}
