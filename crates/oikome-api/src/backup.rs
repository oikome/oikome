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

//! Settings › Backup: where backups go, Back Up Now and Check Backup, each
//! run in the background with its progress, and the backups kept. Backups
//! are made only when the owner asks; one a day for a week, one a week for
//! a month and one a month for a year are kept.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oikome_backup::{
    relocate, repository_password, stage, Keep, Manifest, Meter, Repository, Snapshot,
    StoreBackend, MANIFEST_FILE,
};
use oikome_catalog::{LocationFormat, OikomeCatalog, StorageKind};
use oikome_identity::{decode_recovery_key, NodeIdentity};
use oikome_node::inplace::{self, Place};
use oikome_vfs::RemoteStore;

use crate::{
    Api, ApiError, BackupDestination, BackupInfo, BackupItem, BackupProgress, BackupResult,
    BackupSource, BackupStep, BackupWhere, BackupWork, Caller, Empty, ErrorCode, MissingOriginals,
    RestoreBackup, RestoredBackup,
};

/// The folder the repository is kept in, inside the folder the owner chose.
const REPOSITORY_FOLDER: &str = "oikome-backup";

/// The job under way, if any: one at a time.
#[derive(Clone, Default)]
pub(crate) struct Job(Arc<Mutex<Option<Running>>>);

struct Running {
    work: BackupWork,
    step: BackupStep,
    meter: Arc<Meter>,
    started_at_ms: u64,
}

impl Job {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn progress(&self) -> Option<BackupProgress> {
        self.lock().as_ref().map(|r| {
            let (done, total) = r.meter.read();
            BackupProgress {
                work: r.work,
                step: r.step,
                done,
                total,
                started_at_ms: r.started_at_ms,
            }
        })
    }

    fn step(&self, step: BackupStep) {
        if let Some(running) = self.lock().as_mut() {
            running.step = step;
            running.meter.start(0);
        }
    }
}

impl From<Snapshot> for BackupItem {
    fn from(s: Snapshot) -> Self {
        BackupItem {
            id: s.id,
            made_at_ms: s.made_at_ms,
            bytes: s.bytes,
            files: s.files,
            added: s.added,
        }
    }
}

fn failed(e: impl std::fmt::Display) -> ApiError {
    ApiError::invalid(e.to_string())
}

