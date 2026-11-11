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

//! Folders viewed in place (architecture v3 §7.7, roadmap 3.9).
//!
//! A Space bound to an [`LocationFormat::InPlace`] Location views a folder
//! the owner already has, on this computer (or a mounted share, or a cloud
//! drive's sync folder) or on a NAS over WebDAV. Oikome never writes, renames
//! or deletes there. What it keeps is an index, a cache under
//! `.oikome/index/<location>/` that can be deleted and is rebuilt:
//!
//! - `index.sqlite`: a row per file (path, kind, size, time, and what was
//!   read from it: a photo's caption, a note's date and text), a thumbnail
//!   per photo, and a full-text index (trigram, as for sources, §8);
//! - `files-<n>.parquet`: the rows as the Space's `files` table, which the
//!   node's DuckDB reads.
//!
//! Each CSV, Parquet or JSON file at the top of the folder is a table of the
//! Space, read where it is each time it is read (a NAS's are first fetched
//! into a size-bounded cache under `.oikome/cache/places/`). While the place
//! can't be reached, `files`, search and thumbnails still answer from the
//! index; reading a table or an original says the place is offline.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use oikome_catalog::{Location, LocationFormat, OikomeCatalog, StorageKind};
use oikome_connect::folder::{captions_of, split_front_matter, wall_time, written_time, CAPTIONS};
use oikome_engine::{FileFormat, TableSource};
use oikome_vfs::{check_owner_path, Bucket, Credentials, DropboxClient, GoogleClient, RemoteStore};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::SessionError;
use crate::registry::EngineRegistry;
use crate::search::{fts_query, like_pattern, SearchMatch, MIN_INDEXED_CHARS};
use crate::secrets::SecretKind;

/// The Space's table of its files.
pub const FILES: &str = "files";
pub const IMAGE: &str = "image";
pub const NOTE: &str = "note";
pub const TABLE: &str = "table";
pub const OTHER: &str = "other";

const INDEX_FILE: &str = "index.sqlite";
/// At most this many files are indexed per place, this many folders deep.
pub const MAX_FILES: usize = 200_000;
const MAX_DEPTH: usize = 32;
/// Larger photos and notes are listed, not read.
pub const MAX_IMAGE_BYTES: u64 = 64 << 20;
pub const MAX_NOTE_BYTES: u64 = 1 << 20;
/// A thumbnail's longest edge, in pixels.
const THUMB_EDGE: u32 = 512;
/// The cache of a NAS's table files, per place.
const CACHE_BYTES: u64 = 2 << 30;
/// How long a place has to answer whether it can be reached.
const REACH_FOR: Duration = Duration::from_secs(5);

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];
const NOTE_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];
/// Tables written as text: their first lines are kept, to show some rows.
const TEXT_TABLE_EXTENSIONS: &[&str] = &["csv", "tsv"];
/// The bytes of a text table read for its first lines, and how many are kept.
const HEAD_BYTES: u64 = 2048;
const HEAD_LINES: usize = 4;

/// A file's state in the index: not read yet, read, or listed only.
const UNREAD: i64 = 0;
const READ: i64 = 1;
const LISTED_ONLY: i64 = 2;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS files(
  path TEXT PRIMARY KEY,
  folder TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  size INTEGER NOT NULL,
  modified_ms INTEGER NOT NULL,
  modified TEXT NOT NULL,
  title TEXT, taken TEXT, written TEXT, place TEXT, credit TEXT, text TEXT,
  state INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_folder ON files(folder);
CREATE TABLE IF NOT EXISTS thumbs(
  path TEXT PRIMARY KEY, jpeg BLOB NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS facts(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
  name, title, text, content='files', tokenize='trigram remove_diacritics 1'
);
CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
  INSERT INTO files_fts(rowid, name, title, text) VALUES (new.rowid, new.name, new.title, new.text);
END;
CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
  INSERT INTO files_fts(files_fts, rowid, name, title, text)
  VALUES ('delete', old.rowid, old.name, old.title, old.text);
END;
CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
  INSERT INTO files_fts(files_fts, rowid, name, title, text)
  VALUES ('delete', old.rowid, old.name, old.title, old.text);
  INSERT INTO files_fts(rowid, name, title, text) VALUES (new.rowid, new.name, new.title, new.text);
END;
";

/// The columns of the `files` table, in order.
pub fn files_schema() -> SchemaRef {
    let text = |name: &str| Field::new(name, DataType::Utf8, true);
    Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("folder", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("size", DataType::Int64, false),
        Field::new("modified", DataType::Utf8, false),
        text("title"),
        text("taken"),
        text("written"),
        text("place"),
        text("credit"),
        text("text"),
    ]))
}

/// The kind of a file, by its extension.
pub fn kind_of(name: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        IMAGE
    } else if NOTE_EXTENSIONS.contains(&extension.as_str()) {
        NOTE
    } else if FileFormat::of_extension(&extension).is_some() {
        TABLE
    } else {
        OTHER
    }
}

fn io(e: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("viewing in place: {e}"))
}

fn sql(e: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("the index of a folder viewed in place: {e}"))
}

/// Whether `path` is a relative path within a folder viewed in place.
pub fn is_path_within(path: &str) -> bool {
    check_owner_path(path).is_ok()
}

/// The error for a place that can't be reached now.
pub fn offline(label: &str) -> SessionError {
    SessionError::Unsupported(format!(
        "{label} can't be reached right now; names, text and thumbnails are still shown, \
         and its files open once it is back"
    ))
}

/// Where a folder viewed in place is.
#[derive(Clone, Debug)]
pub enum Place {
    /// A folder on this computer: a local disk, a mounted share or a cloud
    /// drive's sync folder.
    Local(PathBuf),
    /// A folder on a NAS, over WebDAV, or on object storage (boxed: the
    /// store is far larger than a path).
    Remote {
        store: Box<RemoteStore>,
        label: String,
    },
}

impl Place {
    /// What the owner calls it, for messages.
    pub fn label(&self) -> String {
        match self {
            Place::Local(path) => path.display().to_string(),
            Place::Remote { label, .. } => label.clone(),
        }
    }

    /// Whether it can be reached now (briefly asked).
    pub async fn reachable(&self) -> bool {
        match self {
            Place::Local(path) => path.is_dir(),
            Place::Remote { store, .. } => {
                matches!(
                    tokio::time::timeout(REACH_FOR, store.list_owned("")).await,
                    Ok(Ok(_))
                )
            }
        }
    }

    /// `range` of the file at `path` (relative, `/`-separated), at most `max`
    /// bytes.
    pub async fn read(
        &self,
        path: &str,
        range: std::ops::Range<u64>,
        max: u64,
    ) -> Result<Vec<u8>, SessionError> {
        check_owner_path(path).map_err(io)?;
        match self {
            Place::Local(root) => {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                let full = root.join(path);
                let meta = tokio::fs::symlink_metadata(&full).await.map_err(io)?;
                if !meta.is_file() {
                    return Err(io(format!("{path} is not a file")));
                }
                let end = range.end.min(meta.len());
                if range.start > end {
                    return Ok(Vec::new());
                }
                if end - range.start > max {
                    return Err(io(format!("{path}: more than {max} bytes asked for")));
                }
                let mut file = tokio::fs::File::open(&full).await.map_err(io)?;
                file.seek(std::io::SeekFrom::Start(range.start))
                    .await
                    .map_err(io)?;
                let mut out = vec![0; (end - range.start) as usize];
                file.read_exact(&mut out).await.map_err(io)?;
                Ok(out)
            }
            Place::Remote { store, .. } => Ok(store
                .read_owned(path, range, max)
                .await
                .map_err(io)?
                .to_vec()),
        }
    }
}

