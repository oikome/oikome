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

//! Datasets in a folder viewed in place (PR 31). A Lance dataset or an
//! Iceberg table another program wrote is a folder of its own files.
//! Indexing lists it as one item (kind `dataset`, its kind in `text`)
//! instead of those files, and it is a table of the collection, read where
//! it is through DuckDB's `lance` and `iceberg` extensions and never
//! written: its latest version each time, as a file's table follows its
//! file.
//!
//! Only a dataset's own files are read. A Lance version that uses files of
//! another dataset, overlays or indexes elsewhere is refused (its manifest
//! is read with `lance-table`). An Iceberg table's metadata, manifest list
//! and manifests are read here and refused when any path in them is
//! outside the table's location. When the table was written elsewhere
//! (moved, or fetched from a NAS into the cache), DuckDB is given a
//! rewritten copy of that chain whose paths are the files actually read, so
//! no absolute path written into a table can make DuckDB read another file.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use apache_avro::types::Value;
use apache_avro::{Codec, Reader, Writer};
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use oikome_catalog::{Location, OikomeCatalog};
use oikome_engine::TableSource;

use crate::error::SessionError;
use crate::inplace::{self, Place};

/// The folder of a Lance dataset's manifests, one per version.
pub const LANCE_VERSIONS: &str = "_versions";
/// The folder of an Iceberg table's metadata.
pub const ICEBERG_METADATA: &str = "metadata";
/// The most a dataset on another place may fetch into the cache: the files
/// of the version read.
pub const MAX_FETCH_BYTES: u64 = 2 << 30;

fn io(e: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("reading a dataset: {e}"))
}

fn refused(path: &str, why: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("{path} is not read: {why}"))
}

/// What a dataset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DatasetKind {
    Lance,
    Iceberg,
}

impl DatasetKind {
    /// Its name, as kept in the index and named in the API.
    pub fn name(self) -> &'static str {
        match self {
            DatasetKind::Lance => "lance",
            DatasetKind::Iceberg => "iceberg",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "lance" => Some(DatasetKind::Lance),
            "iceberg" => Some(DatasetKind::Iceberg),
            _ => None,
        }
    }

    /// The DuckDB extension that reads it.
    pub fn extension(self) -> &'static str {
        self.name()
    }
}

/// The dataset folder a file of a listing is the mark of, if any:
/// `<root>/_versions/<n>.manifest` marks a Lance dataset and
/// `<root>/metadata/<..>.metadata.json` (or its `version-hint.text`) an
/// Iceberg table. The viewed folder itself is never one of its datasets.
pub fn root_of(path: &str) -> Option<(&str, DatasetKind)> {
    let (folder, name) = path.rsplit_once('/')?;
    let (root, inner) = folder.rsplit_once('/')?;
    if root.is_empty() {
        return None;
    }
    match inner {
        LANCE_VERSIONS if name.ends_with(".manifest") => Some((root, DatasetKind::Lance)),
        ICEBERG_METADATA if name.ends_with(".metadata.json") || name == "version-hint.text" => {
            Some((root, DatasetKind::Iceberg))
        }
        _ => None,
    }
}

/// The dataset folders among `paths`: an outer dataset holds any found
/// inside it.
pub fn roots<'a>(paths: impl Iterator<Item = &'a str>) -> BTreeMap<String, DatasetKind> {
    let mut found: BTreeMap<String, DatasetKind> = BTreeMap::new();
    for path in paths {
        if let Some((root, kind)) = root_of(path) {
            found.entry(root.to_string()).or_insert(kind);
        }
    }
    let all: Vec<String> = found.keys().cloned().collect();
    found.retain(|root, _| {
        !all.iter()
            .any(|outer| outer != root && root.starts_with(&format!("{outer}/")))
    });
    found
}

/// How a dataset is read now.
#[derive(Debug, Clone)]
pub struct DatasetRead {
    pub kind: DatasetKind,
    pub source: TableSource,
    /// Lance's version number; Iceberg's snapshot, by its sequence number
    /// when it has one.
    pub version: Option<String>,
    /// When that version was written, in milliseconds since the epoch.
    pub written_at_ms: Option<i64>,
    /// The folder its files are read from: the dataset's own on this
    /// computer, or where they were fetched to.
    pub base: PathBuf,
    /// The files of the version read, relative to `base`.
    pub files: Vec<String>,
}

/// Reads worked out lately, by dataset and what it looked like then: a
/// statement binds its tables first, and a dataset needn't be worked out
/// again until it changes.
fn reads() -> &'static Mutex<HashMap<String, DatasetRead>> {
    static READS: OnceLock<Mutex<HashMap<String, DatasetRead>>> = OnceLock::new();
    READS.get_or_init(Default::default)
}

/// Serialised per dataset: two statements fetching the same dataset at once
/// would write the same files.
fn lock_for(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(key.to_string())
        .or_default()
        .clone()
}

