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

//! A consistent copy of a data folder, and its manifest. Tables' data
//! files are linked rather than copied where the disk allows; SQLite files
//! are copied through `VACUUM INTO`, as they are written in place.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{io, BackupError, Meter};

/// The manifest's name at the top of a staged or restored copy.
pub const MANIFEST_FILE: &str = "oikome-backup.json";

/// Folders of the data folder a backup leaves out: what is rebuilt
/// (indexes, caches, scratch), what is downloaded again (models, DuckDB's
/// extensions), the login link, and backups' own working folder.
const LEFT_OUT_FOLDERS: [&str; 9] = [
    ".oikome/index",
    ".oikome/cache",
    ".oikome/tmp",
    ".oikome/models",
    ".oikome/extensions",
    ".oikome/web",
    ".oikome/backup",
    ".duckdb",
    "tmp",
];

/// Files left out: the identity's seed (it is the recovery key), stored
/// credentials when there is no keychain, and what only a running node has.
const LEFT_OUT_FILES: [&str; 4] = [
    ".oikome/identity/node_ed25519",
    ".oikome/identity/storage_access.json",
    ".oikome/registry.lock",
    ".oikome/peers.sock",
];

/// What was in a staged copy, to check a restore against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// When the copy was made, in milliseconds since the Unix epoch.
    pub made_at_ms: u64,
    /// The data folder it is a copy of, as the catalog names it (the tables
    /// keep paths under it): a restore elsewhere moves them (see
    /// [`crate::relocate`]).
    #[serde(default)]
    pub data_root: String,
    /// Every file, by its path within the data folder (`/`-separated).
    pub files: Vec<ManifestFile>,
    /// Every table of every SQLite file, with its rows.
    pub sqlite: Vec<SqliteTable>,
    /// What the caller adds: the originals the collections view where they
    /// are, which a backup does not hold.
    #[serde(default)]
    pub originals: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqliteTable {
    pub file: String,
    pub table: String,
    pub rows: u64,
}

fn left_out(relative: &str) -> bool {
    LEFT_OUT_FILES.contains(&relative)
        || LEFT_OUT_FOLDERS
            .iter()
            .any(|f| relative == *f || relative.starts_with(&format!("{f}/")))
        || ["-wal", "-shm", "-journal"]
            .iter()
            .any(|end| relative.ends_with(end))
}

/// Copy order: the registries first, then every table's metadata, then the
/// rest. Data files are never changed once written, so every file the
/// copied registries and metadata name is copied after them.
fn phase(relative: &str) -> u8 {
    let top_registry = relative
        .strip_prefix(".oikome/")
        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".json"));
    if top_registry {
        0
    } else if relative
        .split('/')
        .any(|part| part == "metadata" || part == "_versions" || part == "_transactions")
    {
        1
    } else {
        2
    }
}

/// Every file of `data` a backup keeps, as (relative path, full path); links
/// and anything that is not a plain file or folder are not followed.
fn files_of(data: &Path) -> Result<Vec<(String, PathBuf)>, BackupError> {
    let mut out = Vec::new();
    let mut folders = vec![(data.to_path_buf(), String::new())];
    while let Some((folder, relative)) = folders.pop() {
        for entry in std::fs::read_dir(&folder).map_err(io)? {
            let entry = entry.map_err(io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if left_out(&path) {
                continue;
            }
            let kind = entry.file_type().map_err(io)?;
            if kind.is_dir() {
                folders.push((entry.path(), path));
            } else if kind.is_file() {
                out.push((path, entry.path()));
            }
        }
    }
    out.sort_by(|a, b| (phase(&a.0), &a.0).cmp(&(phase(&b.0), &b.0)));
    Ok(out)
}

/// A table's data file, which its format never changes once written.
fn never_rewritten(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "parquet" | "lance"))
}

fn is_sqlite(path: &Path) -> bool {
    let mut header = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok()
        && &header == b"SQLite format 3\0"
}

fn hash_of(path: &Path) -> Result<String, BackupError> {
    let mut hasher = blake3::Hasher::new();
    hasher
        .update_reader(std::fs::File::open(path).map_err(io)?)
        .map_err(io)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Every table of the SQLite file at `path`, with its rows.
fn tables_of(path: &Path) -> Result<Vec<(String, u64)>, BackupError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(io)?;
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .map_err(io)?
        .query_map([], |r| r.get(0))
        .map_err(io)?
        .collect::<Result<_, _>>()
        .map_err(io)?;
    let mut out = Vec::new();
    for name in names {
        // A name from the file's own schema, quoted as an identifier.
        let quoted = format!("\"{}\"", name.replace('"', "\"\""));
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {quoted}"), [], |r| r.get(0))
            .map_err(io)?;
        out.push((name, u64::try_from(rows).unwrap_or(0)));
    }
    Ok(out)
}