/// The place a Location views, with the storage's credentials for a NAS.
pub fn place_of(
    catalog: &OikomeCatalog,
    engines: &EngineRegistry,
    location: &Location,
) -> Result<Place, SessionError> {
    let LocationFormat::InPlace { place, path } = &location.format else {
        return Err(SessionError::Unsupported(format!(
            "location '{}' is not a folder viewed in place",
            location.id
        )));
    };
    let Some(storage) = place else {
        return Ok(Place::Local(PathBuf::from(path)));
    };
    let def = catalog.get_storage(storage)?;
    let within = path.trim_matches('/');
    if !within.is_empty() {
        check_owner_path(within).map_err(io)?;
    }
    match &def.kind {
        StorageKind::Folder { path: root } | StorageKind::Drive { path: root } => {
            Ok(Place::Local(Path::new(root).join(within)))
        }
        // Without the drive's read access Oikome sees only the files it made.
        StorageKind::Gdrive {
            reads_all: false, ..
        } => Err(SessionError::Unsupported(
            "Oikome may read only the files it made on this Google Drive; connect it again \
             letting it view your other files to view a folder there"
                .to_string(),
        )),
        StorageKind::Gdrive {
            client_id,
            reads_all: true,
        } => {
            let store_err = |message: String| SessionError::Store {
                storage: def.name.clone(),
                message,
            };
            let secret = |kind| engines.storage_secret(&def.name, kind);
            let client_secret = secret(SecretKind::Password)?
                .ok_or_else(|| store_err("this node has no secret of its Google app".into()))?;
            let token = secret(SecretKind::Token)?
                .ok_or_else(|| store_err("this node isn't signed in to its Google Drive".into()))?;
            let client = GoogleClient::new(client_id.as_str(), client_secret.as_str());
            let store = RemoteStore::gdrive_view(within, &client, &token)
                .map_err(|e| store_err(e.to_string()))?;
            let label = if within.is_empty() {
                def.name.clone()
            } else {
                format!("{}/{within}", def.name)
            };
            Ok(Place::Remote {
                store: Box::new(store),
                label,
            })
        }
        StorageKind::Dropbox {
            reads_all: false, ..
        } => Err(SessionError::Unsupported(
            "Oikome may read only its app folder on Dropbox; use a Full Dropbox app to view \
             your other folders there"
                .to_string(),
        )),
        StorageKind::Dropbox {
            client_id,
            reads_all: true,
        } => {
            let store_err = |message: String| SessionError::Store {
                storage: def.name.clone(),
                message,
            };
            let secret = |kind| engines.storage_secret(&def.name, kind);
            let token = secret(SecretKind::Token)?
                .ok_or_else(|| store_err("this node isn't signed in to Dropbox".into()))?;
            let client = DropboxClient::new(client_id.as_str());
            let store = RemoteStore::dropbox_view(within, &client, &token)
                .map_err(|e| store_err(e.to_string()))?;
            let label = if within.is_empty() {
                def.name.clone()
            } else {
                format!("{}/{within}", def.name)
            };
            Ok(Place::Remote {
                store: Box::new(store),
                label,
            })
        }
        StorageKind::Webdav { endpoint, username } => {
            if def.kind.needs_allow_http() && !engines.allow_http() {
                return Err(SessionError::Store {
                    storage: def.name.clone(),
                    message: "it is reached over plaintext http://, which sends its password \
                              unencrypted; use https:// or allow it with --allow-http"
                        .to_string(),
                });
            }
            let secret = |kind| engines.storage_secret(&def.name, kind);
            let credentials = Credentials {
                username: username.clone(),
                password: secret(SecretKind::Password)?,
                token: secret(SecretKind::Token)?,
            };
            let store = RemoteStore::webdav_view(endpoint, within, credentials).map_err(|e| {
                SessionError::Store {
                    storage: def.name.clone(),
                    message: e.to_string(),
                }
            })?;
            let label = if within.is_empty() {
                def.name.clone()
            } else {
                format!("{}/{within}", def.name)
            };
            Ok(Place::Remote {
                store: Box::new(store),
                label,
            })
        }
        StorageKind::S3 {
            endpoint,
            bucket,
            access_key_id,
        } => {
            if def.kind.needs_allow_http() && !engines.allow_http() {
                return Err(SessionError::Store {
                    storage: def.name.clone(),
                    message: "it is reached over plaintext http://, which sends the files \
                              unencrypted; use https:// or allow it with --allow-http"
                        .to_string(),
                });
            }
            let store_err = |message: String| SessionError::Store {
                storage: def.name.clone(),
                message,
            };
            let secret = engines
                .storage_secret(&def.name, SecretKind::Password)?
                .ok_or_else(|| store_err("this node has no secret key for it".to_string()))?;
            let bucket = Bucket {
                endpoint,
                name: bucket,
            };
            let store = RemoteStore::s3_view(bucket, within, access_key_id, &secret)
                .map_err(|e| store_err(e.to_string()))?;
            let label = if within.is_empty() {
                def.name.clone()
            } else {
                format!("{}/{within}", def.name)
            };
            Ok(Place::Remote {
                store: Box::new(store),
                label,
            })
        }
    }
}

/// A file as listed: its size and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Listed {
    size: u64,
    modified_ms: i64,
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Every file under `root` worth listing: no hidden files or folders, no
/// links, at most [`MAX_FILES`], [`MAX_DEPTH`] folders deep.
fn walk_local(root: &Path) -> Result<BTreeMap<String, Listed>, SessionError> {
    let mut out = BTreeMap::new();
    let mut folders = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, rel, depth)) = folders.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // A subfolder that can't be read is left out; the place itself
            // was checked by the caller.
            Err(e) if depth > 0 => {
                tracing::debug!(folder = %dir.display(), error = %e, "skipped a folder");
                continue;
            }
            Err(e) => return Err(io(e)),
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') || name.chars().any(char::is_control) {
                continue;
            }
            // Not followed: a link is not the folder's own.
            let Ok(meta) = entry.metadata() else { continue };
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if meta.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    folders.push((entry.path(), path, depth + 1));
                }
            } else if meta.is_file() {
                if out.len() >= MAX_FILES {
                    return Ok(out);
                }
                out.insert(
                    path,
                    Listed {
                        size: meta.len(),
                        modified_ms: meta.modified().map(millis).unwrap_or(0),
                    },
                );
            }
        }
    }
    Ok(out)
}

/// Every file under a NAS folder worth listing, as [`walk_local`] does.
async fn walk_remote(store: &RemoteStore) -> Result<BTreeMap<String, Listed>, SessionError> {
    let mut out = BTreeMap::new();
    let mut folders = vec![(String::new(), 0usize)];
    while let Some((rel, depth)) = folders.pop() {
        let entries = match store.list_owned(&rel).await {
            Ok(entries) => entries,
            Err(e) if depth > 0 => {
                tracing::debug!(folder = %rel, error = %e, "skipped a folder");
                continue;
            }
            Err(e) => return Err(io(e)),
        };
        for entry in entries {
            if entry.name.starts_with('.') || check_owner_path(&entry.name).is_err() {
                continue;
            }
            let path = if rel.is_empty() {
                entry.name.clone()
            } else {
                format!("{rel}/{}", entry.name)
            };
            if entry.is_dir {
                if depth + 1 < MAX_DEPTH {
                    folders.push((path, depth + 1));
                }
            } else {
                if out.len() >= MAX_FILES {
                    return Ok(out);
                }
                out.insert(
                    path,
                    Listed {
                        size: entry.size,
                        modified_ms: entry.modified_ms,
                    },
                );
            }
        }
    }
    Ok(out)
}

async fn walk(place: &Place) -> Result<BTreeMap<String, Listed>, SessionError> {
    match place {
        Place::Local(root) => {
            let root = root.clone();
            tokio::task::spawn_blocking(move || walk_local(&root))
                .await
                .map_err(io)?
        }
        Place::Remote { store, .. } => walk_remote(store).await,
    }
}

fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned())
}

fn time_text(ms: i64) -> String {
    wall_time(UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0)))
}