/// Where the files of datasets on another place, and the rewritten
/// metadata of Iceberg tables written elsewhere, are kept.
pub(crate) fn datasets_cache(catalog: &OikomeCatalog, location: &Location) -> PathBuf {
    inplace::cache_dir(catalog, location).join("datasets")
}

fn key_of(path: &str) -> String {
    blake3::hash(path.as_bytes()).to_hex()[..32].to_string()
}

/// The dataset at `path` of the folder `place` (its `size` and time as the
/// index has them), as DuckDB is to read it.
pub async fn read(
    catalog: &OikomeCatalog,
    location: &Location,
    place: &Place,
    path: &str,
    kind: DatasetKind,
    size: u64,
    modified_ms: i64,
) -> Result<DatasetRead, SessionError> {
    let cache = datasets_cache(catalog, location).join(key_of(path));
    match place.local_file(path) {
        Some(dir) => {
            let mark = local_mark(&dir, kind)?;
            let key = format!("{}\0{}\0{mark}", location.id, path);
            if let Some(read) = reads().lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
                return Ok(read.clone());
            }
            let lock = lock_for(&key);
            let _held = lock.lock().await;
            let read = match kind {
                DatasetKind::Lance => lance_local(path, &dir).await?,
                DatasetKind::Iceberg => iceberg_local(path, &dir, &cache)?,
            };
            reads()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key, read.clone());
            Ok(read)
        }
        None => {
            let key = format!("{}\0{}\0{size}\0{modified_ms}", location.id, path);
            if let Some(read) = reads().lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
                if source_exists(&read.source) {
                    return Ok(read.clone());
                }
            }
            let lock = lock_for(&key);
            let _held = lock.lock().await;
            let listed = list_remote(place, path).await?;
            let read = match kind {
                DatasetKind::Lance => lance_remote(place, path, &listed, &cache).await?,
                DatasetKind::Iceberg => iceberg_remote(place, path, &listed, &cache).await?,
            };
            reads()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key, read.clone());
            Ok(read)
        }
    }
}

fn source_exists(source: &TableSource) -> bool {
    match source {
        TableSource::LanceDataset { path } => path.is_dir(),
        TableSource::IcebergTable { metadata } => metadata.is_file(),
        _ => true,
    }
}

/// Forget what was fetched for datasets of `location` that are not among
/// `kept` any more.
pub(crate) fn forget_others(catalog: &OikomeCatalog, location: &Location, kept: &[String]) {
    let dir = datasets_cache(catalog, location);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let keep: BTreeSet<String> = kept.iter().map(|p| key_of(p)).collect();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !keep.contains(&name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// What tells a dataset on this computer has changed: its newest manifest,
/// or its metadata files, by name and size.
fn local_mark(dir: &Path, kind: DatasetKind) -> Result<String, SessionError> {
    let folder = dir.join(match kind {
        DatasetKind::Lance => LANCE_VERSIONS,
        DatasetKind::Iceberg => ICEBERG_METADATA,
    });
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&folder).map_err(io)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let keep = match kind {
            DatasetKind::Lance => name.ends_with(".manifest"),
            DatasetKind::Iceberg => name.ends_with(".metadata.json") || name == "version-hint.text",
        };
        if keep {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            names.push(format!("{name}:{len}"));
        }
    }
    names.sort();
    Ok(blake3::hash(names.join("\n").as_bytes())
        .to_hex()
        .to_string())
}

// ---------------------------------------------------------------- Lance

async fn lance_local(path: &str, dir: &Path) -> Result<DatasetRead, SessionError> {
    let version = oikome_catalog::lance::lance_version(dir, None)
        .await
        .map_err(|e| refused(path, e))?;
    Ok(DatasetRead {
        kind: DatasetKind::Lance,
        source: TableSource::LanceDataset {
            path: dir.to_path_buf(),
        },
        version: Some(version.version.to_string()),
        written_at_ms: version.written_at_ms,
        base: dir.to_path_buf(),
        files: std::iter::once(version.manifest)
            .chain(version.files)
            .collect(),
    })
}

/// A Lance manifest's version, by its name: `<version>.manifest`, or, in
/// the naming that lists the newest first, `<u64::MAX - version>` in 20
/// digits.
fn manifest_version(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".manifest")?;
    let n: u64 = stem.parse().ok()?;
    Some(if stem.len() == 20 && n > u64::MAX >> 1 {
        u64::MAX - n
    } else {
        n
    })
}

