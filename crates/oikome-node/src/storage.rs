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
//! WebDAV or mounted as a folder.
//!
//! The writing node keeps the Space on its own disk as usual and, after
//! each write, pushes the new snapshot to the Space's folder on the
//! storage. Other nodes bind the folder read-only and pull the newest
//! complete snapshot into a local copy, which is read like a subscription
//! (§5.3), also while the writer is offline.
//!
//! The folder holds:
//! - `oikome-space.json`: the node that writes the folder, and the format;
//! - `snapshots/<seq>.json`: one snapshot, its tables and their files,
//!   written after the files it names;
//! - `files/<blake3>.parquet`: Iceberg data files;
//! - `lance/<table>.lance/...`: Lance datasets.
//!
//! The writer keeps the last [`KEEP_SNAPSHOTS`] snapshots and the files
//! they name. The writer's DID in `oikome-space.json` guards against two
//! nodes writing one folder by mistake; it is not access control: anyone
//! the storage lets in can change the files. Passwords and tokens come
//! from the environment (`OIKOME_STORAGE_<NAME>_PASSWORD` or `_TOKEN`) and
//! are never written down.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{ArrayRef, Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use oikome_catalog::lance::{lance_tables, DATASET_SUFFIX};
use oikome_catalog::{Location, LocationFormat, StorageBinding, StorageKind};
use oikome_core::AccessMode;
use oikome_engine::EngineError;
use oikome_identity::unix_now_ms;
use oikome_network::protocol::{
    is_file_hash, is_identifier, is_lance_file_path, FileEntry, SnapshotResponse, TableFormat,
    TableSnapshot,
};
use oikome_network::ChunkStream;
use oikome_vfs::{Credentials, RemoteStore, StoreError};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::error::SessionError;
use crate::lance::lance_root;
use crate::result::QueryResult;
use crate::session::OikomeSession;
use crate::snapshot::table_snapshot;
use crate::subscribe::{lock_location, refresh, RefreshReport, RefreshedTable, Source};

const SPACE_FILE: &str = "oikome-space.json";
const SNAPSHOTS_DIR: &str = "snapshots";
const FILES_DIR: &str = "files";
const LANCE_DIR: &str = "lance";
/// Snapshots kept on a storage: the newest and the one before it, so a
/// reader copying the previous one can finish.
const KEEP_SNAPSHOTS: usize = 2;
/// Version of the folder layout this node writes and reads.
const LAYOUT_VERSION: u32 = 1;
/// Largest `oikome-space.json` read.
const MAX_RECORD_BYTES: u64 = 64 << 10;
/// Largest snapshot file read (some 100,000 files).
const MAX_SNAPSHOT_BYTES: u64 = 16 << 20;

/// Looks a secret up by environment variable name.
type SecretLookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Where storages' passwords and tokens come from: a lookup by
/// environment variable name, the process environment by default.
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

    fn get(&self, name: &str) -> Option<Zeroizing<String>> {
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

/// `oikome-space.json`: who writes the folder and how.
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

/// `snapshots/<seq>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredSnapshot {
    seq: u64,
    created_at_ms: u64,
    tables: Vec<TableSnapshot>,
}

fn snapshot_path(seq: u64) -> String {
    format!("{SNAPSHOTS_DIR}/{seq:020}.json")
}

/// Where `file` of `table` lives in the Space's folder.
fn stored_path(table: &TableSnapshot, file: &FileEntry) -> Result<String, String> {
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
    store: RemoteStore,
    storage: String,
    missing: AtomicBool,
}

impl StoredFiles {
    pub(crate) async fn open(
        &self,
        table: &TableSnapshot,
        file: &FileEntry,
    ) -> Result<ChunkStream, SessionError> {
        let path = stored_path(table, file).map_err(|m| self.error(m))?;
        // A reader may fail only once the download started; ask first, so
        // a missing file is told apart from a broken connection.
        if self
            .store
            .size(&path)
            .await
            .map_err(|e| self.fail(e))?
            .is_none()
        {
            self.missing.store(true, Ordering::Relaxed);
            return Err(self.error(format!("{path} is missing")));
        }
        let chunks = self.store.reader(&path).await.map_err(|e| self.fail(e))?;
        Ok(chunks
            .map_err(|e| EngineError::Remote(e.to_string()))
            .boxed())
    }