/// Open (and if need be create) the index in `dir`.
fn open_index(dir: &Path) -> Result<Connection, SessionError> {
    std::fs::create_dir_all(dir).map_err(io)?;
    let conn = Connection::open(dir.join(INDEX_FILE)).map_err(sql)?;
    conn.busy_timeout(Duration::from_secs(10)).map_err(sql)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sql)?;
    conn.execute_batch(SCHEMA_SQL).map_err(sql)?;
    Ok(conn)
}

fn fact(conn: &Connection, key: &str) -> Result<Option<String>, SessionError> {
    conn.query_row("SELECT value FROM facts WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .map_err(sql)
}

fn set_fact(conn: &Connection, key: &str, value: &str) -> Result<(), SessionError> {
    conn.execute(
        "INSERT INTO facts(key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(sql)?;
    Ok(())
}

/// Bring the index in line with a listing: new files added, changed ones to
/// be read again, removed ones gone. Returns whether anything changed.
fn apply_listing(
    conn: &mut Connection,
    listed: &BTreeMap<String, Listed>,
) -> Result<bool, SessionError> {
    let tx = conn.transaction().map_err(sql)?;
    let known: HashMap<String, Listed> = {
        let mut stmt = tx
            .prepare("SELECT path, size, modified_ms FROM files")
            .map_err(sql)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Listed {
                        size: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                        modified_ms: r.get::<_, i64>(2)?,
                    },
                ))
            })
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)?
    };
    let mut changed = false;
    // Folders whose captions changed: their photos are read again.
    let mut recaptioned: BTreeSet<String> = BTreeSet::new();
    for path in known.keys() {
        if !listed.contains_key(path) {
            tx.execute("DELETE FROM files WHERE path = ?1", [path])
                .map_err(sql)?;
            tx.execute("DELETE FROM thumbs WHERE path = ?1", [path])
                .map_err(sql)?;
            if name_of(path) == CAPTIONS {
                recaptioned.insert(folder_of(path).to_string());
            }
            changed = true;
        }
    }
    for (path, file) in listed {
        let kind = kind_of(name_of(path));
        // Other files have nothing to read beyond their listing; a text
        // table, its first lines.
        let state = if matches!(kind, IMAGE | NOTE) || is_text_table(path) {
            UNREAD
        } else {
            READ
        };
        match known.get(path) {
            Some(was) if was == file => continue,
            Some(_) => {
                tx.execute(
                    "UPDATE files SET size = ?2, modified_ms = ?3, modified = ?4, title = NULL, \
                     taken = NULL, written = NULL, place = NULL, credit = NULL, text = NULL, \
                     state = ?5 WHERE path = ?1",
                    params![
                        path,
                        i64::try_from(file.size).unwrap_or(i64::MAX),
                        file.modified_ms,
                        time_text(file.modified_ms),
                        state
                    ],
                )
                .map_err(sql)?;
                tx.execute("DELETE FROM thumbs WHERE path = ?1", [path])
                    .map_err(sql)?;
            }
            None => {
                tx.execute(
                    "INSERT INTO files(path, folder, name, kind, size, modified_ms, modified, \
                     title, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        path,
                        folder_of(path),
                        name_of(path),
                        kind,
                        i64::try_from(file.size).unwrap_or(i64::MAX),
                        file.modified_ms,
                        time_text(file.modified_ms),
                        stem_of(name_of(path)),
                        state
                    ],
                )
                .map_err(sql)?;
            }
        }
        if name_of(path) == CAPTIONS {
            recaptioned.insert(folder_of(path).to_string());
        }
        changed = true;
    }
    for folder in &recaptioned {
        tx.execute(
            "UPDATE files SET state = ?2 WHERE folder = ?1 AND kind = 'image'",
            params![folder, UNREAD],
        )
        .map_err(sql)?;
    }
    // Text tables indexed before their first lines were kept: once.
    let unread = tx
        .execute(
            "UPDATE files SET state = ?1 WHERE state = ?2 AND kind = 'table' AND text IS NULL \
             AND (lower(name) LIKE '%.csv' OR lower(name) LIKE '%.tsv')",
            params![UNREAD, READ],
        )
        .map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(changed || unread > 0)
}

/// Whether a file is a table written as text (CSV, TSV), by its name.
fn is_text_table(path: &str) -> bool {
    Path::new(name_of(path))
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| TEXT_TABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The first lines of a text table, from the start of the file: a line cut
/// off by the end of what was read is left out.
fn head_lines(bytes: &[u8], cut: bool) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut lines: Vec<&str> = text.lines().collect();
    if cut && !text.ends_with('\n') {
        lines.pop();
    }
    lines
        .into_iter()
        .take(HEAD_LINES)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A thumbnail of the image in `bytes`: at most [`THUMB_EDGE`] pixels on
/// its longest edge, turned as its camera recorded, as JPEG. `None` for a
/// file that isn't an image Oikome reads, or one too large to decode.
pub fn thumbnail(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    use image::{codecs::jpeg::JpegEncoder, DynamicImage, ImageDecoder, ImageReader, Limits};
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let small = image.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&small)
        .ok()?;
    Some((out, small.width(), small.height()))
}

/// What was read from one file.
#[derive(Default)]
struct Read {
    title: Option<String>,
    taken: Option<String>,
    written: Option<String>,
    place: Option<String>,
    credit: Option<String>,
    text: Option<String>,
    thumb: Option<(Vec<u8>, u32, u32)>,
    state: i64,
}

/// A file to read: path, kind, size and time.
type Unread = (String, String, u64, i64);

