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

//! Spaces on a storage (architecture v3 §7.5): a NAS at home reached over
//! WebDAV or mounted as a folder, or a cloud drive's sync folder.
//!
//! The writing node keeps the Space on its own disk as usual and, after
//! each write, pushes the new snapshot to the Space's folder on the
//! storage. Other nodes bind the folder read-only and pull the newest
//! complete snapshot into a local copy, which is read like a subscription
//! (§5.3), also while the writer is offline.
//!
//! A plain folder holds:
//! - `oikome-space.json`: the node that writes the folder, and the format;
//! - `snapshots/<seq>.json`: one snapshot, its tables and their files,
//!   written after the files it names;
//! - `files/<blake3>.parquet`: Iceberg data files;
//! - `lance/<table>.lance/...`: Lance datasets.
//!
//! An encrypted folder (always on a cloud drive) holds the same, sealed
//! with the storage's key (`oikome_vfs::seal`): `oikome-space.sealed`,
//! `snapshots/<seq>.sealed` and `objects/<id>`, where the id is a keyed
//! hash of the file's plain path, so the storage sees neither content nor
//! names.
//!
//! The writer keeps the last [`KEEP_SNAPSHOTS`] snapshots and the files
//! they name. The writer's DID in the folder's record guards against two
//! nodes writing one folder by mistake; it is not access control: anyone
//! the storage lets in can change the files. Passwords, tokens and keys
//! come from the node's secrets ([`crate::secrets`]) and are never written
//! to the storage or the registry.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{ArrayRef, Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use oikome_catalog::lance::{lance_tables, DATASET_SUFFIX};
use oikome_catalog::{
    Location, LocationFormat, StorageBinding, StorageDef, StorageKind, OBJECT_STORAGE_FOLDER,
};
use oikome_core::AccessMode;
use oikome_engine::EngineError;
use oikome_identity::unix_now_ms;
use oikome_network::protocol::{
    is_file_hash, is_identifier, is_lance_file_path, FileEntry, SnapshotResponse, TableFormat,
    TableSnapshot,
};
use oikome_network::ChunkStream;
use oikome_vfs::seal::{open_bytes, open_stream, seal_bytes, seal_stream, SealError};
use oikome_vfs::{
    file_chunks, sealed_size, Bucket, Credentials, GoogleClient, RemoteStore, SealKey, StoreError,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::error::SessionError;
use crate::lance::lance_root;
use crate::result::QueryResult;
use crate::secrets::SecretKind;
use crate::session::OikomeSession;
use crate::snapshot::table_snapshot;
use crate::subscribe::{lock_location, refresh, RefreshReport, RefreshedTable, Source};

const SNAPSHOTS_DIR: &str = "snapshots";
const FILES_DIR: &str = "files";
const LANCE_DIR: &str = "lance";
const OBJECTS_DIR: &str = "objects";
/// Snapshots kept on a storage: the newest and the one before it, so a
/// reader copying the previous one can finish.
const KEEP_SNAPSHOTS: usize = 2;
/// Version of the folder layout this node writes and reads.
const LAYOUT_VERSION: u32 = 1;
/// Largest record read.
const MAX_RECORD_BYTES: u64 = 64 << 10;
/// Largest snapshot file read (some 100,000 files).
const MAX_SNAPSHOT_BYTES: u64 = 16 << 20;

/// Looks a secret up by environment variable name.
type SecretLookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Environment overrides of storages' credentials
/// (`OIKOME_STORAGE_<NAME>_PASSWORD`, `_TOKEN`, `_KEY`): a lookup by
/// variable name, the process environment by default.
#[derive(Clone)]
pub struct StorageSecrets(Arc<SecretLookup>);

impl StorageSecrets {
    /// Look secrets up with `lookup` instead of the environment.
    pub fn new(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(lookup))
    }

    /// The process environment.
    pub fn from_env() -> Self {
        Self::new(|name| std::env::var(name).ok())
    }

    pub(crate) fn get(&self, name: &str) -> Option<Zeroizing<String>> {
        (self.0)(name).filter(|v| !v.is_empty()).map(Zeroizing::new)
    }
}

impl Default for StorageSecrets {
    fn default() -> Self {
        Self::from_env()
    }
}

impl std::fmt::Debug for StorageSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StorageSecrets")
    }
}

/// What a person gives for a storage, from the apps or the command line.
/// Values never appear in `Debug`.
#[derive(Default)]
pub struct StorageCredentials {
    pub password: Option<Zeroizing<String>>,
    pub token: Option<Zeroizing<String>>,
    /// The key of an encrypted storage another node made (`export_storage_key`
    /// there); a new one is made when this is `None`.
    pub key: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for StorageCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |v: &Option<Zeroizing<String>>| v.as_ref().map(|_| "<redacted>");
        f.debug_struct("StorageCredentials")
            .field("password", &shown(&self.password))
            .field("token", &shown(&self.token))
            .field("key", &shown(&self.key))
            .finish()
    }
}