async fn lance_remote(
    place: &Place,
    path: &str,
    listed: &BTreeMap<String, (u64, i64)>,
    cache: &Path,
) -> Result<DatasetRead, SessionError> {
    let latest = listed
        .keys()
        .filter_map(|rel| {
            let name = rel.strip_prefix(&format!("{LANCE_VERSIONS}/"))?;
            (!name.contains('/'))
                .then(|| manifest_version(name).map(|v| (v, rel.clone())))
                .flatten()
        })
        .max()
        .map(|(_, rel)| rel)
        .ok_or_else(|| refused(path, "it has no version"))?;
    let mirror = cache.join("files");
    fetch(place, path, listed, std::slice::from_ref(&latest), &mirror).await?;
    // Only the version read is kept: the newest manifest there is it.
    prune(&mirror, &BTreeSet::from([latest.clone()]), |rel| {
        rel.starts_with(&format!("{LANCE_VERSIONS}/"))
    })?;
    let version = oikome_catalog::lance::lance_version(&mirror, None)
        .await
        .map_err(|e| refused(path, e))?;
    fetch(place, path, listed, &version.files, &mirror).await?;
    let mut keep: BTreeSet<String> = version.files.iter().cloned().collect();
    keep.insert(latest);
    prune(&mirror, &keep, |_| true)?;
    Ok(DatasetRead {
        kind: DatasetKind::Lance,
        source: TableSource::LanceDataset {
            path: mirror.clone(),
        },
        version: Some(version.version.to_string()),
        written_at_ms: version.written_at_ms,
        base: mirror,
        files: keep.into_iter().collect(),
    })
}

// ---------------------------------------------------------------- Iceberg

/// A path as a plain path when it names this computer's file system.
fn plain(path: &str) -> &str {
    path.strip_prefix("file://")
        .or_else(|| path.strip_prefix("file:"))
        .unwrap_or(path)
}

/// `path` relative to the table's `location`, or why it is refused: it is
/// outside it, or it climbs out.
fn relative(location: &str, path: &str) -> Result<String, String> {
    let base = plain(location).trim_end_matches('/');
    let rest = plain(path)
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| format!("it refers to {path}, outside its folder"))?;
    let bad = rest.is_empty()
        || rest.contains('\\')
        || rest.chars().any(char::is_control)
        || rest
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        return Err(format!("it refers to {path}, outside its folder"));
    }
    Ok(rest.to_string())
}

/// The metadata file of an Iceberg table's current version, among the
/// names in its `metadata` folder: the one `version-hint.text` names, else
/// the highest numbered (`v<n>.metadata.json`, `<n>-<uuid>.metadata.json`).
fn current_metadata(names: &[String], hint: Option<&str>) -> Option<String> {
    if let Some(hint) = hint.map(str::trim).filter(|h| !h.is_empty()) {
        for candidate in [
            format!("v{hint}.metadata.json"),
            format!("{hint}.metadata.json"),
            hint.to_string(),
        ] {
            if names.contains(&candidate) && !candidate.contains('/') {
                return Some(candidate);
            }
        }
    }
    names
        .iter()
        .filter(|name| name.ends_with(".metadata.json"))
        .filter_map(|name| {
            let stem = name.strip_suffix(".metadata.json")?;
            let digits = stem
                .strip_prefix('v')
                .unwrap_or_else(|| stem.split('-').next().unwrap_or(stem));
            Some((digits.parse::<u64>().ok()?, name.clone()))
        })
        .max()
        .map(|(_, name)| name)
}

/// One version of an Iceberg table: what is read, and the files it uses.
#[derive(Debug)]
struct Chain {
    /// The metadata file, relative to the table's folder.
    metadata: String,
    metadata_json: serde_json::Value,
    location: String,
    snapshot: Option<String>,
    written_at_ms: Option<i64>,
    /// The manifest list and the manifests, relative.
    lists: Vec<String>,
    manifests: Vec<String>,
    /// Live data and delete files, relative: (path, content, format).
    files: Vec<(String, i64, String)>,
}

impl Chain {
    fn all_files(&self) -> Vec<String> {
        let mut all = vec![self.metadata.clone()];
        all.extend(self.lists.iter().cloned());
        all.extend(self.manifests.iter().cloned());
        all.extend(self.files.iter().map(|(p, _, _)| p.clone()));
        all
    }
}

/// A record's field, through the union an optional field is.
fn field<'a>(record: &'a Value, name: &str) -> Option<&'a Value> {
    let Value::Record(fields) = record else {
        return None;
    };
    let mut value = &fields.iter().find(|(k, _)| k == name)?.1;
    while let Value::Union(_, inner) = value {
        value = inner;
    }
    match value {
        Value::Null => None,
        v => Some(v),
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::String(s) => Some(s),
        Value::Enum(_, s) => Some(s),
        _ => None,
    }
}

fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(i) => Some(*i),
        _ => None,
    }
}