/// Read what the index keeps of files not read yet: captions and thumbnails
/// of photos, the date and text of notes. Within `budget` when one is
/// given (the rest is read on a later pass). Returns how many were read.
async fn fill(place: &Place, dir: &Path, budget: Option<Duration>) -> Result<usize, SessionError> {
    let started = Instant::now();
    let mut done = 0;
    let mut captions: HashMap<String, HashMap<String, [Option<String>; 4]>> = HashMap::new();
    loop {
        let dir_owned = dir.to_path_buf();
        let batch: Vec<Unread> = tokio::task::spawn_blocking(move || {
            let conn = open_index(&dir_owned)?;
            let mut stmt = conn
                .prepare(
                    "SELECT path, kind, size, modified_ms FROM files WHERE state = ?1 \
                     ORDER BY path LIMIT 32",
                )
                .map_err(sql)?;
            let rows = stmt
                .query_map([UNREAD], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map_err(sql)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(sql)
        })
        .await
        .map_err(io)??;
        if batch.is_empty() {
            return Ok(done);
        }
        let mut reads = Vec::with_capacity(batch.len());
        for (path, kind, size, modified_ms) in batch {
            let read = match kind.as_str() {
                IMAGE => {
                    let folder = folder_of(&path).to_string();
                    if !captions.contains_key(&folder) {
                        let file = if folder.is_empty() {
                            CAPTIONS.to_string()
                        } else {
                            format!("{folder}/{CAPTIONS}")
                        };
                        let named = match place.read(&file, 0..u64::MAX, 1 << 20).await {
                            Ok(bytes) => captions_of(&bytes).unwrap_or_default(),
                            Err(_) => HashMap::new(),
                        };
                        captions.insert(folder.clone(), named);
                    }
                    let [title, at, taken, credit] = captions
                        .get(&folder)
                        .and_then(|c| c.get(name_of(&path)))
                        .cloned()
                        .unwrap_or_default();
                    let mut read = Read {
                        title: title.or_else(|| Some(stem_of(name_of(&path)))),
                        taken: taken
                            .as_deref()
                            .and_then(written_time)
                            .or_else(|| Some(time_text(modified_ms))),
                        place: at,
                        credit,
                        state: LISTED_ONLY,
                        ..Read::default()
                    };
                    if size <= MAX_IMAGE_BYTES {
                        if let Ok(bytes) = place.read(&path, 0..u64::MAX, MAX_IMAGE_BYTES).await {
                            read.thumb = tokio::task::spawn_blocking(move || thumbnail(&bytes))
                                .await
                                .map_err(io)?;
                            if read.thumb.is_some() {
                                read.state = READ;
                            }
                        }
                    }
                    read
                }
                NOTE => {
                    let mut read = Read {
                        title: Some(stem_of(name_of(&path))),
                        written: Some(time_text(modified_ms)),
                        state: LISTED_ONLY,
                        ..Read::default()
                    };
                    if size <= MAX_NOTE_BYTES {
                        if let Ok(bytes) = place.read(&path, 0..u64::MAX, MAX_NOTE_BYTES).await {
                            let text = String::from_utf8_lossy(&bytes).into_owned();
                            let (date, body) = split_front_matter(&text);
                            if date.is_some() {
                                read.written = date;
                            }
                            read.text = Some(body.to_string());
                            read.state = READ;
                        }
                    }
                    read
                }
                TABLE if is_text_table(&path) => {
                    let mut read = Read {
                        state: LISTED_ONLY,
                        ..Read::default()
                    };
                    if let Ok(bytes) = place.read(&path, 0..HEAD_BYTES, HEAD_BYTES).await {
                        read.text = Some(head_lines(&bytes, size > HEAD_BYTES));
                        read.state = READ;
                    }
                    read
                }
                _ => Read {
                    state: READ,
                    ..Read::default()
                },
            };
            reads.push((path, read));
        }
        done += reads.len();
        let dir_owned = dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut conn = open_index(&dir_owned)?;
            let tx = conn.transaction().map_err(sql)?;
            for (path, read) in reads {
                tx.execute(
                    "UPDATE files SET title = ?2, taken = ?3, written = ?4, place = ?5, \
                     credit = ?6, text = ?7, state = ?8 WHERE path = ?1",
                    params![
                        path,
                        read.title,
                        read.taken,
                        read.written,
                        read.place,
                        read.credit,
                        read.text,
                        read.state
                    ],
                )
                .map_err(sql)?;
                if let Some((jpeg, width, height)) = read.thumb {
                    tx.execute(
                        "INSERT INTO thumbs(path, jpeg, width, height) VALUES (?1, ?2, ?3, ?4) \
                         ON CONFLICT(path) DO UPDATE SET jpeg = excluded.jpeg, \
                         width = excluded.width, height = excluded.height",
                        params![path, jpeg, width, height],
                    )
                    .map_err(sql)?;
                }
            }
            tx.commit().map_err(sql)
        })
        .await
        .map_err(io)??;
        if budget.is_some_and(|b| started.elapsed() >= b) {
            return Ok(done);
        }
    }
}

/// Write the index's rows as the `files` table (a new generation, so a
/// query reading the last one isn't cut short), and drop the one before.
fn export(conn: &Connection, dir: &Path) -> Result<PathBuf, SessionError> {
    use parquet::arrow::ArrowWriter;
    let generation: u64 = fact(conn, "generation")?
        .and_then(|g| g.parse().ok())
        .unwrap_or(0)
        + 1;
    let path = dir.join(format!("files-{generation}.parquet"));
    let tmp = dir.join(format!(".files-{generation}.parquet.tmp"));
    let schema = files_schema();
    let file = std::fs::File::create(&tmp).map_err(io)?;
    let mut writer = ArrowWriter::try_new(file, schema.clone(), None).map_err(io)?;
    let mut stmt = conn
        .prepare(
            "SELECT path, folder, name, kind, size, modified, title, taken, written, place, \
             credit, text FROM files ORDER BY path",
        )
        .map_err(sql)?;
    let mut rows = stmt.query([]).map_err(sql)?;
    let mut texts: Vec<Vec<Option<String>>> = vec![Vec::new(); 12];
    let mut sizes: Vec<i64> = Vec::new();
    let flush = |texts: &mut Vec<Vec<Option<String>>>,
                 sizes: &mut Vec<i64>,
                 writer: &mut ArrowWriter<std::fs::File>|
     -> Result<(), SessionError> {
        if sizes.is_empty() {
            return Ok(());
        }
        let columns: Vec<ArrayRef> = (0..12)
            .map(|i| -> ArrayRef {
                if i == 4 {
                    Arc::new(Int64Array::from(std::mem::take(sizes)))
                } else {
                    Arc::new(StringArray::from(std::mem::take(&mut texts[i])))
                }
            })
            .collect();
        let batch = RecordBatch::try_new(files_schema(), columns).map_err(io)?;
        writer.write(&batch).map_err(io)
    };
    while let Some(row) = rows.next().map_err(sql)? {
        for (i, column) in texts.iter_mut().enumerate() {
            if i != 4 {
                column.push(row.get::<_, Option<String>>(i).map_err(sql)?);
            }
        }
        sizes.push(row.get::<_, i64>(4).map_err(sql)?);
        if sizes.len() == 8192 {
            flush(&mut texts, &mut sizes, &mut writer)?;
        }
    }
    flush(&mut texts, &mut sizes, &mut writer)?;
    writer.close().map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)?;
    set_fact(conn, "generation", &generation.to_string())?;
    if generation > 2 {
        let _ = std::fs::remove_file(dir.join(format!("files-{}.parquet", generation - 2)));
    }
    Ok(path)
}

/// The current `files` table of an index, if it has been written.
fn exported(conn: &Connection, dir: &Path) -> Result<Option<PathBuf>, SessionError> {
    Ok(fact(conn, "generation")?
        .map(|g| dir.join(format!("files-{g}.parquet")))
        .filter(|p| p.is_file()))
}

/// One scan or refresh at a time per index.
fn lock_for(dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(dir.to_path_buf())
        .or_default()
        .clone()
}

/// What a refresh found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refreshed {
    /// Files in the index.
    pub files: u64,
    /// Whether the listing changed (files added, changed or removed).
    pub changed: bool,
    /// Files read (captions, thumbnails, text) on this pass.
    pub read: usize,
    pub reachable: bool,
}

/// Scan the place again and bring the index up to date: list it, read what
/// is new or changed (within `budget` when one is given), and write the
/// `files` table. A place that can't be reached keeps its index.
pub async fn refresh(
    catalog: &OikomeCatalog,
    engines: &EngineRegistry,
    location: &Location,
    budget: Option<Duration>,
) -> Result<Refreshed, SessionError> {
    let dir = catalog.index_dir(location);
    let lock = lock_for(&dir);
    let _held = lock.lock().await;
    let place = place_of(catalog, engines, location)?;
    let reachable = place.reachable().await;
    remember_reachable(&dir, reachable);
    let mut changed = false;
    let mut read = 0;
    if reachable {
        let listed = walk(&place).await?;
        let dir_owned = dir.clone();
        changed = tokio::task::spawn_blocking(move || {
            let mut conn = open_index(&dir_owned)?;
            apply_listing(&mut conn, &listed)
        })
        .await
        .map_err(io)??;
        read = fill(&place, &dir, budget).await?;
    }
    let dir_owned = dir.clone();
    let files = tokio::task::spawn_blocking(move || {
        let conn = open_index(&dir_owned)?;
        if changed || read > 0 || exported(&conn, &dir_owned)?.is_none() {
            export(&conn, &dir_owned)?;
        }
        set_fact(&conn, "reachable", if reachable { "1" } else { "0" })?;
        if reachable {
            set_fact(
                &conn,
                "scanned_at_ms",
                &millis(SystemTime::now()).to_string(),
            )?;
        }
        conn.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
            .map_err(sql)
    })
    .await
    .map_err(io)??;
    Ok(Refreshed {
        files: u64::try_from(files).unwrap_or(0),
        changed,
        read,
        reachable,
    })
}

/// What a folder viewed in place shows as tables: `files`, and one per
/// data file at its top.
pub struct Bound {
    pub files: PathBuf,
    pub tables: Vec<(String, TableSource)>,
}