    fn fail(&self, e: StoreError) -> SessionError {
        if matches!(e, StoreError::NotFound(_)) {
            self.missing.store(true, Ordering::Relaxed);
        }
        self.error(e.to_string())
    }

    fn error(&self, message: String) -> SessionError {
        SessionError::Store {
            storage: self.storage.clone(),
            message,
        }
    }
}

fn store_error(binding: &StorageBinding, e: impl std::fmt::Display) -> SessionError {
    SessionError::Store {
        storage: binding.storage.clone(),
        message: e.to_string(),
    }
}

/// The local tables of the writing Space and, for Lance, its datasets'
/// directory.
type LocalTables = (Vec<String>, Option<PathBuf>);

impl OikomeSession {
    /// The folder of a Space on its storage.
    fn open_store(&self, binding: &StorageBinding) -> Result<RemoteStore, SessionError> {
        let def = self.catalog.get_storage(&binding.storage)?;
        let err = |e: StoreError| store_error(binding, e);
        match &def.kind {
            StorageKind::Webdav { endpoint, username } => {
                if def.kind.is_plaintext() && !self.engines().allow_http() {
                    return Err(store_error(
                        binding,
                        "it is reached over plaintext http://, which sends its password \
                         unencrypted; use https:// or allow it with --allow-http",
                    ));
                }
                let prefix = format!("OIKOME_STORAGE_{}", def.name.to_ascii_uppercase());
                let secrets = self.engines().storage_secrets();
                let credentials = Credentials {
                    username: username.clone(),
                    password: secrets.get(&format!("{prefix}_PASSWORD")),
                    token: secrets.get(&format!("{prefix}_TOKEN")),
                };
                RemoteStore::webdav(endpoint, &binding.path, credentials).map_err(err)
            }
            StorageKind::Folder { path } => {
                RemoteStore::folder(std::path::Path::new(path), &binding.path).map_err(err)
            }
        }
    }

    /// Check that the storage `name` answers and accepts this node's
    /// credentials.
    pub async fn check_storage(&self, name: &str) -> Result<(), SessionError> {
        self.deny_principal("checking a storage")?;
        let binding = StorageBinding {
            storage: name.to_string(),
            path: String::new(),
        };
        let store = self.open_store(&binding)?;
        store.check().await.map_err(|e| store_error(&binding, e))
    }

    async fn read_record(
        &self,
        store: &RemoteStore,
        binding: &StorageBinding,
    ) -> Result<Option<SpaceRecord>, SessionError> {
        let bytes = match store.read(SPACE_FILE, MAX_RECORD_BYTES).await {
            Ok(bytes) => bytes,
            Err(StoreError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(store_error(binding, e)),
        };
        let record: SpaceRecord = serde_json::from_slice(&bytes)
            .map_err(|e| store_error(binding, format!("{SPACE_FILE}: {e}")))?;
        if record.version > LAYOUT_VERSION {
            return Err(store_error(
                binding,
                format!(
                    "folder '{}' was written by a newer Oikome (layout {})",
                    binding.path, record.version
                ),
            ));
        }
        Ok(Some(record))
    }

    /// Sequence numbers of the snapshots on the storage, oldest first.
    async fn stored_snapshots(
        &self,
        store: &RemoteStore,
        binding: &StorageBinding,
    ) -> Result<Vec<u64>, SessionError> {
        let mut seqs: Vec<u64> = store
            .list(SNAPSHOTS_DIR)
            .await
            .map_err(|e| store_error(binding, e))?
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| e.name.strip_suffix(".json")?.parse().ok())
            .collect();
        seqs.sort_unstable();
        Ok(seqs)
    }