/// The folder's record: who writes it and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SpaceRecord {
    version: u32,
    /// DID of the writing node.
    writer: String,
    /// The writer's Location id.
    location: String,
    format: TableFormat,
    created_at_ms: u64,
}

/// `snapshots/<seq>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredSnapshot {
    seq: u64,
    created_at_ms: u64,
    tables: Vec<TableSnapshot>,
}

/// Where `file` of `table` lives in the Space's folder, before any
/// encryption renames it.
fn plain_path(table: &TableSnapshot, file: &FileEntry) -> Result<String, String> {
    if !is_identifier(&table.name) || !is_file_hash(&file.hash) {
        return Err(format!("invalid file in table '{}'", table.name));
    }
    match table.format {
        TableFormat::Iceberg => Ok(format!("{FILES_DIR}/{}.parquet", file.hash)),
        TableFormat::Lance => match file.path.as_deref() {
            Some(path) if is_lance_file_path(path) => {
                Ok(format!("{LANCE_DIR}/{}{DATASET_SUFFIX}/{path}", table.name))
            }
            _ => Err(format!("invalid file in Lance table '{}'", table.name)),
        },
    }
}

/// How a folder names and stores its files: as they are, or sealed with the
/// storage's key.
#[derive(Clone)]
enum Layout {
    Plain,
    Sealed(Arc<SealKey>),
}

impl Layout {
    fn record(&self) -> &'static str {
        match self {
            Layout::Plain => "oikome-space.json",
            Layout::Sealed(_) => "oikome-space.sealed",
        }
    }

    fn other_record(&self) -> &'static str {
        match self {
            Layout::Plain => "oikome-space.sealed",
            Layout::Sealed(_) => "oikome-space.json",
        }
    }

    fn suffix(&self) -> &'static str {
        match self {
            Layout::Plain => ".json",
            Layout::Sealed(_) => ".sealed",
        }
    }

    fn snapshot(&self, seq: u64) -> String {
        format!("{SNAPSHOTS_DIR}/{seq:020}{}", self.suffix())
    }

    /// The name on the storage of the data file at `plain`.
    fn data(&self, plain: &str) -> String {
        match self {
            Layout::Plain => plain.to_string(),
            Layout::Sealed(key) => format!("{OBJECTS_DIR}/{}", key.object_name(plain)),
        }
    }

    fn data_dirs(&self) -> &'static [&'static str] {
        match self {
            Layout::Plain => &[FILES_DIR, LANCE_DIR],
            Layout::Sealed(_) => &[OBJECTS_DIR],
        }
    }
}

fn seal_message(e: SealError) -> String {
    match e {
        SealError::WrongKey => {
            "it was encrypted with another key; add the storage with the key of the node \
             that wrote it"
                .to_string()
        }
        e => e.to_string(),
    }
}

/// The folder of one Space on its storage.
#[derive(Clone)]
pub(crate) struct Folder {
    store: RemoteStore,
    binding: StorageBinding,
    layout: Layout,
}

impl Folder {
    fn err(&self, e: impl std::fmt::Display) -> SessionError {
        SessionError::Store {
            storage: self.binding.storage.clone(),
            message: e.to_string(),
        }
    }

    /// A small file, opened; `None` if it is not there.
    async fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, SessionError> {
        let limit = match self.layout {
            Layout::Plain => max,
            Layout::Sealed(_) => sealed_size(max),
        };
        let bytes = match self.store.read(name, limit).await {
            Ok(bytes) => bytes,
            Err(StoreError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(self.err(e)),
        };
        match &self.layout {
            Layout::Plain => Ok(Some(bytes.to_vec())),
            Layout::Sealed(key) => open_bytes(key, name, &bytes)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {}", seal_message(e)))),
        }
    }

    /// Write a small file, sealing it if the folder is encrypted.
    async fn write(&self, name: &str, plain: Vec<u8>) -> Result<(), SessionError> {
        let bytes = match &self.layout {
            Layout::Plain => plain,
            Layout::Sealed(key) => {
                seal_bytes(key, name, &plain).map_err(|e| self.err(seal_message(e)))?
            }
        };
        self.store
            .write(name, Bytes::from(bytes))
            .await
            .map_err(|e| self.err(e))
    }

    async fn read_record(&self) -> Result<Option<SpaceRecord>, SessionError> {
        let Some(bytes) = self.read(self.layout.record(), MAX_RECORD_BYTES).await? else {
            // Registered one way, written the other.
            if self
                .store
                .size(self.layout.other_record())
                .await
                .map_err(|e| self.err(e))?
                .is_some()
            {
                return Err(self.err(match self.layout {
                    Layout::Plain => format!(
                        "folder '{}' is encrypted; add the storage with encryption and its key",
                        self.binding.path
                    ),
                    Layout::Sealed(_) => format!(
                        "folder '{}' is not encrypted; add the storage without encryption",
                        self.binding.path
                    ),
                }));
            }
            return Ok(None);
        };
        let record: SpaceRecord = serde_json::from_slice(&bytes)
            .map_err(|e| self.err(format!("{}: {e}", self.layout.record())))?;
        if record.version > LAYOUT_VERSION {
            return Err(self.err(format!(
                "folder '{}' was written by a newer Oikome (layout {})",
                self.binding.path, record.version
            )));
        }
        Ok(Some(record))
    }

    async fn write_record(&self, record: &SpaceRecord) -> Result<(), SessionError> {
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|e| self.err(format!("{}: {e}", self.layout.record())))?;
        self.write(self.layout.record(), bytes).await
    }