/// A table name for a data file: its name without the extension, in
/// lowercase letters, digits and `_`.
pub fn table_name(file: &str) -> String {
    let stem = stem_of(file).to_lowercase();
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    match out.chars().next() {
        None => "table".to_string(),
        Some(c) if c.is_ascii_digit() => format!("t_{out}"),
        Some(_) => out,
    }
}

/// A file name a table may be made of: the path goes into DuckDB's SQL as a
/// string, so names with quotes or backslashes are listed in `files` only.
fn safe_for_sql(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|p| !p.chars().any(|c| c.is_control() || c == '\'' || c == '\\'))
}

/// The tables of a folder viewed in place, for its engine. The index is
/// built first if it is missing (it is a cache).
pub async fn bound(
    catalog: &OikomeCatalog,
    engines: &EngineRegistry,
    location: &Location,
) -> Result<Bound, SessionError> {
    let dir = catalog.index_dir(location);
    let built = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || {
            let conn = open_index(&dir)?;
            exported(&conn, &dir)
        })
        .await
        .map_err(io)??
    };
    if built.is_none() {
        refresh(catalog, engines, location, None).await?;
    }
    let place = place_of(catalog, engines, location)?;
    let reachable = reachable_lately(&dir, &place).await;
    let dir_owned = dir.clone();
    let (files, top) = tokio::task::spawn_blocking(move || {
        let conn = open_index(&dir_owned)?;
        let files =
            exported(&conn, &dir_owned)?.ok_or_else(|| io("the index has no files table"))?;
        let mut stmt = conn
            .prepare(
                "SELECT path, size, modified_ms FROM files WHERE folder = '' AND kind = ?1 \
                 ORDER BY path",
            )
            .map_err(sql)?;
        let top = stmt
            .query_map([TABLE], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok::<_, SessionError>((files, top))
    })
    .await
    .map_err(io)??;

    let mut tables = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::from([FILES.to_string()]);
    for (path, size, modified_ms) in top {
        // The captions of the photos beside it, not a table of its own.
        if name_of(&path) == CAPTIONS {
            continue;
        }
        let Some(format) = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(FileFormat::of_extension)
        else {
            continue;
        };
        let mut name = table_name(&path);
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{}_{n}", table_name(&path));
            n += 1;
        }
        let source = if !reachable {
            TableSource::Unavailable {
                message: format!("{} can't be reached right now", place.label()),
            }
        } else {
            match &place {
                Place::Local(root) => {
                    let file = root.join(&path);
                    if !safe_for_sql(&file) {
                        continue;
                    }
                    TableSource::File { path: file, format }
                }
                Place::Remote { .. } => {
                    match cached(catalog, location, &place, &path, size, modified_ms).await {
                        Ok(file) => TableSource::File { path: file, format },
                        Err(e) => TableSource::Unavailable {
                            message: e.to_string(),
                        },
                    }
                }
            }
        };
        taken.insert(name.clone());
        tables.push((name, source));
    }
    Ok(Bound { files, tables })
}

/// Whether `place` can be reached, asked at most every 15 s: tables are
/// bound before every statement, and a NAS needn't be asked each time.
fn asked() -> &'static Mutex<HashMap<PathBuf, (Instant, bool)>> {
    static ASKED: OnceLock<Mutex<HashMap<PathBuf, (Instant, bool)>>> = OnceLock::new();
    ASKED.get_or_init(Default::default)
}

/// Remember whether the place of the index in `dir` could be reached.
fn remember_reachable(dir: &Path, answer: bool) {
    asked()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(dir.to_path_buf(), (Instant::now(), answer));
}

async fn reachable_lately(dir: &Path, place: &Place) -> bool {
    let asked = asked();
    if let Some((at, answer)) = asked
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(dir)
        .copied()
    {
        if at.elapsed() < Duration::from_secs(15) {
            return answer;
        }
    }
    let answer = place.reachable().await;
    remember_reachable(dir, answer);
    answer
}

/// A NAS's data file, fetched into the cache unless it is there already in
/// the same size and time; the cache keeps at most [`CACHE_BYTES`] per
/// place, the files read least recently going first.
async fn cached(
    catalog: &OikomeCatalog,
    location: &Location,
    place: &Place,
    path: &str,
    size: u64,
    modified_ms: i64,
) -> Result<PathBuf, SessionError> {
    let dir = cache_dir(catalog, location);
    let key = blake3::hash(format!("{path}\0{size}\0{modified_ms}").as_bytes())
        .to_hex()
        .to_string();
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("data")
        .to_ascii_lowercase();
    let file = dir.join(format!("{key}.{extension}"));
    if file.is_file() {
        // Read again: the last to go.
        let _ = std::fs::File::options()
            .append(true)
            .open(&file)
            .and_then(|f| f.set_modified(SystemTime::now()));
        return Ok(file);
    }
    if size > CACHE_BYTES {
        return Err(io(format!(
            "{path} is larger than the cache for a NAS's files"
        )));
    }
    let bytes = place.read(path, 0..u64::MAX, CACHE_BYTES).await?;
    tokio::fs::create_dir_all(&dir).await.map_err(io)?;
    let tmp = dir.join(format!(".{key}.tmp"));
    tokio::fs::write(&tmp, &bytes).await.map_err(io)?;
    tokio::fs::rename(&tmp, &file).await.map_err(io)?;
    trim_cache(&dir);
    Ok(file)
}

fn cache_dir(catalog: &OikomeCatalog, location: &Location) -> PathBuf {
    catalog
        .oikome_dir()
        .join("cache")
        .join("places")
        .join(&location.id)
}

/// Remove the files read least recently until the cache fits.
fn trim_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= CACHE_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Remove a folder's index and cache (the folder itself is never touched).
pub async fn forget(catalog: &OikomeCatalog, location: &Location) -> Result<(), SessionError> {
    for dir in [catalog.index_dir(location), cache_dir(catalog, location)] {
        let lock = lock_for(&dir);
        let _held = lock.lock().await;
        match tokio::fs::remove_dir_all(&dir).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
            _ => {}
        }
    }
    Ok(())
}

/// What the index says of a folder viewed in place.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub files: u64,
    /// When it was last scanned, in milliseconds since the Unix epoch.
    pub scanned_at_ms: Option<u64>,
    /// Whether it could be reached when last asked.
    pub reachable: Option<bool>,
}

pub fn status(catalog: &OikomeCatalog, location: &Location) -> Result<Status, SessionError> {
    let dir = catalog.index_dir(location);
    if !dir.join(INDEX_FILE).is_file() {
        return Ok(Status::default());
    }
    let conn = open_index(&dir)?;
    Ok(Status {
        files: conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
            .map_err(sql)
            .map(|n| u64::try_from(n).unwrap_or(0))?,
        scanned_at_ms: fact(&conn, "scanned_at_ms")?.and_then(|t| t.parse().ok()),
        reachable: fact(&conn, "reachable")?.map(|r| r == "1"),
    })
}

/// A file as the index knows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub modified: String,
    /// The same moment, in milliseconds since the Unix epoch.
    pub modified_ms: i64,
    pub title: Option<String>,
    pub taken: Option<String>,
    pub written: Option<String>,
    pub place: Option<String>,
    pub credit: Option<String>,
    /// A note's text: the start of it in a listing, all of it for one file.
    pub text: Option<String>,
    pub thumbnail: bool,
}

/// A folder inside the folder listed, with the files under it: how many,
/// their bytes, and when the newest of them was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubFolder {
    pub name: String,
    pub files: u64,
    pub size: u64,
    pub modified_ms: Option<i64>,
    /// Photos and notes under it, in all its folders.
    pub photos: u64,
    pub notes: u64,
    /// What its cover shows, as a collection's does: its newest photos
    /// that have a thumbnail (up to three), else its newest note.
    pub cover: Vec<FileEntry>,
}

/// How a folder's files are put in order when listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileOrder {
    /// Photos, then notes, tables and the rest, each by name.
    #[default]
    Kinds,
    Name,
    Modified,
    Size,
    /// By kind (photos, notes, tables, the rest) then name, either way.
    Kind,
}