    async fn read_snapshot(
        &self,
        store: &RemoteStore,
        binding: &StorageBinding,
        seq: u64,
    ) -> Result<Option<StoredSnapshot>, SessionError> {
        let bytes = match store.read(&snapshot_path(seq), MAX_SNAPSHOT_BYTES).await {
            Ok(bytes) => bytes,
            Err(StoreError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(store_error(binding, e)),
        };
        let snapshot: StoredSnapshot = serde_json::from_slice(&bytes)
            .map_err(|e| store_error(binding, format!("snapshot {seq}: {e}")))?;
        if snapshot.seq != seq {
            return Err(store_error(binding, format!("snapshot {seq} is misnamed")));
        }
        Ok(Some(snapshot))
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
        let store = self.open_store(binding)?;
        let did = self.engines().identity()?.did().to_string();
        let ours =
            |r: &SpaceRecord| r.writer == did && r.location == location.id && r.format == format;
        match self.read_record(&store, binding).await? {
            Some(record) if ours(&record) => return Ok(()),
            Some(record) => {
                return Err(store_error(
                    binding,
                    format!(
                        "folder '{}' is written by {}; bind it with ACCESS = 'readonly'",
                        binding.path, record.writer
                    ),
                ))
            }
            None => {}
        }
        if !store
            .list("")
            .await
            .map_err(|e| store_error(binding, e))?
            .is_empty()
        {
            return Err(store_error(
                binding,
                format!(
                    "folder '{}' holds files but no Oikome space; choose another PATH",
                    binding.path
                ),
            ));
        }
        let record = SpaceRecord {
            version: LAYOUT_VERSION,
            writer: did.clone(),
            location: location.id.clone(),
            format,
            created_at_ms: unix_now_ms(),
        };
        let bytes = serde_json::to_vec_pretty(&record)
            .map_err(|e| store_error(binding, format!("{SPACE_FILE}: {e}")))?;
        store
            .write(SPACE_FILE, Bytes::from(bytes))
            .await
            .map_err(|e| store_error(binding, e))?;
        // WebDAV cannot create a file only if it is absent: read back, in
        // case another node claimed the folder at the same moment.
        match self.read_record(&store, binding).await? {
            Some(record) if ours(&record) => Ok(()),
            _ => Err(store_error(
                binding,
                format!("another node claimed folder '{}' first", binding.path),
            )),
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
        let store = self.open_store(&binding)?;
        let _lock = lock_location(&self.catalog, &location.id).await?;
        let did = self.engines().identity()?.did().to_string();
        match self.read_record(&store, &binding).await? {
            Some(record) if record.writer == did && record.location == location.id => {}
            Some(record) => {
                return Err(store_error(
                    &binding,
                    format!(
                        "folder '{}' is now written by {}; this space no longer writes it",
                        binding.path, record.writer
                    ),
                ))
            }
            None => {
                return Err(store_error(
                    &binding,
                    format!("folder '{}' lost its {SPACE_FILE}", binding.path),
                ))
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

        let seqs = self.stored_snapshots(&store, &binding).await?;
        let latest = match seqs.last() {
            Some(&seq) => self.read_snapshot(&store, &binding, seq).await?,
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
        let present = self.stored_files(&store, &binding).await?;
        for ((table, paths), pushed) in current.iter().zip(&local).zip(&mut report.tables) {
            for (file, path) in table.files.iter().zip(paths) {
                let target = stored_path(table, file).map_err(|m| store_error(&binding, m))?;
                if present.contains(&target) {
                    continue;
                }
                pushed.bytes_uploaded += store
                    .upload(path, &target)
                    .await
                    .map_err(|e| store_error(&binding, e))?;
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
        let bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|e| store_error(&binding, format!("snapshot {seq}: {e}")))?;
        store
            .write(&snapshot_path(seq), Bytes::from(bytes))
            .await
            .map_err(|e| store_error(&binding, e))?;
        report.seq = Some(seq);

        let kept: Vec<&StoredSnapshot> = latest.iter().chain([&snapshot]).collect();
        if let Err(e) = self.prune(&store, &binding, &seqs, &kept).await {
            // The next push tries again.
            tracing::warn!(space = %name, error = %e, "removing old snapshots from the storage failed");
        }
        Ok(report)
    }

    /// The data files on the storage: `files/*` and every file under
    /// `lance/`, as paths in the Space's folder.
    async fn stored_files(
        &self,
        store: &RemoteStore,
        binding: &StorageBinding,
    ) -> Result<HashSet<String>, SessionError> {
        let err = |e: StoreError| store_error(binding, e);
        let mut found = HashSet::new();
        let mut pending = vec![FILES_DIR.to_string(), LANCE_DIR.to_string()];
        while let Some(dir) = pending.pop() {
            for entry in store.list(&dir).await.map_err(err)? {
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

    /// Delete the snapshots older than the [`KEEP_SNAPSHOTS`] newest, then
    /// the files no kept snapshot names, including temporary files left by
    /// interrupted uploads. `kept` are the kept snapshots.
    async fn prune(
        &self,
        store: &RemoteStore,
        binding: &StorageBinding,
        seqs: &[u64],
        kept: &[&StoredSnapshot],
    ) -> Result<(), SessionError> {
        let err = |e: StoreError| store_error(binding, e);
        let keep_from = (seqs.len() + 1).saturating_sub(KEEP_SNAPSHOTS);
        for &seq in &seqs[..keep_from.min(seqs.len())] {
            store.delete(&snapshot_path(seq)).await.map_err(err)?;
        }
        let keep: HashSet<String> = kept
            .iter()
            .flat_map(|s| s.tables.iter())
            .flat_map(|t| t.files.iter().map(move |f| stored_path(t, f)))
            .filter_map(Result::ok)
            .collect();
        for path in self.stored_files(store, binding).await? {
            if !keep.contains(&path) {
                store.delete(&path).await.map_err(err)?;
            }
        }
        // Datasets of dropped tables, now empty.
        for dataset in store.list(LANCE_DIR).await.map_err(err)? {
            let dir = format!("{LANCE_DIR}/{}", dataset.name);
            if !keep.iter().any(|k| k.starts_with(&format!("{dir}/"))) {
                store.delete_dir(&dir).await.map_err(err)?;
            }
        }
        let snapshots_dir = store.list(SNAPSHOTS_DIR).await.map_err(err)?;
        for entry in snapshots_dir {
            if entry.name.starts_with(".tmp-") {
                store
                    .delete(&format!("{SNAPSHOTS_DIR}/{}", entry.name))
                    .await
                    .map_err(err)?;
            }
        }
        Ok(())
    }

    /// Copy the newest complete snapshot of a Space on a storage into the
    /// local copy of `location`. A snapshot whose files are not all on the
    /// storage (pruned meanwhile, or never uploaded) is passed over for the
    /// one before it.
    pub(crate) async fn pull(
        &self,
        location: &Location,
    ) -> Result<Vec<RefreshedTable>, SessionError> {
        let binding = location
            .storage
            .as_ref()
            .ok_or_else(|| SessionError::Unsupported("a location without a storage".into()))?;
        let store = self.open_store(binding)?;
        let _lock = lock_location(&self.catalog, &location.id).await?;
        let record = self.read_record(&store, binding).await?.ok_or_else(|| {
            store_error(
                binding,
                format!("folder '{}' holds no Oikome space", binding.path),
            )
        })?;
        let dir = self.catalog.replica_dir(&location.id);
        // The writer may prune the snapshot being copied: list again once.
        for _ in 0..2 {
            let seqs = self.stored_snapshots(&store, binding).await?;
            if seqs.is_empty() {
                let source = Source::Storage(&StoredFiles {
                    store: store.clone(),
                    storage: binding.storage.clone(),
                    missing: AtomicBool::new(false),
                });
                return refresh(
                    &source,
                    SnapshotResponse::default(),
                    &dir,
                    Some(record.format),
                )
                .await;
            }
            for &seq in seqs.iter().rev() {
                let Some(snapshot) = self.read_snapshot(&store, binding, seq).await? else {
                    continue;
                };
                let files = StoredFiles {
                    store: store.clone(),
                    storage: binding.storage.clone(),
                    missing: AtomicBool::new(false),
                };
                let response = SnapshotResponse {
                    tables: snapshot.tables,
                    restricted: Vec::new(),
                };
                match refresh(
                    &Source::Storage(&files),
                    response,
                    &dir,
                    Some(record.format),
                )
                .await
                {
                    Ok(tables) => return Ok(tables),
                    Err(e) if files.missing.load(Ordering::Relaxed) => {
                        tracing::info!(storage = %binding.storage, seq, error = %e, "snapshot incomplete; trying an older one");
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Err(store_error(
            binding,
            format!("folder '{}' holds no complete snapshot", binding.path),
        ))
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