/// Copy `data` to the empty folder `staging` as a backup keeps it, and
/// write the manifest there. `originals` goes into the manifest as given.
pub fn stage(
    data: &Path,
    staging: &Path,
    originals: serde_json::Value,
    meter: Option<&Meter>,
) -> Result<Manifest, BackupError> {
    std::fs::create_dir_all(staging).map_err(io)?;
    let mut manifest = Manifest {
        made_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        // As the catalog names it, which is how the tables wrote it.
        data_root: data.to_string_lossy().into_owned(),
        originals,
        ..Manifest::default()
    };
    let files = files_of(data)?;
    if let Some(meter) = meter {
        meter.start(
            files
                .iter()
                .filter_map(|(_, from)| std::fs::metadata(from).ok())
                .map(|m| m.len())
                .sum(),
        );
    }
    for (relative, from) in files {
        let to = staging.join(&relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        if is_sqlite(&from) {
            // A consistent copy while the node may be writing it.
            let conn =
                Connection::open_with_flags(&from, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(io)?;
            let target = to.to_str().ok_or_else(|| io("a path that is not UTF-8"))?;
            conn.execute("VACUUM INTO ?1", [target]).map_err(io)?;
            for (table, rows) in tables_of(&to)? {
                manifest.sqlite.push(SqliteTable {
                    file: relative.clone(),
                    table,
                    rows,
                });
            }
        } else {
            // A table's data file (Parquet, Lance) is never changed once
            // written, so a link keeps it as it is now and takes no room;
            // anything else may be rewritten in place and is copied.
            let linked = if never_rewritten(&relative) {
                std::fs::hard_link(&from, &to).or_else(|_| std::fs::copy(&from, &to).map(|_| ()))
            } else {
                std::fs::copy(&from, &to).map(|_| ())
            };
            match linked {
                Ok(()) => {}
                // Removed while the copy was made: not in this backup.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io(format!("{relative}: {e}"))),
            }
        }
        let size = std::fs::metadata(&to).map_err(io)?.len();
        if let Some(meter) = meter {
            meter.add(size);
        }
        manifest.files.push(ManifestFile {
            blake3: hash_of(&to)?,
            path: relative,
            size,
        });
    }
    std::fs::write(
        staging.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).map_err(io)?,
    )
    .map_err(io)?;
    Ok(manifest)
}

/// Check a staged or restored copy against its manifest: every file there
/// with its hash, every SQLite file sound and its tables' rows.
pub fn verify(copy: &Path) -> Result<Manifest, BackupError> {
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(copy.join(MANIFEST_FILE))
            .map_err(|e| BackupError::Damaged(format!("no manifest: {e}")))?,
    )
    .map_err(|e| BackupError::Damaged(format!("the manifest cannot be read: {e}")))?;
    for file in &manifest.files {
        let path = copy.join(&file.path);
        let hash =
            hash_of(&path).map_err(|e| BackupError::Damaged(format!("{}: {e}", file.path)))?;
        if hash != file.blake3 {
            return Err(BackupError::Damaged(format!("{} has changed", file.path)));
        }
    }
    let mut expected: BTreeMap<&str, Vec<(&str, u64)>> = BTreeMap::new();
    for t in &manifest.sqlite {
        expected
            .entry(&t.file)
            .or_default()
            .push((&t.table, t.rows));
    }
    for (file, tables) in expected {
        let path = copy.join(file);
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| BackupError::Damaged(format!("{file}: {e}")))?;
        let check: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| BackupError::Damaged(format!("{file}: {e}")))?;
        if check != "ok" {
            return Err(BackupError::Damaged(format!("{file}: {check}")));
        }
        let found: BTreeMap<String, u64> = tables_of(&path)?.into_iter().collect();
        for (table, rows) in tables {
            if found.get(table) != Some(&rows) {
                return Err(BackupError::Damaged(format!(
                    "{file}: {table} does not have its {rows} rows"
                )));
            }
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn a_copy_keeps_the_owners_data_and_leaves_out_secrets_and_caches() {
        let data = tempfile::tempdir().unwrap();
        let root = data.path();
        write(root, ".oikome/spaces.json", "{}");
        write(root, ".oikome/identity/node_ed25519", "seed");
        write(
            root,
            ".oikome/identity/storage_access.json",
            "{\"password\":\"x\"}",
        );
        write(root, ".oikome/identity/remote_tokens.json", "[]");
        write(root, ".oikome/index/abc/index.sqlite", "cache");
        write(root, ".oikome/web/login", "token");
        write(root, "photos/trips/metadata/v1.metadata.json", "{}");
        write(root, "photos/trips/data/a.parquet", "rows");
        let db = root.join(".oikome/sqlite/notes.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE memos (id INTEGER, body TEXT); INSERT INTO memos VALUES (1, 'a'), (2, 'b');")
            .unwrap();

        let staging = tempfile::tempdir().unwrap();
        let manifest = stage(root, staging.path(), serde_json::Value::Null, None).unwrap();
        let kept: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            kept,
            [
                ".oikome/spaces.json",
                "photos/trips/metadata/v1.metadata.json",
                ".oikome/identity/remote_tokens.json",
                ".oikome/sqlite/notes.sqlite",
                "photos/trips/data/a.parquet",
            ],
            "registries first, then metadata, then the rest; no seed, credentials, caches or login"
        );
        assert_eq!(
            manifest.sqlite,
            [SqliteTable {
                file: ".oikome/sqlite/notes.sqlite".into(),
                table: "memos".into(),
                rows: 2
            }]
        );
        drop(conn);
        assert_eq!(verify(staging.path()).unwrap(), manifest);

        // A changed file is found.
        std::fs::write(
            staging.path().join(".oikome/spaces.json"),
            "{\"changed\":1}",
        )
        .unwrap();
        assert!(matches!(
            verify(staging.path()),
            Err(BackupError::Damaged(_))
        ));
    }
}