impl FileOrder {
    /// Its `ORDER BY`, made of these fixed words only.
    fn sql(self, descending: bool) -> &'static str {
        match (self, descending) {
            (FileOrder::Kinds, _) | (FileOrder::Kind, false) => {
                "CASE f.kind WHEN 'image' THEN 0 WHEN 'note' THEN 1 WHEN 'table' THEN 2 ELSE 3 END, \
                 f.name COLLATE NOCASE"
            }
            (FileOrder::Kind, true) => {
                "CASE f.kind WHEN 'image' THEN 0 WHEN 'note' THEN 1 WHEN 'table' THEN 2 ELSE 3 END \
                 DESC, f.name COLLATE NOCASE"
            }
            (FileOrder::Name, false) => "f.name COLLATE NOCASE, f.path",
            (FileOrder::Name, true) => "f.name COLLATE NOCASE DESC, f.path DESC",
            (FileOrder::Modified, false) => "f.modified_ms, f.name COLLATE NOCASE",
            (FileOrder::Modified, true) => "f.modified_ms DESC, f.name COLLATE NOCASE",
            (FileOrder::Size, false) => "f.size, f.name COLLATE NOCASE",
            (FileOrder::Size, true) => "f.size DESC, f.name COLLATE NOCASE",
        }
    }
}

/// One folder of a folder viewed in place, from the index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub folders: Vec<SubFolder>,
    pub files: Vec<FileEntry>,
    /// Files directly in the folder.
    pub total: u64,
}

const ENTRY_COLUMNS: &str = "f.path, f.name, f.kind, f.size, f.modified, f.title, f.taken, \
     f.written, f.place, f.credit, f.text, t.path IS NOT NULL, f.modified_ms";

/// The newest photos with a thumbnail under each folder inside `?1` (the
/// top when `''`; `?2` its `LIKE` prefix), at most `?3` a folder, newest
/// first, each with that folder's name as column 13.
const COVER_PHOTOS: &str = "
WITH under AS (
  SELECT f.rowid AS id, coalesce(f.taken, f.modified) AS at,
         CASE WHEN ?1 = '' THEN f.folder ELSE substr(f.folder, length(?1) + 2) END AS rest
  FROM files f JOIN thumbs t ON t.path = f.path
  WHERE f.kind = 'image' AND f.folder != ?1 AND (?1 = '' OR f.folder LIKE ?2 ESCAPE '\\')
), ranked AS (
  SELECT id, CASE WHEN instr(rest, '/') > 0 THEN substr(rest, 1, instr(rest, '/') - 1) ELSE rest END AS child, at
  FROM under
), numbered AS (
  SELECT id, child, row_number() OVER (PARTITION BY child ORDER BY at DESC, id) AS n FROM ranked
)
SELECT f.path, f.name, f.kind, f.size, f.modified, f.title, f.taken, f.written, f.place, f.credit,
       f.text, t.path IS NOT NULL, f.modified_ms, n.child
FROM numbered n JOIN files f ON f.rowid = n.id LEFT JOIN thumbs t ON t.path = f.path
WHERE n.n <= ?3 ORDER BY n.child, n.n";

/// The newest notes under each folder inside `?1`, as [`COVER_PHOTOS`].
const COVER_NOTES: &str = "
WITH under AS (
  SELECT f.rowid AS id, coalesce(f.written, f.modified) AS at,
         CASE WHEN ?1 = '' THEN f.folder ELSE substr(f.folder, length(?1) + 2) END AS rest
  FROM files f
  WHERE f.kind = 'note' AND f.folder != ?1 AND (?1 = '' OR f.folder LIKE ?2 ESCAPE '\\')
), ranked AS (
  SELECT id, CASE WHEN instr(rest, '/') > 0 THEN substr(rest, 1, instr(rest, '/') - 1) ELSE rest END AS child, at
  FROM under
), numbered AS (
  SELECT id, child, row_number() OVER (PARTITION BY child ORDER BY at DESC, id) AS n FROM ranked
)
SELECT f.path, f.name, f.kind, f.size, f.modified, f.title, f.taken, f.written, f.place, f.credit,
       f.text, t.path IS NOT NULL, f.modified_ms, n.child
FROM numbered n JOIN files f ON f.rowid = n.id LEFT JOIN thumbs t ON t.path = f.path
WHERE n.n <= ?3 ORDER BY n.child, n.n";

fn entry_of(r: &rusqlite::Row<'_>, excerpt: Option<usize>) -> rusqlite::Result<FileEntry> {
    let text: Option<String> = r.get(10)?;
    Ok(FileEntry {
        path: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        size: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
        modified: r.get(4)?,
        title: r.get(5)?,
        taken: r.get(6)?,
        written: r.get(7)?,
        place: r.get(8)?,
        credit: r.get(9)?,
        text: match excerpt {
            Some(chars) => text.map(|t| excerpt_of(&t, chars)),
            None => text,
        },
        thumbnail: r.get(11)?,
        modified_ms: r.get(12)?,
    })
}

/// The start of `text`, at most `chars` characters, cut between words and
/// marked with an ellipsis when there is more.
fn excerpt_of(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(chars).collect();
    let at = cut
        .rfind(char::is_whitespace)
        .filter(|&i| i > chars / 2)
        .unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end())
}

/// The folder `folder` (`""` for the top) of a folder viewed in place: its
/// folders, and a page of its files in `order` (`descending` turns it).
pub fn list(
    catalog: &OikomeCatalog,
    location: &Location,
    folder: &str,
    (order, descending): (FileOrder, bool),
    offset: u64,
    limit: u64,
) -> Result<Listing, SessionError> {
    let folder = folder.trim_matches('/');
    let conn = open_index(&catalog.index_dir(location))?;
    let mut counts: BTreeMap<String, SubFolder> = BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT folder, count(*), coalesce(sum(size), 0), max(modified_ms), \
                 sum(kind = 'image'), sum(kind = 'note') FROM files \
                 WHERE ?1 = '' OR folder = ?1 OR folder LIKE ?2 ESCAPE '\\' GROUP BY folder",
            )
            .map_err(sql)?;
        let prefix = format!("{}/%", like_escape(folder));
        let rows = stmt
            .query_map(params![folder, prefix], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    (r.get::<_, i64>(4)?, r.get::<_, i64>(5)?),
                ))
            })
            .map_err(sql)?;
        for row in rows {
            let (sub, n, size, modified_ms, (photos, notes)) = row.map_err(sql)?;
            let rest = if folder.is_empty() {
                sub.as_str()
            } else {
                match sub.strip_prefix(folder).and_then(|r| r.strip_prefix('/')) {
                    Some(rest) => rest,
                    None => continue,
                }
            };
            if rest.is_empty() {
                continue;
            }
            let child = rest.split('/').next().unwrap_or(rest).to_string();
            let entry = counts.entry(child.clone()).or_insert(SubFolder {
                name: child,
                files: 0,
                size: 0,
                modified_ms: None,
                photos: 0,
                notes: 0,
                cover: Vec::new(),
            });
            entry.files += u64::try_from(n).unwrap_or(0);
            entry.size += u64::try_from(size).unwrap_or(0);
            entry.modified_ms = entry.modified_ms.max(modified_ms);
            entry.photos += u64::try_from(photos).unwrap_or(0);
            entry.notes += u64::try_from(notes).unwrap_or(0);
        }
    }
    if !counts.is_empty() {
        let prefix = format!("{}/%", like_escape(folder));
        // A folder's newest photos are its cover; its newest note only when
        // it has none.
        for (sql_text, per, only_without) in [(COVER_PHOTOS, 3_i64, false), (COVER_NOTES, 1, true)]
        {
            let mut stmt = conn.prepare(sql_text).map_err(sql)?;
            let rows = stmt
                .query_map(params![folder, prefix, per], |r| {
                    Ok((r.get::<_, String>(13)?, entry_of(r, Some(240))?))
                })
                .map_err(sql)?;
            let mut taken: HashSet<String> = HashSet::new();
            for row in rows {
                let (child, entry) = row.map_err(sql)?;
                if let Some(sub) = counts.get_mut(&child) {
                    if !only_without || sub.cover.is_empty() || taken.contains(&child) {
                        taken.insert(child);
                        sub.cover.push(entry);
                    }
                }
            }
        }
    }
    let total = conn
        .query_row(
            "SELECT count(*) FROM files WHERE folder = ?1",
            [folder],
            |r| r.get::<_, i64>(0),
        )
        .map_err(sql)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM files f LEFT JOIN thumbs t ON t.path = f.path \
             WHERE f.folder = ?1 ORDER BY {} LIMIT ?2 OFFSET ?3",
            order.sql(descending)
        ))
        .map_err(sql)?;
    let files = stmt
        .query_map(
            params![
                folder,
                i64::try_from(limit).unwrap_or(i64::MAX),
                i64::try_from(offset).unwrap_or(i64::MAX)
            ],
            |r| entry_of(r, Some(240)),
        )
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    Ok(Listing {
        folders: counts.into_values().collect(),
        files,
        total: u64::try_from(total).unwrap_or(0),
    })
}