    /// Sequence numbers of the snapshots, oldest first.
    async fn snapshots(&self) -> Result<Vec<u64>, SessionError> {
        let suffix = self.layout.suffix();
        let mut seqs: Vec<u64> = self
            .store
            .list(SNAPSHOTS_DIR)
            .await
            .map_err(|e| self.err(e))?
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| e.name.strip_suffix(suffix)?.parse().ok())
            .collect();
        seqs.sort_unstable();
        Ok(seqs)
    }

    async fn read_snapshot(&self, seq: u64) -> Result<Option<StoredSnapshot>, SessionError> {
        let Some(bytes) = self
            .read(&self.layout.snapshot(seq), MAX_SNAPSHOT_BYTES)
            .await?
        else {
            return Ok(None);
        };
        let snapshot: StoredSnapshot =
            serde_json::from_slice(&bytes).map_err(|e| self.err(format!("snapshot {seq}: {e}")))?;
        if snapshot.seq != seq {
            return Err(self.err(format!("snapshot {seq} is misnamed")));
        }
        Ok(Some(snapshot))
    }

    async fn write_snapshot(&self, snapshot: &StoredSnapshot) -> Result<(), SessionError> {
        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| self.err(format!("snapshot {}: {e}", snapshot.seq)))?;
        self.write(&self.layout.snapshot(snapshot.seq), bytes).await
    }

    /// The name on the storage of `file` of `table`.
    fn data_name(&self, table: &TableSnapshot, file: &FileEntry) -> Result<String, SessionError> {
        plain_path(table, file)
            .map(|p| self.layout.data(&p))
            .map_err(|m| self.err(m))
    }

    /// The names of the data files on the storage.
    async fn data_present(&self) -> Result<HashSet<String>, SessionError> {
        let mut found = HashSet::new();
        let mut pending: Vec<String> = self
            .layout
            .data_dirs()
            .iter()
            .map(|d| d.to_string())
            .collect();
        while let Some(dir) = pending.pop() {
            for entry in self.store.list(&dir).await.map_err(|e| self.err(e))? {
                let path = format!("{dir}/{}", entry.name);
                if entry.is_dir {
                    pending.push(path);
                } else {
                    found.insert(path);
                }
            }
        }
        Ok(found)
    }

    /// Upload the local file `local` as `name`. Returns the bytes sent.
    async fn upload(&self, local: &Path, name: &str) -> Result<u64, SessionError> {
        match &self.layout {
            Layout::Plain => self.store.upload(local, name).await,
            Layout::Sealed(key) => {
                let (size, chunks) = file_chunks(local).await.map_err(|e| self.err(e))?;
                let sealed = seal_stream(key.clone(), name.to_string(), chunks);
                self.store
                    .upload_stream(name, sealed_size(size), sealed)
                    .await
            }
        }
        .map_err(|e| self.err(e))
    }

    /// The bytes of the data file `name`, opened; `Ok(None)` if it is not
    /// on the storage.
    async fn open(&self, name: &str) -> Result<Option<ChunkStream>, SessionError> {
        // A reader may fail only once the download started; ask first, so
        // a missing file is told apart from a broken connection.
        if self
            .store
            .size(name)
            .await
            .map_err(|e| self.err(e))?
            .is_none()
        {
            return Ok(None);
        }
        let chunks = match self.store.reader(name).await {
            Ok(chunks) => chunks,
            Err(StoreError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(self.err(e)),
        };
        let storage = self.binding.storage.clone();
        let remote = move |m: String| EngineError::Remote(format!("storage '{storage}': {m}"));
        Ok(Some(match &self.layout {
            Layout::Plain => chunks.map_err(move |e| remote(e.to_string())).boxed(),
            Layout::Sealed(key) => open_stream(key.clone(), name.to_string(), chunks)
                .map_err(move |e| remote(seal_message(e)))
                .boxed(),
        }))
    }

    /// Delete the snapshots older than the [`KEEP_SNAPSHOTS`] newest, then
    /// the files no kept snapshot names, including temporary files left by
    /// interrupted uploads. `seqs` are the snapshots before `kept`'s newest
    /// was written; `kept` the snapshots kept.
    async fn prune(&self, seqs: &[u64], kept: &[&StoredSnapshot]) -> Result<(), SessionError> {
        let keep_from = (seqs.len() + 1).saturating_sub(KEEP_SNAPSHOTS);
        for &seq in &seqs[..keep_from.min(seqs.len())] {
            self.store
                .delete(&self.layout.snapshot(seq))
                .await
                .map_err(|e| self.err(e))?;
        }
        let mut keep = HashSet::new();
        for snapshot in kept {
            for table in &snapshot.tables {
                for file in &table.files {
                    keep.insert(self.data_name(table, file)?);
                }
            }
        }
        for name in self.data_present().await? {
            if !keep.contains(&name) {
                self.store.delete(&name).await.map_err(|e| self.err(e))?;
            }
        }
        // Datasets of dropped tables, now empty.
        if let Layout::Plain = self.layout {
            for dataset in self.store.list(LANCE_DIR).await.map_err(|e| self.err(e))? {
                let dir = format!("{LANCE_DIR}/{}", dataset.name);
                if !keep.iter().any(|k| k.starts_with(&format!("{dir}/"))) {
                    self.store.delete_dir(&dir).await.map_err(|e| self.err(e))?;
                }
            }
        }
        for entry in self
            .store
            .list(SNAPSHOTS_DIR)
            .await
            .map_err(|e| self.err(e))?
        {
            if entry.name.starts_with(".tmp-") {
                self.store
                    .delete(&format!("{SNAPSHOTS_DIR}/{}", entry.name))
                    .await
                    .map_err(|e| self.err(e))?;
            }
        }
        Ok(())
    }
}