fn avro_records(bytes: &[u8]) -> Result<Vec<Value>, String> {
    Reader::new(bytes)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Read the chain of the version `metadata` names, each file through
/// `read` (relative to the table's folder).
fn chain(
    path: &str,
    metadata: String,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, SessionError>,
) -> Result<Chain, SessionError> {
    let json: serde_json::Value =
        serde_json::from_slice(&read(&format!("{ICEBERG_METADATA}/{metadata}"))?)
            .map_err(|e| refused(path, format!("its metadata can't be read: {e}")))?;
    let location = json["location"]
        .as_str()
        .ok_or_else(|| refused(path, "its metadata names no location"))?
        .to_string();
    let current = json["current-snapshot-id"].as_i64().filter(|id| *id >= 0);
    let mut chain = Chain {
        metadata: format!("{ICEBERG_METADATA}/{metadata}"),
        metadata_json: json.clone(),
        location: location.clone(),
        snapshot: None,
        written_at_ms: json["last-updated-ms"].as_i64(),
        lists: Vec::new(),
        manifests: Vec::new(),
        files: Vec::new(),
    };
    let Some(current) = current else {
        return Ok(chain);
    };
    let snapshot = json["snapshots"]
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|s| s["snapshot-id"].as_i64() == Some(current))
        })
        .ok_or_else(|| refused(path, "its current snapshot is missing"))?;
    chain.snapshot = Some(
        snapshot["sequence-number"]
            .as_i64()
            .filter(|n| *n > 0)
            .unwrap_or(current)
            .to_string(),
    );
    chain.written_at_ms = snapshot["timestamp-ms"].as_i64().or(chain.written_at_ms);
    let list = snapshot["manifest-list"].as_str().ok_or_else(|| {
        refused(
            path,
            "its snapshot lists manifests in its metadata (Iceberg v1 without a manifest list)",
        )
    })?;
    let list = relative(&location, list).map_err(|e| refused(path, e))?;
    let entries = avro_records(&read(&list)?)
        .map_err(|e| refused(path, format!("its manifest list can't be read: {e}")))?;
    chain.lists.push(list);
    for entry in entries {
        let manifest = text(field(&entry, "manifest_path"))
            .ok_or_else(|| refused(path, "its manifest list has an entry without a path"))?;
        let manifest = relative(&location, manifest).map_err(|e| refused(path, e))?;
        let records = avro_records(&read(&manifest)?)
            .map_err(|e| refused(path, format!("a manifest can't be read: {e}")))?;
        chain.manifests.push(manifest);
        for record in records {
            // Deleted entries are not part of the snapshot.
            if int(field(&record, "status")) == Some(2) {
                continue;
            }
            let file = field(&record, "data_file")
                .ok_or_else(|| refused(path, "a manifest entry has no file"))?;
            let file_path = text(field(file, "file_path"))
                .ok_or_else(|| refused(path, "a manifest entry has no path"))?;
            let content = int(field(file, "content")).unwrap_or(0);
            let format = text(field(file, "file_format"))
                .unwrap_or("PARQUET")
                .to_ascii_uppercase();
            if let Some(other) = text(field(file, "referenced_data_file")) {
                relative(&location, other).map_err(|e| refused(path, e))?;
            }
            let rel = relative(&location, file_path).map_err(|e| refused(path, e))?;
            chain.files.push((rel, content, format));
        }
    }
    Ok(chain)
}

fn read_local(dir: &Path, rel: &str) -> Result<Vec<u8>, SessionError> {
    std::fs::read(dir.join(rel)).map_err(|e| io(format!("{rel}: {e}")))
}

fn metadata_names(dir: &Path) -> Result<(Vec<String>, Option<String>), SessionError> {
    let folder = dir.join(ICEBERG_METADATA);
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&folder).map_err(io)?.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let hint = std::fs::read_to_string(folder.join("version-hint.text")).ok();
    Ok((names, hint))
}

fn iceberg_local(path: &str, dir: &Path, cache: &Path) -> Result<DatasetRead, SessionError> {
    let (names, hint) = metadata_names(dir)?;
    let metadata = current_metadata(&names, hint.as_deref())
        .ok_or_else(|| refused(path, "it has no metadata file"))?;
    let chain = chain(path, metadata, |rel| read_local(dir, rel))?;
    // Written here: every path in it is one of its own files already.
    let written_here = std::fs::canonicalize(plain(&chain.location))
        .ok()
        .zip(std::fs::canonicalize(dir).ok())
        .is_some_and(|(a, b)| a == b);
    let metadata = if written_here {
        dir.join(&chain.metadata)
    } else {
        rewritten(path, &chain, dir, cache)?
    };
    Ok(DatasetRead {
        kind: DatasetKind::Iceberg,
        source: TableSource::IcebergTable { metadata },
        version: chain.snapshot.clone(),
        written_at_ms: chain.written_at_ms,
        base: dir.to_path_buf(),
        files: chain.all_files(),
    })
}