impl Api {
    /// The repository at `destination`, made ready to use: its folder there,
    /// opened with the key derived from this node's recovery key.
    fn backup_repository(&self, destination: &BackupDestination) -> Result<Repository, ApiError> {
        let within = destination.path.trim().trim_matches('/');
        let (store, label) = match &destination.place {
            None => {
                let chosen = Path::new(destination.path.trim());
                if !chosen.is_absolute() {
                    return Err(ApiError::invalid("choose a folder on this computer"));
                }
                let chosen = chosen.canonicalize().map_err(|e| {
                    ApiError::invalid(format!("{} cannot be reached: {e}", chosen.display()))
                })?;
                let data = Path::new(self.catalog.root_path())
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(self.catalog.root_path()));
                if !chosen.is_dir() || chosen.starts_with(&data) || data.starts_with(&chosen) {
                    return Err(ApiError::invalid(
                        "choose a folder outside the data folder, such as one on another disk",
                    ));
                }
                let folder = chosen.join(REPOSITORY_FOLDER);
                std::fs::create_dir_all(&folder).map_err(failed)?;
                let store = RemoteStore::folder(&folder, "").map_err(failed)?;
                (store, folder.display().to_string())
            }
            Some(storage) => {
                let def = self.catalog.get_storage(storage)?;
                if !matches!(
                    def.kind,
                    StorageKind::Folder { .. }
                        | StorageKind::Drive { .. }
                        | StorageKind::Webdav { .. }
                        | StorageKind::S3 { .. }
                ) {
                    return Err(ApiError::invalid("backups go to a folder, a NAS or RustFS"));
                }
                let at = if within.is_empty() {
                    REPOSITORY_FOLDER.to_string()
                } else {
                    format!("{within}/{REPOSITORY_FOLDER}")
                };
                match inplace::place_on(&self.catalog, &self.engines, storage, &at)? {
                    Place::Local(folder) => {
                        std::fs::create_dir_all(&folder).map_err(failed)?;
                        let store = RemoteStore::folder(&folder, "").map_err(failed)?;
                        (store, folder.display().to_string())
                    }
                    Place::Remote { store, label } => (*store, label),
                    Place::Several { .. } => {
                        return Err(ApiError::internal("a storage is a single place"))
                    }
                }
            }
        };
        let password = self
            .engines
            .identity()
            .map_err(ApiError::internal)?
            .with_seed(repository_password);
        let backend = StoreBackend::new(Arc::new(store), tokio::runtime::Handle::current(), label);
        Ok(Repository::new(backend, password))
    }

    fn backup_where(&self, destination: &BackupDestination) -> BackupWhere {
        let label = match &destination.place {
            None => "This computer".to_string(),
            Some(storage) => self
                .settings()
                .ok()
                .and_then(|s| s.storage_labels.get(storage).cloned())
                .unwrap_or_else(|| storage.clone()),
        };
        BackupWhere {
            place: destination.place.clone(),
            path: destination.path.clone(),
            label,
        }
    }

    /// Settings › Backup: where backups go, the job under way and how the
    /// last ones ended.
    pub async fn backup_info(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        let settings = self.settings()?;
        let keep = Keep::default();
        let to_u32 = |n: i32| u32::try_from(n).unwrap_or(0);
        Ok(BackupInfo {
            destination: settings
                .backup_destination
                .as_ref()
                .map(|d| self.backup_where(d)),
            keep_daily: to_u32(keep.daily),
            keep_weekly: to_u32(keep.weekly),
            keep_monthly: to_u32(keep.monthly),
            running: self.backup_job.progress(),
            last_backup: settings.last_backup,
            last_check: settings.last_backup_check,
        })
    }

    /// Choose where backups go. The place must be reachable, and backups
    /// already there must have been made with this node's recovery key.
    pub async fn set_backup_destination(
        &self,
        caller: &Caller,
        req: BackupDestination,
    ) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let destination = BackupDestination {
            place: req.place.filter(|p| !p.trim().is_empty()),
            path: req.path.trim().to_string(),
        };
        let repo = self.backup_repository(&destination)?;
        tokio::task::spawn_blocking(move || {
            if repo.exists()? {
                repo.snapshots().map(|_| ()).map_err(|_| {
                    oikome_backup::BackupError::Repository(
                        "the backups there were made with another recovery key".into(),
                    )
                })
            } else {
                Ok(())
            }
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
        self.update_settings(|s| s.backup_destination = Some(destination))?;
        self.backup_info(caller, Empty {}).await
    }

    /// The backups kept at the backup place, the newest first.
    pub async fn list_backups(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<BackupItem>, ApiError> {
        caller.require_owner()?;
        let Some(destination) = self.settings()?.backup_destination else {
            return Ok(Vec::new());
        };
        let repo = self.backup_repository(&destination)?;
        let snapshots = tokio::task::spawn_blocking(move || {
            if repo.exists()? {
                repo.snapshots()
            } else {
                Ok(Vec::new())
            }
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
        Ok(snapshots.into_iter().map(BackupItem::from).collect())
    }

    /// Start a job: one at a time, to the place chosen.
    fn start_backup_job(
        &self,
        work: BackupWork,
        step: BackupStep,
    ) -> Result<(Repository, Arc<Meter>), ApiError> {
        let destination = self
            .settings()?
            .backup_destination
            .ok_or_else(|| ApiError::invalid("choose where backups go first"))?;
        let meter = Arc::new(Meter::default());
        let repo = self
            .backup_repository(&destination)?
            .with_meter(meter.clone());
        let mut running = self.backup_job.lock();
        if running.is_some() {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "a backup or a check is under way",
            ));
        }
        *running = Some(Running {
            work,
            step,
            meter: meter.clone(),
            started_at_ms: crate::now_ms(),
        });
        Ok((repo, meter))
    }

    fn finish_backup_job(&self, result: BackupResult) {
        let work = result.work;
        if let Err(e) = self.update_settings(|s| match work {
            BackupWork::BackUp => s.last_backup = Some(result),
            BackupWork::Check => s.last_backup_check = Some(result),
        }) {
            tracing::warn!(error = %e, "keeping how a backup ended");
        }
        *self.backup_job.lock() = None;
    }

    /// What the collections view where they are, which a backup does not
    /// hold: each collection's folder and its files, so that a restore can
    /// say which are missing.
    fn originals_for_backup(&self) -> serde_json::Value {
        let mut out = Vec::new();
        for space in self.catalog.list_spaces() {
            let Ok(location) = self.catalog.get_location(&space.location) else {
                continue;
            };
            let LocationFormat::InPlace { place, path, .. } = &location.format else {
                continue;
            };
            let files: Vec<serde_json::Value> = inplace::indexed_files(&self.catalog, &location)
                .unwrap_or_default()
                .into_iter()
                .map(|f| serde_json::json!([f.path, f.size, f.modified_ms]))
                .collect();
            let LocationFormat::InPlace { sources, .. } = &location.format else {
                continue;
            };
            // Folders added from a storage: their files cannot be looked for
            // before the storage is signed in to again.
            let elsewhere: Vec<&String> = sources
                .iter()
                .flatten()
                .filter(|s| s.place.is_some())
                .filter_map(|s| s.mount.as_ref())
                .collect();
            out.push(serde_json::json!({
                "collection": space.name,
                "place": place,
                "path": path,
                "elsewhere": elsewhere,
                "files": files,
            }));
        }
        serde_json::Value::Array(out)
    }

    /// Back up now, in the background: a consistent copy of the data
    /// folder, sent to the backup place; then the backups not kept go.
    pub async fn back_up_now(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (repo, meter) = self.start_backup_job(BackupWork::BackUp, BackupStep::Copying)?;
        let originals = self.originals_for_backup();
        let data = PathBuf::from(self.catalog.root_path());
        let staging = self
            .catalog
            .oikome_dir()
            .join("backup")
            .join(format!("staging-{}", crate::now_ms()));
        let api = self.clone();
        tokio::spawn(async move {
            let made = async {
                let copy = staging.clone();
                let counted = meter.clone();
                tokio::task::spawn_blocking(move || stage(&data, &copy, originals, Some(&counted)))
                    .await
                    .map_err(failed)?
                    .map_err(failed)?;
                api.backup_job.step(BackupStep::Sending);
                let copy = staging.clone();
                tokio::task::spawn_blocking(move || repo.back_up(&copy, &Keep::default()))
                    .await
                    .map_err(failed)?
                    .map_err(failed)
            }
            .await;
            if let Err(e) = std::fs::remove_dir_all(&staging) {
                tracing::warn!(error = %e, "removing a backup's staging folder");
            }
            api.finish_backup_job(BackupResult {
                work: BackupWork::BackUp,
                at_ms: crate::now_ms(),
                error: made.as_ref().err().map(|e| e.message.clone()),
                backup: made.ok().map(BackupItem::from),
            });
        });
        self.backup_info(caller, Empty {}).await
    }

    /// Check Backup, in the background: every file at the backup place read
    /// back and checked to be whole.
    pub async fn check_backup(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        let (repo, _) = self.start_backup_job(BackupWork::Check, BackupStep::Checking)?;
        let api = self.clone();
        tokio::spawn(async move {
            let checked = tokio::task::spawn_blocking(move || repo.check())
                .await
                .map_err(failed)
                .and_then(|r| r.map_err(failed));
            api.finish_backup_job(BackupResult {
                work: BackupWork::Check,
                at_ms: crate::now_ms(),
                error: checked.err().map(|e| e.message),
                backup: None,
            });
        });
        self.backup_info(caller, Empty {}).await
    }
}

/// Where a restore waits, checked, for Oikome's next start.
fn pending_dir(root: &Path) -> PathBuf {
    root.join(".oikome").join("backup").join("pending")
}

/// The repository at `source`: the folder chosen for backups, or the
/// `oikome-backup` folder in it.
async fn source_store(source: &BackupSource) -> Result<(RemoteStore, String), ApiError> {
    let open = |within: &str| -> Result<(RemoteStore, String), ApiError> {
        match source {
            BackupSource::Folder { path } => {
                let folder = Path::new(path.trim()).join(within);
                if !folder.is_absolute() {
                    return Err(ApiError::invalid("give the whole path of the folder"));
                }
                Ok((
                    RemoteStore::folder(&folder, "").map_err(failed)?,
                    folder.display().to_string(),
                ))
            }
            BackupSource::Rustfs {
                address,
                bucket,
                access_key,
                secret_key,
                path,
            } => {
                let folder = join_within(path, within);
                let store = RemoteStore::s3_view(
                    oikome_vfs::remote::Bucket {
                        endpoint: address.trim(),
                        name: bucket.trim(),
                    },
                    &folder,
                    access_key.trim(),
                    secret_key,
                )
                .map_err(failed)?;
                Ok((store, format!("{} / {bucket} / {folder}", address.trim())))
            }
            BackupSource::Webdav {
                address,
                username,
                password,
                path,
            } => {
                let folder = join_within(path, within);
                let credentials = oikome_vfs::remote::Credentials {
                    username: username.clone().filter(|u| !u.is_empty()),
                    password: password.clone().map(zeroize::Zeroizing::new),
                    token: None,
                };
                let store = RemoteStore::webdav_view(address.trim(), &folder, credentials)
                    .map_err(failed)?;
                Ok((store, format!("{} / {folder}", address.trim())))
            }
        }
    };
    let (store, label) = open("")?;
    if store.size("config").await.ok().flatten().is_some() {
        return Ok((store, label));
    }
    let (store, label) = open(REPOSITORY_FOLDER)?;
    if store.size("config").await.ok().flatten().is_some() {
        return Ok((store, label));
    }
    Err(ApiError::not_found(format!(
        "no backups were found at {label}; choose the folder that was chosen for backups"
    )))
}

fn join_within(path: &str, within: &str) -> String {
    let path = path.trim().trim_matches('/');
    match (path.is_empty(), within.is_empty()) {
        (true, _) => within.to_string(),
        (false, true) => path.to_string(),
        (false, false) => format!("{path}/{within}"),
    }
}

/// The originals the backup's collections view where they are, and which
/// of those on this computer are not where they were; and the storages to
/// sign in to again.
fn originals_after(manifest: &Manifest, root: &Path) -> (Vec<MissingOriginals>, Vec<String>) {
    let mut missing = Vec::new();
    let mut storages: Vec<String> = Vec::new();
    let old_root = manifest.data_root.trim_end_matches('/');
    for entry in manifest.originals.as_array().into_iter().flatten() {
        let collection = entry["collection"].as_str().unwrap_or_default().to_string();
        if let Some(place) = entry["place"].as_str() {
            if !storages.iter().any(|s| s == place) {
                storages.push(place.to_string());
            }
            continue;
        }
        let Some(folder) = entry["path"].as_str() else {
            continue;
        };
        // A folder in the data folder moved with it.
        let folder = match folder.strip_prefix(old_root) {
            Some(rest) if !old_root.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
                format!("{}{rest}", root.display())
            }
            _ => folder.to_string(),
        };
        let elsewhere: Vec<&str> = entry["elsewhere"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str())
            .collect();
        let mut gone = Vec::new();
        for file in entry["files"].as_array().into_iter().flatten() {
            let Some(path) = file[0].as_str() else {
                continue;
            };
            if elsewhere
                .iter()
                .any(|m| path == *m || path.starts_with(&format!("{m}/")))
            {
                continue;
            }
            if !Path::new(&folder).join(path).exists() {
                gone.push(path.to_string());
            }
        }
        if !gone.is_empty() {
            missing.push(MissingOriginals {
                collection,
                folder,
                missing: gone.len() as u64,
                some: gone.into_iter().take(5).collect(),
            });
        }
    }
    (missing, storages)
}