/// One table of a [`PushReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedTable {
    pub table: String,
    pub snapshot_id: Option<i64>,
    pub files_uploaded: u64,
    pub bytes_uploaded: u64,
}

/// What a push to a storage did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushReport {
    pub space: String,
    /// The snapshot written, or `None` when the storage already described
    /// the current one (missing files may still have been uploaded).
    pub seq: Option<u64>,
    pub tables: Vec<PushedTable>,
}

impl PushReport {
    /// The report as a result set:
    /// `(table, snapshot_id, files_uploaded, bytes_uploaded, status)`, with
    /// status `uploaded` or `unchanged`.
    pub fn into_result(self) -> Result<QueryResult, SessionError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("table", DataType::Utf8, false),
            Field::new("snapshot_id", DataType::Int64, true),
            Field::new("files_uploaded", DataType::Int64, false),
            Field::new("bytes_uploaded", DataType::Int64, false),
            Field::new("status", DataType::Utf8, false),
        ]));
        let t = &self.tables;
        let status = if self.seq.is_some() || t.iter().any(|r| r.files_uploaded > 0) {
            "uploaded"
        } else {
            "unchanged"
        };
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from_iter_values(t.iter().map(|r| &r.table))),
            Arc::new(Int64Array::from_iter(t.iter().map(|r| r.snapshot_id))),
            Arc::new(Int64Array::from_iter_values(
                t.iter().map(|r| r.files_uploaded as i64),
            )),
            Arc::new(Int64Array::from_iter_values(
                t.iter().map(|r| r.bytes_uploaded as i64),
            )),
            Arc::new(StringArray::from_iter_values(t.iter().map(|_| status))),
        ];
        let batch = arrow_array::RecordBatch::try_new(schema.clone(), columns)?;
        Ok(QueryResult::Batches {
            schema,
            batches: vec![batch],
        })
    }
}

/// The files of a snapshot on a storage, read by a refresh
/// ([`Source::Storage`]). Remembers whether a file was missing, so the
/// caller can fall back to an older snapshot.
pub(crate) struct StoredFiles {
    folder: Folder,
    missing: AtomicBool,
}

impl StoredFiles {
    pub(crate) async fn open(
        &self,
        table: &TableSnapshot,
        file: &FileEntry,
    ) -> Result<ChunkStream, SessionError> {
        let name = self.folder.data_name(table, file)?;
        match self.folder.open(&name).await? {
            Some(chunks) => Ok(chunks),
            None => {
                self.missing.store(true, Ordering::Relaxed);
                Err(self.folder.err(format!("{name} is missing")))
            }
        }
    }
}

/// Where the owner's Google app is kept in the node's secret store.
const GOOGLE_APP_ID: &str = "google/app/client-id";
const GOOGLE_APP_SECRET: &str = "google/app/client-secret";

fn store_error(storage: &str, e: impl std::fmt::Display) -> SessionError {
    SessionError::Store {
        storage: storage.to_string(),
        message: e.to_string(),
    }
}

/// The local tables of the writing Space and, for Lance, its datasets'
/// directory.
type LocalTables = (Vec<String>, Option<PathBuf>);