async fn iceberg_remote(
    place: &Place,
    path: &str,
    listed: &BTreeMap<String, (u64, i64)>,
    cache: &Path,
) -> Result<DatasetRead, SessionError> {
    let mirror = cache.join("files");
    let names: Vec<String> = listed
        .keys()
        .filter_map(|rel| rel.strip_prefix(&format!("{ICEBERG_METADATA}/")))
        .filter(|name| !name.contains('/'))
        .map(str::to_string)
        .collect();
    let hint_rel = format!("{ICEBERG_METADATA}/version-hint.text");
    let hint = if listed.contains_key(&hint_rel) {
        fetch(
            place,
            path,
            listed,
            std::slice::from_ref(&hint_rel),
            &mirror,
        )
        .await?;
        std::fs::read_to_string(mirror.join(&hint_rel)).ok()
    } else {
        None
    };
    let metadata = current_metadata(&names, hint.as_deref())
        .ok_or_else(|| refused(path, "it has no metadata file"))?;
    // The chain is read file by file, each fetched as it is named.
    let mut wanted = vec![format!("{ICEBERG_METADATA}/{metadata}")];
    let chain = loop {
        fetch(place, path, listed, &wanted, &mirror).await?;
        let mut missing = None;
        let attempt = chain(path, metadata.clone(), |rel| {
            if mirror.join(rel).is_file() {
                read_local(&mirror, rel)
            } else {
                missing = Some(rel.to_string());
                Err(io(format!("{rel} is being fetched")))
            }
        });
        match (attempt, missing) {
            (Ok(chain), _) => break chain,
            (Err(_), Some(rel)) if listed.contains_key(&rel) && !wanted.contains(&rel) => {
                wanted.push(rel);
            }
            (Err(_), Some(rel)) => {
                return Err(refused(path, format!("{rel} is not in its folder")));
            }
            (Err(e), None) => return Err(e),
        }
    };
    let files = chain.all_files();
    for rel in &files {
        if !listed.contains_key(rel) {
            return Err(refused(path, format!("{rel} is not in its folder")));
        }
    }
    fetch(place, path, listed, &files, &mirror).await?;
    let keep: BTreeSet<String> = files
        .iter()
        .cloned()
        .chain(listed.contains_key(&hint_rel).then_some(hint_rel))
        .collect();
    prune(&mirror, &keep, |_| true)?;
    let metadata = rewritten(path, &chain, &mirror, cache)?;
    Ok(DatasetRead {
        kind: DatasetKind::Iceberg,
        source: TableSource::IcebergTable { metadata },
        version: chain.snapshot.clone(),
        written_at_ms: chain.written_at_ms,
        base: mirror,
        files,
    })
}