/// What a folder viewed in place holds, from its index: its files, their
/// bytes, and when the newest of them was changed (milliseconds since the
/// Unix epoch).
pub fn totals(
    catalog: &OikomeCatalog,
    location: &Location,
) -> Result<(u64, u64, Option<i64>), SessionError> {
    let conn = open_index(&catalog.index_dir(location))?;
    conn.query_row(
        "SELECT count(*), coalesce(sum(size), 0), max(modified_ms) FROM files",
        [],
        |r| {
            Ok((
                u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                r.get::<_, Option<i64>>(2)?,
            ))
        },
    )
    .map_err(sql)
}

fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Files of every folder of a folder viewed in place, of `kind` when one is
/// given (the most recently taken or written first), with how many there
/// are: what a cover or a card on Home shows.
pub fn list_deep(
    catalog: &OikomeCatalog,
    location: &Location,
    kind: Option<&str>,
    offset: u64,
    limit: u64,
) -> Result<Listing, SessionError> {
    let conn = open_index(&catalog.index_dir(location))?;
    let kind = kind.unwrap_or("");
    let total = conn
        .query_row(
            "SELECT count(*) FROM files WHERE ?1 = '' OR kind = ?1",
            [kind],
            |r| r.get::<_, i64>(0),
        )
        .map_err(sql)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM files f LEFT JOIN thumbs t ON t.path = f.path \
             WHERE ?1 = '' OR f.kind = ?1 \
             ORDER BY coalesce(f.taken, f.written, f.modified) DESC, f.path LIMIT ?2 OFFSET ?3"
        ))
        .map_err(sql)?;
    let files = stmt
        .query_map(
            params![
                kind,
                i64::try_from(limit).unwrap_or(i64::MAX),
                i64::try_from(offset).unwrap_or(i64::MAX)
            ],
            |r| entry_of(r, Some(240)),
        )
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    Ok(Listing {
        folders: Vec::new(),
        files,
        total: u64::try_from(total).unwrap_or(0),
    })
}

/// One file of a folder viewed in place, with all of a note's text.
pub fn file(
    catalog: &OikomeCatalog,
    location: &Location,
    path: &str,
) -> Result<Option<FileEntry>, SessionError> {
    let conn = open_index(&catalog.index_dir(location))?;
    conn.query_row(
        &format!(
            "SELECT {ENTRY_COLUMNS} FROM files f LEFT JOIN thumbs t ON t.path = f.path \
             WHERE f.path = ?1"
        ),
        [path],
        |r| entry_of(r, None),
    )
    .optional()
    .map_err(sql)
}

/// A page of the rows of one data file (a CSV, TSV, Parquet or JSON) of a
/// folder viewed in place, read where the file is: a NAS's or a drive's is
/// fetched through its cache. No SQL is involved; the file is parsed by its
/// own reader. At most [`oikome_connect::rows::MAX_VIEW_BYTES`] of the file
/// are read, the view says when it shows only the first part.
pub async fn file_rows(
    catalog: &OikomeCatalog,
    engines: &EngineRegistry,
    location: &Location,
    path: &str,
    offset: u64,
    limit: usize,
) -> Result<oikome_connect::rows::FileRows, SessionError> {
    check_owner_path(path).map_err(io)?;
    let file = file(catalog, location, path)?
        .ok_or_else(|| SessionError::Unsupported(format!("{path} is not a file in it")))?;
    let format = Path::new(&file.name)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(FileFormat::of_extension)
        .ok_or_else(|| SessionError::Unsupported(format!("{path} is not a table to view")))?;
    let place = place_of(catalog, engines, location)?;
    if !place.reachable().await {
        return Err(offline(&place.label()));
    }
    let local = match &place {
        Place::Local(root) => root.join(path),
        Place::Remote { .. } => {
            cached(catalog, location, &place, path, file.size, file.modified_ms).await?
        }
    };
    let name = file.name.clone();
    tokio::task::spawn_blocking(move || {
        let (bytes, cut) = read_capped(&local)?;
        oikome_connect::rows::read_page(format, &name, &bytes, offset, limit, cut)
            .map_err(|e| SessionError::Unsupported(format!("viewing {name} as a table: {e}")))
    })
    .await
    .map_err(io)?
}

/// The first [`oikome_connect::rows::MAX_VIEW_BYTES`] of `file`, and whether
/// the file goes on past them.
fn read_capped(file: &Path) -> Result<(Vec<u8>, bool), SessionError> {
    use std::io::Read;
    let mut opened = std::fs::File::open(file).map_err(io)?;
    let mut bytes = Vec::new();
    opened
        .by_ref()
        .take(oikome_connect::rows::MAX_VIEW_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    let cut = bytes.len() > oikome_connect::rows::MAX_VIEW_BYTES;
    bytes.truncate(oikome_connect::rows::MAX_VIEW_BYTES);
    Ok((bytes, cut))
}

/// The photos of a folder viewed in place (all its folders), newest
/// taken first, up to `limit`.
pub fn photos(
    catalog: &OikomeCatalog,
    location: &Location,
    limit: u64,
) -> Result<Vec<FileEntry>, SessionError> {
    let conn = open_index(&catalog.index_dir(location))?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM files f LEFT JOIN thumbs t ON t.path = f.path \
             WHERE f.kind = 'image' ORDER BY f.path LIMIT ?1"
        ))
        .map_err(sql)?;
    let rows = stmt
        .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
            entry_of(r, None)
        })
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

/// A photo's thumbnail (JPEG), if the index has one.
pub fn thumb(
    catalog: &OikomeCatalog,
    location: &Location,
    path: &str,
) -> Result<Option<Vec<u8>>, SessionError> {
    let dir = catalog.index_dir(location);
    if !dir.join(INDEX_FILE).is_file() {
        return Ok(None);
    }
    let conn = open_index(&dir)?;
    conn.query_row("SELECT jpeg FROM thumbs WHERE path = ?1", [path], |r| {
        r.get::<_, Vec<u8>>(0)
    })
    .optional()
    .map_err(sql)
}