impl OikomeSession {
    /// The folder `binding` names, with the storage's credentials and, for
    /// an encrypted storage, its key.
    fn open_folder(&self, binding: &StorageBinding) -> Result<Folder, SessionError> {
        let def = self.catalog.get_storage(&binding.storage)?;
        let err = |e: StoreError| store_error(&def.name, e);
        let secret = |kind| self.engines().storage_secret(&def.name, kind);
        let store = match &def.kind {
            StorageKind::Webdav { endpoint, username } => {
                if def.kind.needs_allow_http() && !self.engines().allow_http() {
                    return Err(store_error(
                        &def.name,
                        "it is reached over plaintext http://, which sends its password \
                         unencrypted; use https:// or allow it with --allow-http",
                    ));
                }
                let credentials = Credentials {
                    username: username.clone(),
                    password: secret(SecretKind::Password)?,
                    token: secret(SecretKind::Token)?,
                };
                RemoteStore::webdav(endpoint, &binding.path, credentials).map_err(err)?
            }
            StorageKind::Folder { path } | StorageKind::Drive { path } => {
                RemoteStore::folder(Path::new(path), &binding.path).map_err(err)?
            }
            StorageKind::S3 {
                endpoint,
                bucket,
                access_key_id,
            } => {
                if def.kind.needs_allow_http() && !self.engines().allow_http() {
                    return Err(store_error(
                        &def.name,
                        "it is reached over plaintext http://, which sends its keys' signatures \
                         and the files unencrypted; use https:// or allow it with --allow-http",
                    ));
                }
                let secret = secret(SecretKind::Password)?.ok_or_else(|| {
                    store_error(
                        &def.name,
                        "this node has no secret key for it; add it again with its secret key",
                    )
                })?;
                // Oikome's files go in their own folder of the bucket.
                let folder = match binding.path.as_str() {
                    "" => OBJECT_STORAGE_FOLDER.to_string(),
                    p => format!("{OBJECT_STORAGE_FOLDER}/{p}"),
                };
                let bucket = Bucket {
                    endpoint,
                    name: bucket,
                };
                RemoteStore::s3(bucket, &folder, access_key_id, &secret).map_err(err)?
            }
            StorageKind::Gdrive { client_id, .. } => {
                // The owner's own Google app: its ID here, its secret and
                // the sign-in with the node's secrets.
                let client_secret = secret(SecretKind::Password)?.ok_or_else(|| {
                    store_error(
                        &def.name,
                        "this node has no secret of its Google app; connect it again",
                    )
                })?;
                let token = secret(SecretKind::Token)?.ok_or_else(|| {
                    store_error(
                        &def.name,
                        "this node isn't signed in to its Google Drive; connect it again",
                    )
                })?;
                let client = GoogleClient::new(client_id.as_str(), client_secret.as_str());
                // Oikome's files go in its own folder at the top of the drive.
                let folder = match binding.path.as_str() {
                    "" => OBJECT_STORAGE_FOLDER.to_string(),
                    p => format!("{OBJECT_STORAGE_FOLDER}/{p}"),
                };
                RemoteStore::gdrive(&folder, &client, &token).map_err(err)?
            }
        };
        let layout = if def.encrypted {
            let key = secret(SecretKind::Key)?.ok_or_else(|| {
                store_error(
                    &def.name,
                    "it is encrypted and this node has no key for it; add it again with the \
                     key exported on a node that has it",
                )
            })?;
            let key = SealKey::parse(&key).map_err(|e| store_error(&def.name, e))?;
            Layout::Sealed(Arc::new(key))
        } else {
            Layout::Plain
        };
        Ok(Folder {
            store,
            binding: binding.clone(),
            layout,
        })
    }

    /// Check that the storage `name` answers and accepts this node's
    /// credentials.
    pub async fn check_storage(&self, name: &str) -> Result<(), SessionError> {
        self.deny_principal("checking a storage")?;
        let folder = self.open_folder(&StorageBinding {
            storage: name.to_string(),
            path: String::new(),
        })?;
        folder.store.check().await.map_err(|e| folder.err(e))
    }

    /// Register the storage `name` with what a person entered for it, in
    /// the apps or on the command line: credentials go to the node's secret
    /// store (never the registry), an encrypted storage gets `credentials.key`
    /// or a new key, and the storage is checked. If anything fails, nothing
    /// is kept.
    pub async fn add_storage(
        &self,
        name: &str,
        kind: StorageKind,
        encrypted: bool,
        credentials: StorageCredentials,
    ) -> Result<StorageDef, SessionError> {
        self.deny_principal("adding a storage")?;
        if credentials.key.is_some() && !encrypted {
            return Err(SessionError::Unsupported(
                "a key is for an encrypted storage".to_string(),
            ));
        }
        let def = self.catalog.add_storage(name, kind, encrypted)?;
        let added = async {
            let backend = self.engines().secret_backend()?;
            if let Some(password) = &credentials.password {
                backend.set(&SecretKind::Password.account(name), password)?;
            }
            if let Some(token) = &credentials.token {
                backend.set(&SecretKind::Token.account(name), token)?;
            }
            if encrypted {
                let key = match &credentials.key {
                    Some(text) => SealKey::parse(text).map_err(|e| store_error(name, e))?,
                    None => SealKey::generate().map_err(|e| store_error(name, e))?,
                };
                // A key in the environment wins over the stored one anyway.
                backend.set(&SecretKind::Key.account(name), &key.export())?;
            }
            self.check_storage(name).await
        }
        .await;
        match added {
            Ok(()) => Ok(def),
            Err(e) => {
                if let Err(undo) = self.forget_storage(name) {
                    tracing::warn!(storage = %name, error = %undo, "could not undo a failed storage");
                }
                Err(e)
            }
        }
    }