/// Every string in `value` that names a file of the table, through `map`.
fn map_json(
    value: &mut serde_json::Value,
    location: &str,
    map: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    match value {
        serde_json::Value::String(s) => {
            if let Some(new) = mapped(s, location, map)? {
                *s = new;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .try_for_each(|v| map_json(v, location, map)),
        serde_json::Value::Object(fields) => fields
            .values_mut()
            .try_for_each(|v| map_json(v, location, map)),
        _ => Ok(()),
    }
}

/// `text` mapped when it is the table's location or a path in it.
fn mapped(
    text: &str,
    location: &str,
    map: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<String>, String> {
    let base = plain(location).trim_end_matches('/');
    let plain_text = plain(text);
    if plain_text == base || plain_text == format!("{base}/") {
        return map("").map(Some);
    }
    if plain_text.starts_with(&format!("{base}/")) {
        let rel = relative(location, text)?;
        return map(&rel).map(Some);
    }
    Ok(None)
}

fn map_avro(
    value: Value,
    location: &str,
    map: &dyn Fn(&str) -> Result<String, String>,
    sizes: &HashMap<String, i64>,
) -> Result<Value, String> {
    Ok(match value {
        Value::String(s) => Value::String(mapped(&s, location, map)?.unwrap_or(s)),
        Value::Union(i, v) => Value::Union(i, Box::new(map_avro(*v, location, map, sizes)?)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| map_avro(v, location, map, sizes))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(entries) => Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| Ok((k, map_avro(v, location, map, sizes)?)))
                .collect::<Result<_, String>>()?,
        ),
        Value::Record(fields) => {
            let mut fields: Vec<(String, Value)> = fields
                .into_iter()
                .map(|(k, v)| Ok((k, map_avro(v, location, map, sizes)?)))
                .collect::<Result<_, String>>()?;
            // A manifest list names each manifest's length: the rewritten one's.
            let length = fields.iter().find_map(|(k, v)| match (k.as_str(), v) {
                ("manifest_path", Value::String(p)) => sizes.get(p).copied(),
                _ => None,
            });
            if let Some(length) = length {
                for (k, v) in &mut fields {
                    if k == "manifest_length" {
                        *v = Value::Long(length);
                    }
                }
            }
            Value::Record(fields)
        }
        other => other,
    })
}

fn rewrite_avro(
    bytes: &[u8],
    location: &str,
    map: &dyn Fn(&str) -> Result<String, String>,
    sizes: &HashMap<String, i64>,
) -> Result<Vec<u8>, String> {
    let reader = Reader::new(bytes).map_err(|e| e.to_string())?;
    let schema = reader.writer_schema().clone();
    let metadata = reader.user_metadata().clone();
    let records: Vec<Value> = reader
        .map(|r| {
            r.map_err(|e| e.to_string())
                .and_then(|v| map_avro(v, location, map, sizes))
        })
        .collect::<Result<_, _>>()?;
    let mut writer = Writer::with_codec(&schema, Vec::new(), Codec::Null);
    for (key, value) in metadata {
        writer
            .add_user_metadata(key, value)
            .map_err(|e| e.to_string())?;
    }
    for record in records {
        writer.append(record).map_err(|e| e.to_string())?;
    }
    writer.into_inner().map_err(|e| e.to_string())
}

/// A positional delete file with the data files it names mapped.
fn rewrite_deletes(
    from: &Path,
    to: &Path,
    location: &str,
    map: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use parquet::arrow::ArrowWriter;

    let file = std::fs::File::open(from).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let schema = builder.schema().clone();
    let column = schema.index_of("file_path").map_err(|e| e.to_string())?;
    let reader = builder.build().map_err(|e| e.to_string())?;
    let out = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut writer = ArrowWriter::try_new(out, schema.clone(), None).map_err(|e| e.to_string())?;
    for batch in reader {
        let batch = batch.map_err(|e| e.to_string())?;
        let paths = batch
            .column(column)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("a delete file's paths are not text")?;
        let moved: StringArray = (0..paths.len())
            .map(|i| {
                if paths.is_null(i) {
                    return Ok(None);
                }
                let path = paths.value(i);
                Ok(Some(mapped(path, location, map)?.ok_or_else(|| {
                    format!("a delete file refers to {path}, outside its folder")
                })?))
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .collect();
        let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
        columns[column] = Arc::new(moved);
        let batch = RecordBatch::try_new(schema.clone(), columns).map_err(|e| e.to_string())?;
        writer.write(&batch).map_err(|e| e.to_string())?;
    }
    writer.close().map_err(|e| e.to_string())?;
    Ok(())
}

/// The metadata chain of `chain` rewritten under `cache` so that each path
/// in it is the file read: the table's files in `base`, the rewritten ones
/// beside the metadata. Made once per version.
fn rewritten(
    path: &str,
    chain: &Chain,
    base: &Path,
    cache: &Path,
) -> Result<PathBuf, SessionError> {
    let version = blake3::hash(
        format!(
            "{}\0{}\0{}",
            chain.metadata,
            base.display(),
            serde_json::to_string(&chain.metadata_json).unwrap_or_default()
        )
        .as_bytes(),
    )
    .to_hex()[..24]
        .to_string();
    let out = cache.join(format!("read-{version}"));
    let done = out.join(&chain.metadata);
    if done.is_file() {
        return Ok(done);
    }
    if chain
        .files
        .iter()
        .any(|(_, content, format)| *content != 0 && format == "PUFFIN")
    {
        return Err(refused(
            path,
            "it has deletion vectors (Iceberg v3) and was written in another folder, which is not read yet",
        ));
    }
    let tmp = cache.join(format!("tmp-{version}"));
    let _ = std::fs::remove_dir_all(&tmp);
    let deletes: BTreeSet<String> = chain
        .files
        .iter()
        .filter(|(_, content, _)| *content == 1)
        .map(|(p, _, _)| p.clone())
        .collect();
    let rewritten: BTreeSet<String> = chain
        .lists
        .iter()
        .chain(chain.manifests.iter())
        .cloned()
        .chain(deletes.iter().cloned())
        .chain(std::iter::once(chain.metadata.clone()))
        .collect();
    let out_text = out.to_string_lossy().into_owned();
    let base_text = base.to_string_lossy().into_owned();
    let map = |rel: &str| -> Result<String, String> {
        if rel.is_empty() {
            return Ok(base_text.clone());
        }
        Ok(if rewritten.contains(rel) {
            format!("{out_text}/{rel}")
        } else {
            format!("{base_text}/{rel}")
        })
    };
    let fail = |e: String| refused(path, e);
    let write = |rel: &str, bytes: &[u8]| -> Result<(), SessionError> {
        let file = tmp.join(rel);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::write(file, bytes).map_err(io)
    };
    for rel in &deletes {
        let file = tmp.join(rel);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        rewrite_deletes(&base.join(rel), &file, &chain.location, &map).map_err(fail)?;
    }
    let mut sizes = HashMap::new();
    for rel in &chain.manifests {
        let bytes = rewrite_avro(
            &read_local(base, rel)?,
            &chain.location,
            &map,
            &HashMap::new(),
        )
        .map_err(fail)?;
        sizes.insert(format!("{out_text}/{rel}"), bytes.len() as i64);
        write(rel, &bytes)?;
    }
    for rel in &chain.lists {
        let bytes =
            rewrite_avro(&read_local(base, rel)?, &chain.location, &map, &sizes).map_err(fail)?;
        write(rel, &bytes)?;
    }
    let mut json = chain.metadata_json.clone();
    map_json(&mut json, &chain.location, &map).map_err(fail)?;
    write(&chain.metadata, &serde_json::to_vec(&json).map_err(io)?)?;
    // Earlier versions' rewrites are not read any more.
    if let Ok(entries) = std::fs::read_dir(cache) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("read-") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    std::fs::rename(&tmp, &out).map_err(io)?;
    Ok(done)
}

// ---------------------------------------------------------------- copying

/// A file of a dataset as it was copied or read: its path within the
/// dataset, its size and its BLAKE3 hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedFile {
    pub path: String,
    pub size: u64,
    pub blake3: String,
}

fn hash_file(file: &Path) -> Result<(u64, String), SessionError> {
    let mut hasher = blake3::Hasher::new();
    let mut opened = std::fs::File::open(file).map_err(io)?;
    let size = std::io::copy(&mut opened, &mut hasher).map_err(io)?;
    Ok((size, hasher.finalize().to_hex().to_string()))
}

/// Each of `files` under `base`, hashed.
pub fn hash_files(base: &Path, files: &[String]) -> Result<Vec<HashedFile>, SessionError> {
    files
        .iter()
        .map(|rel| {
            let (size, blake3) = hash_file(&base.join(rel))?;
            Ok(HashedFile {
                path: rel.clone(),
                size,
                blake3,
            })
        })
        .collect()
}

/// Copy every file of the dataset `path` of `place` (each of its versions)
/// into `dest`, which must not exist yet; what was copied. The copy is made
/// beside `dest` and moved there once whole.
pub async fn copy_whole(
    place: &Place,
    path: &str,
    dest: &Path,
) -> Result<Vec<HashedFile>, SessionError> {
    if dest.exists() {
        return Err(io(format!("{} exists already", dest.display())));
    }
    let tmp = dest.with_extension("oikome-copy");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(io)?;
    let copied = async {
        let mut out = Vec::new();
        match place.local_file(path) {
            Some(dir) => {
                let mut folders = vec![dir.clone()];
                while let Some(folder) = folders.pop() {
                    for entry in std::fs::read_dir(&folder).map_err(io)?.flatten() {
                        let file = entry.path();
                        // Not followed: a link is not the dataset's own.
                        let kind = entry.file_type().map_err(io)?;
                        if kind.is_dir() {
                            folders.push(file);
                            continue;
                        }
                        if !kind.is_file() {
                            continue;
                        }
                        let rel = file
                            .strip_prefix(&dir)
                            .map_err(io)?
                            .to_string_lossy()
                            .replace('\\', "/");
                        let target = tmp.join(&rel);
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent).map_err(io)?;
                        }
                        std::fs::copy(&file, &target).map_err(io)?;
                        let (size, blake3) = hash_file(&target)?;
                        out.push(HashedFile {
                            path: rel,
                            size,
                            blake3,
                        });
                    }
                }
            }
            None => {
                let listed = list_remote(place, path).await?;
                for rel in listed.keys() {
                    let bytes = place
                        .read(&format!("{path}/{rel}"), 0..u64::MAX, MAX_FETCH_BYTES)
                        .await?;
                    let target = tmp.join(rel);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(io)?;
                    }
                    std::fs::write(&target, &bytes).map_err(io)?;
                    out.push(HashedFile {
                        path: rel.clone(),
                        size: bytes.len() as u64,
                        blake3: blake3::hash(&bytes).to_hex().to_string(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok::<_, SessionError>(out)
    }
    .await;
    match copied {
        Ok(out) => {
            std::fs::rename(&tmp, dest).map_err(io)?;
            Ok(out)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- fetching

/// Every file under the dataset `path` of `place`, relative to it, with its
/// size and time.
async fn list_remote(
    place: &Place,
    path: &str,
) -> Result<BTreeMap<String, (u64, i64)>, SessionError> {
    let (on, rest) = place.route(path);
    let Place::Remote { store, label } = on else {
        return Err(io(format!("{path} is not on another place")));
    };
    let mut out = BTreeMap::new();
    let mut folders = vec![String::new()];
    while let Some(rel) = folders.pop() {
        let dir = if rel.is_empty() {
            rest.to_string()
        } else {
            format!("{rest}/{rel}")
        };
        let entries = store
            .list_owned(&dir)
            .await
            .map_err(|_| inplace::offline(label))?;
        for entry in entries {
            let child = if rel.is_empty() {
                entry.name.clone()
            } else {
                format!("{rel}/{}", entry.name)
            };
            if entry.is_dir {
                folders.push(child);
            } else {
                out.insert(child, (entry.size, entry.modified_ms));
            }
            if out.len() > inplace::MAX_FILES {
                return Err(refused(path, "it has too many files"));
            }
        }
    }
    Ok(out)
}

/// Fetch `wanted` (relative to the dataset `path`) into `mirror`, unless a
/// file of the same size is there already.
async fn fetch(
    place: &Place,
    path: &str,
    listed: &BTreeMap<String, (u64, i64)>,
    wanted: &[String],
    mirror: &Path,
) -> Result<(), SessionError> {
    let total: u64 = wanted
        .iter()
        .filter_map(|rel| listed.get(rel))
        .map(|(size, _)| size)
        .sum();
    if total > MAX_FETCH_BYTES {
        return Err(refused(
            path,
            format!(
                "the files of its version are larger than the {} GB kept for another place's files",
                MAX_FETCH_BYTES >> 30
            ),
        ));
    }
    for rel in wanted {
        let Some((size, _)) = listed.get(rel) else {
            return Err(refused(path, format!("{rel} is not in its folder")));
        };
        let target = mirror.join(rel);
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == *size) {
            continue;
        }
        let bytes = place
            .read(&format!("{path}/{rel}"), 0..u64::MAX, MAX_FETCH_BYTES)
            .await?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = target.with_extension("oikome-tmp");
        std::fs::write(&tmp, &bytes).map_err(io)?;
        std::fs::rename(&tmp, &target).map_err(io)?;
    }
    Ok(())
}

/// Remove the files under `mirror` that `consider` covers and `keep` does
/// not name.
fn prune(
    mirror: &Path,
    keep: &BTreeSet<String>,
    consider: impl Fn(&str) -> bool,
) -> Result<(), SessionError> {
    let mut folders = vec![mirror.to_path_buf()];
    while let Some(dir) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                folders.push(file);
                continue;
            }
            let Ok(rel) = file.strip_prefix(mirror) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if consider(&rel) && !keep.contains(&rel) {
                let _ = std::fs::remove_file(&file);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasets_are_found_by_their_marks() {
        assert_eq!(
            root_of("lab/runs.lance/_versions/3.manifest"),
            Some(("lab/runs.lance", DatasetKind::Lance))
        );
        assert_eq!(
            root_of("shop/orders/metadata/v2.metadata.json"),
            Some(("shop/orders", DatasetKind::Iceberg))
        );
        assert_eq!(root_of("orders/metadata/x.avro"), None);
        assert_eq!(root_of("_versions/1.manifest"), None);
        assert_eq!(root_of("metadata/v1.metadata.json"), None);
        let found = roots(
            [
                "a/t/metadata/v1.metadata.json",
                "a/t/data/inner/_versions/1.manifest",
                "b/x.csv",
            ]
            .into_iter(),
        );
        assert_eq!(
            found,
            BTreeMap::from([("a/t".to_string(), DatasetKind::Iceberg)])
        );
    }

    #[test]
    fn lance_manifest_names_give_versions() {
        assert_eq!(manifest_version("12.manifest"), Some(12));
        let inverted = format!("{:020}.manifest", u64::MAX - 7);
        assert_eq!(manifest_version(&inverted), Some(7));
        assert_eq!(manifest_version("x.manifest"), None);
    }

    #[test]
    fn the_current_iceberg_metadata_is_the_hinted_or_newest() {
        let names: Vec<String> = [
            "v1.metadata.json",
            "v2.metadata.json",
            "v10.metadata.json",
            "snap-1.avro",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            current_metadata(&names, None).as_deref(),
            Some("v10.metadata.json")
        );
        assert_eq!(
            current_metadata(&names, Some("2\n")).as_deref(),
            Some("v2.metadata.json")
        );
        let java: Vec<String> = ["00001-1b7e.metadata.json", "00002-9a3f.metadata.json"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            current_metadata(&java, None).as_deref(),
            Some("00002-9a3f.metadata.json")
        );
    }

    #[test]
    fn paths_outside_a_table_are_refused() {
        let at = "file:///data/shop/orders";
        assert_eq!(
            relative(at, "/data/shop/orders/data/a.parquet").unwrap(),
            "data/a.parquet"
        );
        assert_eq!(
            relative(at, "file:/data/shop/orders/metadata/snap.avro").unwrap(),
            "metadata/snap.avro"
        );
        for outside in [
            "/data/shop/other/a.parquet",
            "/data/shop/orders/../secret.parquet",
            "/data/shop/ordersX/a.parquet",
            "/data/shop/orders",
            "/data/shop/orders//a.parquet",
        ] {
            assert!(relative(at, outside).is_err(), "{outside}");
        }
    }
}