/// Search a folder viewed in place through its index: file names, photos'
/// titles and notes' text.
pub fn search(
    catalog: &OikomeCatalog,
    location: &Location,
    terms: &[String],
    limit: usize,
) -> Result<Vec<SearchMatch>, SessionError> {
    let dir = catalog.index_dir(location);
    if !dir.join(INDEX_FILE).is_file() {
        return Ok(Vec::new());
    }
    let conn = open_index(&dir)?;
    let (long, short): (Vec<&str>, Vec<&str>) = terms
        .iter()
        .map(String::as_str)
        .partition(|t| t.chars().count() >= MIN_INDEXED_CHARS);
    let mut values: Vec<String> = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    if !long.is_empty() {
        values.push(fts_query(&long));
        conditions.push(format!("files_fts MATCH ?{}", values.len()));
    }
    for term in short {
        values.push(like_pattern(term));
        let n = values.len();
        conditions.push(format!(
            "(f.name LIKE ?{n} ESCAPE '\\' OR f.title LIKE ?{n} ESCAPE '\\' \
             OR f.text LIKE ?{n} ESCAPE '\\')"
        ));
    }
    if conditions.is_empty() {
        return Ok(Vec::new());
    }
    let ranked = !long.is_empty();
    let sql_text = format!(
        "SELECT f.path, f.name, f.title, f.text, {} FROM files f {} WHERE {} ORDER BY 5 LIMIT {}",
        if ranked { "bm25(files_fts)" } else { "0.0" },
        if ranked {
            "JOIN files_fts ON files_fts.rowid = f.rowid"
        } else {
            ""
        },
        conditions.join(" AND "),
        limit
    );
    let mut stmt = conn.prepare(&sql_text).map_err(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            let path: String = r.get(0)?;
            // A note is shown by its text and a photo by its caption, before
            // its file's name (a note's title is its file's name).
            let mut texts = vec![("path".to_string(), path.clone())];
            for (i, column) in [(3, "text"), (2, "title")] {
                if let Some(text) = r.get::<_, Option<String>>(i)? {
                    texts.push((column.to_string(), text));
                }
            }
            texts.push(("name".to_string(), r.get(1)?));
            Ok(SearchMatch {
                table: FILES.to_string(),
                key: Some(vec![path]),
                texts,
                score: ranked.then(|| r.get::<_, f64>(4)).transpose()?,
            })
        })
        .map_err(sql)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql)
}

impl crate::session::OikomeSession {
    /// The Location of `space` if it views a folder in place.
    pub fn in_place_location(&self, space: &str) -> Result<Option<Location>, SessionError> {
        let def = self.space(space)?;
        let location = self.catalog.get_location(&def.location)?;
        Ok(matches!(location.format, LocationFormat::InPlace { .. }).then_some(location))
    }

    /// `CREATE SPACE <name> WITH PLACE = …, PATH = …`: view a folder in
    /// place. A folder on this computer must exist and be neither hidden nor
    /// inside the data center's own `.oikome`; one on a NAS must answer. The
    /// first files are read at once, the rest in the background. Owner
    /// only.
    pub(crate) async fn create_in_place(
        &self,
        mut request: oikome_core::CreateSpaceRequest,
    ) -> Result<(), SessionError> {
        self.deny_principal("CREATE SPACE")?;
        let place = request.place.clone().unwrap_or_default();
        let path = request.path.clone().unwrap_or_default();
        request.path = Some(if place == oikome_catalog::space::PLACE_LOCAL {
            self.local_folder(&path)?
        } else {
            let within = path.trim_matches('/').to_string();
            if !within.is_empty() {
                check_owner_path(&within).map_err(io)?;
            }
            within
        });
        self.catalog.create_space(request.clone()).await?;
        let space = self.space(&request.name)?;
        let location = self.catalog.get_location(&space.location)?;
        let made = async {
            let place = place_of(&self.catalog, self.engines(), &location)?;
            if !place.reachable().await {
                return Err(offline(&place.label()));
            }
            refresh(
                &self.catalog,
                self.engines(),
                &location,
                Some(Duration::from_secs(10)),
            )
            .await
        }
        .await;
        if let Err(e) = made {
            let _ = self.catalog.drop_space(&request.name);
            let _ = forget(&self.catalog, &location).await;
            return Err(e);
        }
        let (catalog, engines) = (self.catalog.clone(), self.engines_arc());
        tokio::spawn(async move {
            if let Err(e) = refresh(&catalog, &engines, &location, None).await {
                tracing::warn!(location = %location.id, error = %e, "reading a folder viewed in place");
            }
        });
        Ok(())
    }

    /// A folder on this computer to view in place, as its canonical path.
    fn local_folder(&self, path: &str) -> Result<String, SessionError> {
        let refuse = |why: &str| SessionError::Unsupported(format!("{path}: {why}"));
        if path.chars().any(char::is_control) || !Path::new(path).is_absolute() {
            return Err(refuse("give the whole path of a folder on this computer"));
        }
        let folder = std::fs::canonicalize(path).map_err(|_| refuse("there is no folder there"))?;
        if !folder.is_dir() {
            return Err(refuse("it is a file, not a folder"));
        }
        // A hidden folder itself (`~/.ssh`); what is hidden inside a folder
        // is left out when it is read.
        if folder
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            return Err(refuse("it is a hidden folder"));
        }
        let own = std::fs::canonicalize(self.catalog.oikome_dir())
            .unwrap_or_else(|_| self.catalog.oikome_dir());
        if folder.starts_with(&own) {
            return Err(refuse("it is the data center's own folder"));
        }
        folder
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| refuse("its path is not UTF-8"))
    }

    /// Scan a folder viewed in place again (`REFRESH SPACE`). Owner only.
    pub async fn refresh_in_place(&self, name: &str) -> Result<Refreshed, SessionError> {
        self.deny_principal("REFRESH SPACE")?;
        let location = self.in_place_location(name)?.ok_or_else(|| {
            SessionError::Unsupported(format!("space '{name}' does not view a folder in place"))
        })?;
        refresh(&self.catalog, self.engines(), &location, None).await
    }

    /// Scan every folder viewed in place, each for at most `budget`, on the
    /// node's schedule; failures are logged and retried next time.
    pub async fn refresh_places(&self, budget: Duration) {
        for space in self.catalog.list_spaces() {
            let Ok(location) = self.catalog.get_location(&space.location) else {
                continue;
            };
            if !matches!(location.format, LocationFormat::InPlace { .. }) {
                continue;
            }
            if let Err(e) = refresh(&self.catalog, self.engines(), &location, Some(budget)).await {
                tracing::warn!(space = %space.name, error = %e, "scanning a folder viewed in place");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_told_apart_by_their_extensions() {
        assert_eq!(kind_of("Pastéis.JPG"), IMAGE);
        assert_eq!(kind_of("1 Welcome.md"), NOTE);
        assert_eq!(kind_of("2025.csv"), TABLE);
        assert_eq!(kind_of("rows.parquet"), TABLE);
        assert_eq!(kind_of("movie.mov"), OTHER);
        assert_eq!(kind_of("README"), OTHER);
    }

    #[test]
    fn an_excerpt_ends_between_words() {
        assert_eq!(excerpt_of("short", 10), "short");
        assert_eq!(
            excerpt_of("Sources shows where each collection", 26),
            "Sources shows where each…"
        );
        assert_eq!(excerpt_of("里斯本的电车整天在爬坡", 5), "里斯本的电…");
    }

    #[test]
    fn tables_are_named_after_their_files() {
        assert_eq!(table_name("Spending 2025.csv"), "spending_2025");
        assert_eq!(table_name("2025.csv"), "t_2025");
        assert_eq!(table_name("Lisboa — café.parquet"), "lisboa_caf");
        assert_eq!(table_name("....csv"), "table");
    }

    #[test]
    fn a_path_with_a_quote_makes_no_table() {
        assert!(safe_for_sql(Path::new("/data/rows.csv")));
        assert!(safe_for_sql(Path::new("/data/Café 2025 (all).csv")));
        assert!(!safe_for_sql(Path::new("/data/it's.csv")));
        assert!(!safe_for_sql(Path::new("/data/a\\b.csv")));
    }

    #[test]
    fn a_thumbnail_fits_its_edge_and_keeps_the_shape() {
        let image = image::RgbImage::from_fn(1200, 600, |x, _| image::Rgb([(x % 255) as u8, 0, 0]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (jpeg, width, height) = thumbnail(&png).expect("a thumbnail");
        assert_eq!((width, height), (512, 256));
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert!(thumbnail(b"not an image").is_none());
    }
}