/// Make the newest backup at `source` ready for this data folder: download
/// it, check it against its manifest, and set it beside the data folder
/// with the identity the recovery key gives, to take effect when Oikome
/// next starts ([`apply_pending_restore`]). Only a data center that holds
/// nothing yet can take a backup.
pub async fn prepare_restore(
    catalog: &OikomeCatalog,
    req: RestoreBackup,
) -> Result<RestoredBackup, ApiError> {
    catalog.refresh()?;
    let busy = !catalog.list_spaces().is_empty()
        || !catalog.list_contacts().is_empty()
        || !catalog.list_issued_ucans().is_empty();
    if busy {
        return Err(ApiError::new(
            ErrorCode::Conflict,
            "this data center already holds data; restore into a new one",
        ));
    }
    let seed = decode_recovery_key(req.recovery_key.trim())
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let (store, label) = source_store(&req.source).await?;
    let backend = StoreBackend::new(Arc::new(store), tokio::runtime::Handle::current(), label);
    let repo = Repository::new(backend, repository_password(&seed));

    let root = PathBuf::from(catalog.root_path());
    let incoming = catalog
        .oikome_dir()
        .join("backup")
        .join(format!("incoming-{}", crate::now_ms()));
    let into = incoming.clone();
    let manifest = tokio::task::spawn_blocking(move || repo.restore(None, &into))
        .await
        .map_err(ApiError::internal)?;
    let manifest = match manifest {
        Ok(manifest) => manifest,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&incoming);
            return Err(match e {
                oikome_backup::BackupError::Repository(m)
                    if m.contains("password") || m.contains("key") =>
                {
                    ApiError::invalid("this recovery key does not open these backups")
                }
                other => failed(other),
            });
        }
    };
    let key = incoming
        .join(".oikome")
        .join("identity")
        .join(oikome_identity::keys::NODE_KEY_FILE);
    let identity = NodeIdentity::write_key_file(&key, &seed).map_err(ApiError::internal)?;
    let pending = pending_dir(&root);
    if pending.exists() {
        std::fs::remove_dir_all(&pending).map_err(ApiError::internal)?;
    }
    std::fs::rename(&incoming, &pending).map_err(ApiError::internal)?;
    let (missing_originals, sign_in_again) = originals_after(&manifest, &root);
    Ok(RestoredBackup {
        did: identity.did().to_string(),
        made_at_ms: manifest.made_at_ms,
        files: manifest.files.len() as u64,
        missing_originals,
        sign_in_again,
    })
}