    /// Replace one credential of the storage `name`.
    pub fn set_storage_secret(
        &self,
        name: &str,
        kind: SecretKind,
        value: &str,
    ) -> Result<(), SessionError> {
        self.deny_principal("setting a storage credential")?;
        let def = self.catalog.get_storage(name)?;
        if kind == SecretKind::Key {
            if !def.encrypted {
                return Err(SessionError::Unsupported(format!(
                    "storage '{name}' is not encrypted"
                )));
            }
            SealKey::parse(value).map_err(|e| store_error(name, e))?;
        }
        self.engines()
            .secret_backend()?
            .set(&kind.account(name), value)
    }

    /// The room of the storage `name` as the storage says: free and total
    /// bytes, or `None` where it doesn't say. Asked of Google Drive over the
    /// network; folders on this computer are measured by the apps.
    pub async fn storage_room(&self, name: &str) -> Result<Option<(u64, u64)>, SessionError> {
        self.deny_principal("reading a storage's room")?;
        let def = self.catalog.get_storage(name)?;
        let StorageKind::Gdrive { client_id, .. } = &def.kind else {
            return Ok(None);
        };
        let secret = |kind| self.engines().storage_secret(&def.name, kind);
        let (Some(client_secret), Some(token)) =
            (secret(SecretKind::Password)?, secret(SecretKind::Token)?)
        else {
            return Ok(None);
        };
        let client = GoogleClient::new(client_id.as_str(), client_secret.as_str());
        match client.room(&token).await {
            Ok((used, Some(limit))) => Ok(Some((limit.saturating_sub(used), limit))),
            // No limit: nothing to measure against.
            Ok((_, None)) => Ok(None),
            Err(e) => Err(store_error(name, e)),
        }
    }

    /// Remember the owner's Google app (3.4c), so connecting Google Drive
    /// again, or another one, asks for nothing: its client ID and secret go
    /// to the node's secret store, never to a file of the registry or a log.
    pub fn set_google_app(&self, client_id: &str, client_secret: &str) -> Result<(), SessionError> {
        self.deny_principal("setting the Google app")?;
        let (client_id, client_secret) = (client_id.trim(), client_secret.trim());
        if client_id.is_empty()
            || client_id.len() > 256
            || !client_id.chars().all(|c| c.is_ascii_graphic())
            || client_secret.is_empty()
        {
            return Err(SessionError::Unsupported(
                "a Google app needs its client ID and secret".to_string(),
            ));
        }
        let backend = self.engines().secret_backend()?;
        backend.set(GOOGLE_APP_ID, client_id)?;
        backend.set(GOOGLE_APP_SECRET, client_secret)
    }

    /// The owner's Google app as remembered: its client ID and secret.
    pub fn google_app(&self) -> Result<Option<(String, Zeroizing<String>)>, SessionError> {
        self.deny_principal("reading the Google app")?;
        let backend = self.engines().secret_backend()?;
        match (backend.get(GOOGLE_APP_ID)?, backend.get(GOOGLE_APP_SECRET)?) {
            (Some(id), Some(secret)) => Ok(Some((id.to_string(), secret))),
            _ => Ok(None),
        }
    }

    /// The key of the encrypted storage `name`, to add the storage on
    /// another node.
    pub fn export_storage_key(&self, name: &str) -> Result<Zeroizing<String>, SessionError> {
        self.deny_principal("exporting a storage key")?;
        if !self.catalog.get_storage(name)?.encrypted {
            return Err(SessionError::Unsupported(format!(
                "storage '{name}' is not encrypted"
            )));
        }
        self.engines()
            .storage_secret(name, SecretKind::Key)?
            .ok_or_else(|| store_error(name, "this node has no key for it"))
    }

    /// Unregister the storage `name` and forget its credentials; refused
    /// while a Space is on it. Nothing on the storage is touched.
    pub fn remove_storage(&self, name: &str) -> Result<(), SessionError> {
        self.deny_principal("removing a storage")?;
        self.forget_storage(name)
    }

    fn forget_storage(&self, name: &str) -> Result<(), SessionError> {
        self.catalog.remove_storage(name)?;
        let backend = self.engines().secret_backend()?;
        for kind in SecretKind::ALL {
            backend.delete(&kind.account(name))?;
        }
        Ok(())
    }

    /// Make this node the writer of the Space's folder on its storage: the
    /// folder must be empty or already written by this node for
    /// `location`.
    pub(crate) async fn claim_storage(
        &self,
        location: &Location,
        format: TableFormat,
    ) -> Result<(), SessionError> {
        let binding = location
            .storage
            .as_ref()
            .ok_or_else(|| SessionError::Unsupported("a location without a storage".into()))?;
        let folder = self.open_folder(binding)?;
        let did = self.engines().identity()?.did().to_string();
        let ours =
            |r: &SpaceRecord| r.writer == did && r.location == location.id && r.format == format;
        match folder.read_record().await? {
            Some(record) if ours(&record) => return Ok(()),
            Some(record) => {
                return Err(folder.err(format!(
                    "folder '{}' is written by {}; bind it with ACCESS = 'readonly'",
                    binding.path, record.writer
                )))
            }
            None => {}
        }
        if !folder
            .store
            .list("")
            .await
            .map_err(|e| folder.err(e))?
            .is_empty()
        {
            return Err(folder.err(format!(
                "folder '{}' holds files but no Oikome space; choose another PATH",
                binding.path
            )));
        }
        folder
            .write_record(&SpaceRecord {
                version: LAYOUT_VERSION,
                writer: did.clone(),
                location: location.id.clone(),
                format,
                created_at_ms: unix_now_ms(),
            })
            .await?;
        // A storage cannot create a file only if it is absent: read back, in
        // case another node claimed the folder at the same moment.
        match folder.read_record().await? {
            Some(record) if ours(&record) => Ok(()),
            _ => Err(folder.err(format!(
                "another node claimed folder '{}' first",
                binding.path
            ))),
        }
    }

    /// `REFRESH SPACE` on the writer of a Space on a storage, and the push
    /// after each write: copy the Space's current snapshot to the storage
    /// unless the storage already has it.
    pub async fn push_space(&self, name: &str) -> Result<PushReport, SessionError> {
        self.deny_principal("REFRESH SPACE")?;
        let space = self.space(name)?;
        let location = self.catalog.get_location(&space.location)?;
        let binding = match &location.storage {
            Some(binding)
                if location.format != LocationFormat::StorageCopy
                    && space.access == AccessMode::Writable =>
            {
                binding.clone()
            }
            _ => {
                return Err(SessionError::Unsupported(format!(
                    "space '{name}' does not write a space on a storage"
                )))
            }
        };
        let folder = self.open_folder(&binding)?;
        let _lock = lock_location(&self.catalog, &location.id).await?;
        let did = self.engines().identity()?.did().to_string();
        match folder.read_record().await? {
            Some(record) if record.writer == did && record.location == location.id => {}
            Some(record) => {
                return Err(folder.err(format!(
                    "folder '{}' is now written by {}; this space no longer writes it",
                    binding.path, record.writer
                )))
            }
            None => {
                return Err(folder.err(format!(
                    "folder '{}' lost its {}",
                    binding.path,
                    folder.layout.record()
                )))
            }
        }

        let (tables, root) = self.local_tables(&space).await?;
        let mut current = Vec::with_capacity(tables.len());
        let mut local = Vec::with_capacity(tables.len());
        for table in &tables {
            let (snapshot, paths) = table_snapshot(
                &self.catalog,
                self.engines().hashes(),
                &space,
                root.as_deref(),
                table,
            )
            .await?;
            current.push(snapshot);
            local.push(paths);
        }

        let seqs = folder.snapshots().await?;
        let latest = match seqs.last() {
            Some(&seq) => folder.read_snapshot(seq).await?,
            None => None,
        };
        let mut report = PushReport {
            space: name.to_string(),
            seq: None,
            tables: current
                .iter()
                .map(|t| PushedTable {
                    table: t.name.clone(),
                    snapshot_id: t.snapshot_id,
                    files_uploaded: 0,
                    bytes_uploaded: 0,
                })
                .collect(),
        };
        // Upload what the storage lacks: new files, and any a reader or a
        // person removed.
        let present = folder.data_present().await?;
        for ((table, paths), pushed) in current.iter().zip(&local).zip(&mut report.tables) {
            for (file, path) in table.files.iter().zip(paths) {
                let target = folder.data_name(table, file)?;
                if present.contains(&target) {
                    continue;
                }
                pushed.bytes_uploaded += folder.upload(path, &target).await?;
                pushed.files_uploaded += 1;
            }
        }
        if latest.as_ref().is_some_and(|l| l.tables == current) {
            return Ok(report);
        }

        let seq = seqs.last().map_or(1, |s| s + 1);
        let snapshot = StoredSnapshot {
            seq,
            created_at_ms: unix_now_ms(),
            tables: current,
        };
        folder.write_snapshot(&snapshot).await?;
        report.seq = Some(seq);

        let kept: Vec<&StoredSnapshot> = latest.iter().chain([&snapshot]).collect();
        if let Err(e) = folder.prune(&seqs, &kept).await {
            // The next push tries again.
            tracing::warn!(space = %name, error = %e, "removing old snapshots from the storage failed");
        }
        Ok(report)
    }

    /// The local tables of `space` and, for Lance, its datasets' directory.
    async fn local_tables(
        &self,
        space: &oikome_catalog::Space,
    ) -> Result<LocalTables, SessionError> {
        Ok(match lance_root(&self.catalog, space)? {
            Some(root) => (lance_tables(&root)?, Some(root)),
            None => (self.catalog.space_tables(space).await?, None),
        })
    }