/// Put a restore made ready by [`prepare_restore`] in place, before
/// anything opens the data folder `root`: its files replace those there
/// (with any SQLite side files of the replaced ones), and the paths kept
/// absolute move with the data folder. Whether there was one.
pub fn apply_pending_restore(root: &Path) -> Result<bool, ApiError> {
    let pending = pending_dir(root);
    if !pending.is_dir() {
        return Ok(false);
    }
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(pending.join(MANIFEST_FILE)).map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    let mut folders = vec![pending.clone()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)
            .map_err(ApiError::internal)?
            .flatten()
        {
            let from = entry.path();
            let relative = from.strip_prefix(&pending).map_err(ApiError::internal)?;
            if from.is_dir() {
                folders.push(from);
                continue;
            }
            if relative == Path::new(MANIFEST_FILE) {
                continue;
            }
            let to = root.join(relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
            }
            for side in ["-wal", "-shm", "-journal"] {
                let mut name = to.clone().into_os_string();
                name.push(side);
                let _ = std::fs::remove_file(PathBuf::from(name));
            }
            std::fs::rename(&from, &to).map_err(ApiError::internal)?;
        }
    }
    if !manifest.data_root.is_empty() {
        relocate(root, &manifest.data_root).map_err(failed)?;
    }
    std::fs::remove_dir_all(&pending).map_err(ApiError::internal)?;
    Ok(true)
}

impl Api {
    /// Restore from Backup, on a new data center: the newest backup at the
    /// place given, opened with the recovery key, made ready to take effect
    /// when Oikome is started again.
    pub async fn restore_from_backup(
        &self,
        caller: &Caller,
        req: RestoreBackup,
    ) -> Result<RestoredBackup, ApiError> {
        caller.require_owner()?;
        prepare_restore(&self.catalog, req).await
    }
}