    /// Copy the newest complete snapshot of a Space on a storage into the
    /// local copy of `location`. A snapshot whose files are not all on the
    /// storage (pruned meanwhile, or not synced yet) is passed over for the
    /// one before it.
    pub(crate) async fn pull(
        &self,
        location: &Location,
    ) -> Result<Vec<RefreshedTable>, SessionError> {
        let binding = location
            .storage
            .as_ref()
            .ok_or_else(|| SessionError::Unsupported("a location without a storage".into()))?;
        let folder = self.open_folder(binding)?;
        let _lock = lock_location(&self.catalog, &location.id).await?;
        let record = folder.read_record().await?.ok_or_else(|| {
            folder.err(format!("folder '{}' holds no Oikome space", binding.path))
        })?;
        let dir = self.catalog.replica_dir(&location.id);
        let files = |folder: &Folder| StoredFiles {
            folder: folder.clone(),
            missing: AtomicBool::new(false),
        };
        // The writer may prune the snapshot being copied: list again once.
        for _ in 0..2 {
            let seqs = folder.snapshots().await?;
            if seqs.is_empty() {
                let empty = files(&folder);
                return refresh(
                    &Source::Storage(&empty),
                    SnapshotResponse::default(),
                    &dir,
                    Some(record.format),
                )
                .await;
            }
            for &seq in seqs.iter().rev() {
                let Some(snapshot) = folder.read_snapshot(seq).await? else {
                    continue;
                };
                let stored = files(&folder);
                let response = SnapshotResponse {
                    tables: snapshot.tables,
                    restricted: Vec::new(),
                };
                match refresh(
                    &Source::Storage(&stored),
                    response,
                    &dir,
                    Some(record.format),
                )
                .await
                {
                    Ok(tables) => return Ok(tables),
                    Err(e) if stored.missing.load(Ordering::Relaxed) => {
                        tracing::info!(storage = %binding.storage, seq, error = %e, "snapshot incomplete; trying an older one");
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Err(folder.err(format!(
            "folder '{}' holds no complete snapshot",
            binding.path
        )))
    }

    /// `CREATE SPACE ... WITH STORAGE = '<storage>'`: claim the folder and
    /// push the first snapshot (writer), or copy the newest complete
    /// snapshot (`ACCESS = 'readonly'`). Nothing is registered if this
    /// fails.
    pub(crate) async fn create_space_on_storage(
        &self,
        request: oikome_core::CreateSpaceRequest,
    ) -> Result<QueryResult, SessionError> {
        let name = request.name.clone();
        let (_, location, _) = self.catalog.plan_space(&request)?;
        if location.format == LocationFormat::StorageCopy {
            let created = async {
                let tables = self.pull(&location).await?;
                self.catalog.create_space(request).await?;
                Ok::<_, SessionError>(tables)
            }
            .await;
            return match created {
                Ok(tables) => RefreshReport {
                    space: name,
                    tables,
                }
                .into_result(),
                Err(e) => {
                    let _ = self.remove_replica(&location.id).await;
                    Err(e)
                }
            };
        }
        let format = match location.format {
            LocationFormat::Lance { .. } => TableFormat::Lance,
            _ => TableFormat::Iceberg,
        };
        self.claim_storage(&location, format).await?;
        self.catalog.create_space(request).await?;
        if let Err(e) = self.push_space(&name).await {
            tracing::warn!(space = %name, error = %e, "first push to the storage failed; the next write retries");
        }
        Ok(QueryResult::Empty)
    }

    /// After a write to `space`: push its new snapshot if it writes a Space
    /// on a storage. A failure is logged, not returned: the write is
    /// committed on this node, and the next write, `REFRESH SPACE` or the
    /// serving node's sync pushes it.
    pub(crate) async fn push_after_write(&self, space: &str) {
        let writes_storage = self.space(space).ok().is_some_and(|s| {
            s.access == AccessMode::Writable
                && self
                    .catalog
                    .get_location(&s.location)
                    .is_ok_and(|l| l.storage.is_some() && l.format != LocationFormat::StorageCopy)
        });
        if writes_storage {
            if let Err(e) = self.push_space(space).await {
                tracing::warn!(%space, error = %e, "push to the storage failed; the next write retries");
            }
        }
    }

    /// Bring every Space on a storage up to date with it: pull into copies,
    /// push from writers.
    pub async fn sync_storages(&self) {
        for space in self.catalog.list_spaces() {
            let Ok(location) = self.catalog.get_location(&space.location) else {
                continue;
            };
            if location.storage.is_none() {
                continue;
            }
            let result = if location.format == LocationFormat::StorageCopy {
                self.refresh_space(&space.name).await.map(|_| ())
            } else if space.access == AccessMode::Writable {
                self.push_space(&space.name).await.map(|_| ())
            } else {
                continue;
            };
            if let Err(e) = result {
                tracing::warn!(space = %space.name, error = %e, "storage sync failed");
            }
        }
    }
}

/// Keep every Space on a storage in step with it on a background task:
/// every `every`, copies pull the newest snapshot and writers push any
/// snapshot a failed push left behind. Abort the returned handle to stop.
pub fn spawn_storage_sync(session: Arc<OikomeSession>, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = session.catalog.refresh() {
                tracing::warn!(error = %e, "reloading the catalog failed");
            }
            session.sync_storages().await;
        }
    })
}
