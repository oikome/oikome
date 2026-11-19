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

//! Settings › Backup. Backups are off until the owner turns them on with
//! where they go (one or more places: a folder on this computer or another
//! disk, a NAS, RustFS) and when (daily or weekly, at a time of this
//! computer's day). Nothing is backed up then until that time comes, or the
//! owner asks; a time missed while the node was off is made up when it runs
//! again. Each backup adds only what changed; one a day for a week, one a
//! week for a month and one a month for a year are kept.
//!
//! Restoring turns this data center back to a backup of this very data
//! folder, on this computer: it is made ready beside the data folder and
//! put in place when Oikome next starts ([`apply_pending_restore`]).
//! Restoring onto another computer or into another folder is not offered.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{Datelike, Local, NaiveTime, TimeZone};
use oikome_backup::{
    relocate, repository_password, stage, Keep, Manifest, Meter, Repository, Snapshot,
    StoreBackend, MANIFEST_FILE,
};
use oikome_catalog::{LocationFormat, StorageKind};
use oikome_identity::{decode_recovery_key, NodeIdentity};
use oikome_node::inplace::{self, Place};
use oikome_vfs::RemoteStore;
use serde::{Deserialize, Serialize};

use crate::{
    Api, ApiError, BackupDestination, BackupEvery, BackupInfo, BackupItem, BackupPlace,
    BackupPlaceRef, BackupProgress, BackupResult, BackupSchedule, BackupSettings, BackupStep,
    BackupWork, Caller, Empty, ErrorCode, MissingOriginals, RestoreBackup, RestoredBackup,
};

/// The folder the repository is kept in, inside the folder the owner chose.
const REPOSITORY_FOLDER: &str = "oikome-backup";
/// The most places backups go to at once.
const MAX_PLACES: usize = 4;

/// Backups, as `node.json` keeps them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct BackupState {
    #[serde(default)]
    enabled: bool,
    /// When backups were last turned on: a time before then is not made up.
    #[serde(default)]
    enabled_at_ms: u64,
    #[serde(default)]
    destinations: Vec<BackupDestination>,
    #[serde(default)]
    schedule: Option<BackupSchedule>,
    /// How the last backup and check at each place ended, by place.
    #[serde(default)]
    results: BTreeMap<String, PlaceResults>,
    /// When the last backup, asked for or due, started.
    #[serde(default)]
    last_started_ms: u64,
}

impl BackupState {
    pub(crate) fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct PlaceResults {
    #[serde(default)]
    backup: Option<BackupResult>,
    #[serde(default)]
    check: Option<BackupResult>,
}

fn key_of(destination: &BackupDestination) -> String {
    format!(
        "{}:{}",
        destination.place.as_deref().unwrap_or(""),
        destination.path
    )
}

/// The job under way, if any: one at a time.
#[derive(Clone, Default)]
pub(crate) struct Job(Arc<Mutex<Option<Running>>>);

struct Running {
    work: BackupWork,
    step: BackupStep,
    place: Option<String>,
    index: u32,
    of: u32,
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
                place: r.place.clone(),
                index: r.index,
                of: r.of,
                done,
                total,
                started_at_ms: r.started_at_ms,
            }
        })
    }

    fn at(&self, step: BackupStep, place: &str, index: usize) {
        if let Some(running) = self.lock().as_mut() {
            running.step = step;
            running.place = Some(place.to_string());
            running.index = u32::try_from(index + 1).unwrap_or(u32::MAX);
            running.meter.start(0);
        }
    }
}

impl BackupItem {
    fn from_snapshot(s: Snapshot, this_folder: bool) -> Self {
        BackupItem {
            id: s.id,
            made_at_ms: s.made_at_ms,
            bytes: s.bytes,
            files: s.files,
            added: s.added,
            this_folder,
        }
    }
}

fn failed(e: impl std::fmt::Display) -> ApiError {
    ApiError::invalid(e.to_string())
}

/// The latest time `schedule` names at or before `now`, and the next one
/// after it, in milliseconds, in this computer's time.
fn times_around(schedule: &BackupSchedule, now: chrono::DateTime<Local>) -> (i64, i64) {
    let at = NaiveTime::from_hms_opt(schedule.hour.into(), schedule.minute.into(), 0)
        .unwrap_or(NaiveTime::MIN);
    let millis = |date: chrono::NaiveDate| {
        Local
            .from_local_datetime(&date.and_time(at))
            .earliest()
            .map(|t| t.timestamp_millis())
    };
    let step = chrono::Duration::days(match schedule.every {
        BackupEvery::Daily => 1,
        BackupEvery::Weekly => 7,
    });
    let today = now.date_naive();
    let mut date = match schedule.every {
        BackupEvery::Daily => today,
        BackupEvery::Weekly => {
            let want = i64::from(schedule.weekday.unwrap_or(0));
            let have = i64::from(today.weekday().num_days_from_monday());
            today - chrono::Duration::days((have - want).rem_euclid(7))
        }
    };
    if millis(date).is_none_or(|t| t > now.timestamp_millis()) {
        date -= step;
    }
    (
        millis(date).unwrap_or(0),
        millis(date + step).unwrap_or(i64::MAX),
    )
}

impl Api {
    fn backup_state(&self) -> Result<BackupState, ApiError> {
        Ok(self.settings()?.backup)
    }

    /// The place `destination` names, made ready: its `oikome-backup` folder,
    /// and what the owner reads for it.
    fn backup_store(
        &self,
        destination: &BackupDestination,
    ) -> Result<(RemoteStore, String), ApiError> {
        let within = destination.path.trim().trim_matches('/');
        match &destination.place {
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
                Ok((store, folder.display().to_string()))
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
                        Ok((store, folder.display().to_string()))
                    }
                    Place::Remote { store, label } => Ok((*store, label)),
                    Place::Several { .. } => Err(ApiError::internal("a storage is a single place")),
                }
            }
        }
    }

    /// This data center's own key to its backups.
    fn own_backup_password(&self) -> Result<String, ApiError> {
        Ok(self
            .engines
            .identity()
            .map_err(ApiError::internal)?
            .with_seed(repository_password))
    }

    fn backup_repository(&self, destination: &BackupDestination) -> Result<Repository, ApiError> {
        let (store, label) = self.backup_store(destination)?;
        let backend = StoreBackend::new(Arc::new(store), tokio::runtime::Handle::current(), label);
        Ok(Repository::new(backend, self.own_backup_password()?))
    }

    fn place_label(&self, destination: &BackupDestination) -> String {
        match &destination.place {
            None => "This computer".to_string(),
            Some(storage) => self
                .settings()
                .ok()
                .and_then(|s| s.storage_labels.get(storage).cloned())
                .unwrap_or_else(|| storage.clone()),
        }
    }

    /// Whether `data_root`, a backup's label, is this data folder.
    fn is_this_folder(&self, data_root: &str) -> bool {
        let root = self.catalog.root_path();
        data_root == root
            || matches!(
                (Path::new(data_root).canonicalize(), Path::new(root).canonicalize()),
                (Ok(a), Ok(b)) if a == b
            )
    }

    /// Settings › Backup.
    pub async fn backup_info(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        let state = self.backup_state()?;
        let keep = Keep::default();
        let to_u32 = |n: i32| u32::try_from(n).unwrap_or(0);
        let next_at_ms = match (&state.schedule, state.enabled) {
            (Some(schedule), true) => {
                let (last, next) = times_around(schedule, Local::now());
                let since = i64::try_from(state.enabled_at_ms.max(state.last_started_ms))
                    .unwrap_or(i64::MAX);
                // A missed time is made up at once.
                Some(if last > since {
                    crate::now_ms()
                } else {
                    u64::try_from(next).unwrap_or(0)
                })
            }
            _ => None,
        };
        Ok(BackupInfo {
            enabled: state.enabled,
            places: state
                .destinations
                .iter()
                .map(|d| {
                    let results = state.results.get(&key_of(d)).cloned().unwrap_or_default();
                    BackupPlace {
                        place: d.place.clone(),
                        path: d.path.clone(),
                        label: self.place_label(d),
                        last_backup: results.backup,
                        last_check: results.check,
                    }
                })
                .collect(),
            schedule: state.schedule,
            next_at_ms,
            keep_daily: to_u32(keep.daily),
            keep_weekly: to_u32(keep.weekly),
            keep_monthly: to_u32(keep.monthly),
            running: self.backup_job.progress(),
        })
    }

    /// Turn backups on, or change where they go and when. Each place must be
    /// reachable, and backups already there must be this data center's.
    /// Nothing is backed up until the time comes.
    pub async fn set_backup(
        &self,
        caller: &Caller,
        req: BackupSettings,
    ) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let schedule = req.schedule;
        let weekly_ok =
            schedule.every == BackupEvery::Daily || schedule.weekday.is_some_and(|d| d < 7);
        if schedule.hour > 23 || schedule.minute > 59 || !weekly_ok {
            return Err(ApiError::invalid(
                "choose a time of day, and a day for weekly backups",
            ));
        }
        let mut destinations: Vec<BackupDestination> = Vec::new();
        for d in req.destinations {
            let d = BackupDestination {
                place: d.place.filter(|p| !p.trim().is_empty()),
                path: d.path.trim().to_string(),
            };
            if !destinations.iter().any(|e| key_of(e) == key_of(&d)) {
                destinations.push(d);
            }
        }
        if destinations.is_empty() || destinations.len() > MAX_PLACES {
            return Err(ApiError::invalid(format!(
                "choose one to {MAX_PLACES} places for backups"
            )));
        }
        for d in &destinations {
            let repo = self.backup_repository(d)?;
            let theirs = tokio::task::spawn_blocking(move || {
                Ok::<_, oikome_backup::BackupError>(repo.exists()? && repo.snapshots().is_err())
            })
            .await
            .map_err(ApiError::internal)?
            .map_err(failed)?;
            if theirs {
                return Err(ApiError::invalid(format!(
                    "the backups at {} were made by another data center",
                    self.place_label(d)
                ))
                .because("backup_other_key"));
            }
        }
        let schedule = BackupSchedule {
            weekday: if schedule.every == BackupEvery::Weekly {
                schedule.weekday
            } else {
                None
            },
            ..schedule
        };
        self.update_settings(|s| {
            let state = &mut s.backup;
            if !state.enabled {
                state.enabled_at_ms = crate::now_ms();
            }
            state.enabled = true;
            let keys: BTreeSet<String> = destinations.iter().map(key_of).collect();
            state.results.retain(|k, _| keys.contains(k));
            state.destinations = destinations;
            state.schedule = Some(schedule);
        })?;
        self.backup_info(caller, Empty {}).await
    }

    /// Turn backups off; the places and time are kept for turning them on
    /// again, and the backups made stay where they are.
    pub async fn turn_off_backup(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        self.update_settings(|s| s.backup.enabled = false)?;
        self.backup_info(caller, Empty {}).await
    }

    /// The backups kept at one place, the newest first.
    pub async fn list_backups(
        &self,
        caller: &Caller,
        req: BackupPlaceRef,
    ) -> Result<Vec<BackupItem>, ApiError> {
        caller.require_owner()?;
        let state = self.backup_state()?;
        let destination = usize::try_from(req.place)
            .ok()
            .and_then(|i| state.destinations.get(i))
            .ok_or_else(|| ApiError::not_found("there is no such backup place"))?;
        let repo = self.backup_repository(destination)?;
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
        Ok(snapshots
            .into_iter()
            .map(|s| {
                let here = self.is_this_folder(&s.data_root);
                BackupItem::from_snapshot(s, here)
            })
            .collect())
    }

    /// Start a job over every backup place: one job at a time.
    #[allow(clippy::type_complexity)]
    fn start_backup_job(
        &self,
        work: BackupWork,
        step: BackupStep,
    ) -> Result<(Vec<(BackupDestination, Repository, String)>, Arc<Meter>), ApiError> {
        let state = self.backup_state()?;
        if state.destinations.is_empty() {
            return Err(
                ApiError::invalid("choose where backups go first").because("backup_no_place")
            );
        }
        if self.backup_job.lock().is_some() {
            return Err(
                ApiError::new(ErrorCode::Conflict, "a backup or a check is under way")
                    .because("backup_running"),
            );
        }
        let meter = Arc::new(Meter::default());
        let mut targets = Vec::new();
        for d in &state.destinations {
            let label = self.place_label(d);
            // A place that cannot be reached now fails on its own.
            match self.backup_repository(d) {
                Ok(repo) => targets.push((d.clone(), repo.with_meter(meter.clone()), label)),
                Err(e) => self.record_backup_result(
                    d,
                    BackupResult {
                        work,
                        at_ms: crate::now_ms(),
                        error: Some(e.message),
                        backup: None,
                    },
                ),
            }
        }
        let mut running = self.backup_job.lock();
        if running.is_some() {
            return Err(
                ApiError::new(ErrorCode::Conflict, "a backup or a check is under way")
                    .because("backup_running"),
            );
        }
        *running = Some(Running {
            work,
            step,
            place: None,
            index: 0,
            of: u32::try_from(targets.len()).unwrap_or(u32::MAX),
            meter: meter.clone(),
            started_at_ms: crate::now_ms(),
        });
        Ok((targets, meter))
    }

    fn record_backup_result(&self, destination: &BackupDestination, result: BackupResult) {
        let key = key_of(destination);
        if let Err(e) = self.update_settings(|s| {
            let kept = s.backup.results.entry(key).or_default();
            match result.work {
                BackupWork::BackUp => kept.backup = Some(result),
                BackupWork::Check => kept.check = Some(result),
            }
        }) {
            tracing::warn!(error = %e, "keeping how a backup ended");
        }
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
            let LocationFormat::InPlace {
                place,
                path,
                sources,
            } = &location.format
            else {
                continue;
            };
            let files: Vec<serde_json::Value> = inplace::indexed_files(&self.catalog, &location)
                .unwrap_or_default()
                .into_iter()
                .map(|f| serde_json::json!([f.path, f.size, f.modified_ms]))
                .collect();
            // Folders added from a storage: not looked for on this computer.
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

    /// Start a backup in the background: one consistent copy of the data
    /// folder, sent to each backup place in turn.
    fn start_backup(&self) -> Result<(), ApiError> {
        let (targets, meter) = self.start_backup_job(BackupWork::BackUp, BackupStep::Copying)?;
        self.update_settings(|s| s.backup.last_started_ms = crate::now_ms())?;
        let originals = self.originals_for_backup();
        let root = self.catalog.root_path().to_string();
        let staging = self
            .catalog
            .oikome_dir()
            .join("backup")
            .join(format!("staging-{}", crate::now_ms()));
        let api = self.clone();
        tokio::spawn(async move {
            let copy = staging.clone();
            let (data, counted) = (PathBuf::from(&root), meter.clone());
            let staged =
                tokio::task::spawn_blocking(move || stage(&data, &copy, originals, Some(&counted)))
                    .await
                    .map_err(failed)
                    .and_then(|r| r.map_err(failed));
            for (i, (destination, repo, label)) in targets.into_iter().enumerate() {
                let made = match &staged {
                    Err(e) => Err(e.clone()),
                    Ok(_) => {
                        api.backup_job.at(BackupStep::Sending, &label, i);
                        let (copy, root) = (staging.clone(), root.clone());
                        tokio::task::spawn_blocking(move || {
                            repo.back_up(&copy, &root, &Keep::default())
                        })
                        .await
                        .map_err(failed)
                        .and_then(|r| r.map_err(failed))
                    }
                };
                api.record_backup_result(
                    &destination,
                    BackupResult {
                        work: BackupWork::BackUp,
                        at_ms: crate::now_ms(),
                        error: made.as_ref().err().map(|e| e.message.clone()),
                        backup: made.ok().map(|s| BackupItem::from_snapshot(s, true)),
                    },
                );
            }
            if let Err(e) = std::fs::remove_dir_all(&staging) {
                tracing::warn!(error = %e, "removing a backup's staging folder");
            }
            *api.backup_job.lock() = None;
        });
        Ok(())
    }

    /// Back up now, in the background, to every backup place.
    pub async fn back_up_now(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.start_backup()?;
        self.backup_info(caller, Empty {}).await
    }

    /// Back up when the time set has come (or was missed while the node was
    /// off), on the node's schedule; failures are kept with each place.
    pub async fn back_up_when_due(&self) {
        let Ok(state) = self.backup_state() else {
            return;
        };
        let Some(schedule) = state.schedule.filter(|_| state.enabled) else {
            return;
        };
        let (last, _) = times_around(&schedule, Local::now());
        let since =
            i64::try_from(state.enabled_at_ms.max(state.last_started_ms)).unwrap_or(i64::MAX);
        if last <= since || self.backup_job.lock().is_some() {
            return;
        }
        if let Err(e) = self.refresh().and_then(|()| self.start_backup()) {
            tracing::warn!(error = %e, "starting a scheduled backup");
        }
    }

    /// Check Backup, in the background: every backup place read back and
    /// checked to be whole.
    pub async fn check_backup(&self, caller: &Caller, _: Empty) -> Result<BackupInfo, ApiError> {
        caller.require_owner()?;
        let (targets, _) = self.start_backup_job(BackupWork::Check, BackupStep::Checking)?;
        let api = self.clone();
        tokio::spawn(async move {
            for (i, (destination, repo, label)) in targets.into_iter().enumerate() {
                api.backup_job.at(BackupStep::Checking, &label, i);
                let checked = tokio::task::spawn_blocking(move || repo.check())
                    .await
                    .map_err(failed)
                    .and_then(|r| r.map_err(failed));
                api.record_backup_result(
                    &destination,
                    BackupResult {
                        work: BackupWork::Check,
                        at_ms: crate::now_ms(),
                        error: checked.err().map(|e| e.message),
                        backup: None,
                    },
                );
            }
            *api.backup_job.lock() = None;
        });
        self.backup_info(caller, Empty {}).await
    }

    /// Restore from Backup: turn this data center back to a backup of this
    /// very data folder, from one of its backup places or another folder on
    /// this computer. The backup is downloaded and checked now and put in
    /// place when Oikome is started again. The data center's own identity
    /// opens its backups; the recovery key is asked for only when it no
    /// longer does.
    pub async fn restore_from_backup(
        &self,
        caller: &Caller,
        req: RestoreBackup,
    ) -> Result<RestoredBackup, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if self.backup_job.lock().is_some() {
            return Err(
                ApiError::new(ErrorCode::Conflict, "a backup or a check is under way")
                    .because("backup_running"),
            );
        }
        let (store, label) = match (req.place, req.folder.as_deref()) {
            (Some(i), None) => {
                let state = self.backup_state()?;
                let destination = usize::try_from(i)
                    .ok()
                    .and_then(|i| state.destinations.get(i))
                    .ok_or_else(|| ApiError::not_found("there is no such backup place"))?;
                self.backup_store(destination)?
            }
            (None, Some(folder)) => folder_store(folder).await?,
            _ => return Err(ApiError::invalid("choose where the backups are")),
        };
        let store = Arc::new(store);
        let open = |password: String| {
            let backend = StoreBackend::new(
                store.clone(),
                tokio::runtime::Handle::current(),
                label.clone(),
            );
            let repo = Repository::new(backend, password);
            tokio::task::spawn_blocking(move || {
                let snapshots = repo.snapshots();
                (repo, snapshots)
            })
        };
        let opened = open(self.own_backup_password()?)
            .await
            .map_err(ApiError::internal)?;
        let (repo, snapshots, seed) = match opened {
            (repo, Ok(snapshots)) => (repo, snapshots, None),
            (_, Err(_)) => {
                let Some(key) = req
                    .recovery_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                else {
                    return Err(ApiError::invalid(
                        "this data center's identity does not open these backups; enter the recovery key",
                    )
                    .because("backup_key_needed"));
                };
                let seed = decode_recovery_key(key)
                    .map_err(|e| ApiError::invalid(e.to_string()).because("backup_wrong_key"))?;
                match open(repository_password(&seed))
                    .await
                    .map_err(ApiError::internal)?
                {
                    (repo, Ok(snapshots)) => (repo, snapshots, Some(seed)),
                    (_, Err(_)) => {
                        return Err(ApiError::invalid(
                            "this recovery key does not open these backups",
                        )
                        .because("backup_wrong_key"))
                    }
                }
            }
        };
        let chosen = match req.backup.as_deref() {
            Some(id) => snapshots
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| ApiError::not_found("there is no such backup"))?,
            None => snapshots
                .into_iter()
                .find(|s| self.is_this_folder(&s.data_root))
                .ok_or_else(|| {
                    ApiError::not_found("there is no backup of this data folder there")
                        .because("backup_none_here")
                })?,
        };
        if !self.is_this_folder(&chosen.data_root) {
            return Err(ApiError::invalid(
                "this backup is of another data folder; restoring onto another computer or into another folder is part of Oikome's commercial edition",
            )
            .because("backup_elsewhere"));
        }

        let root = PathBuf::from(self.catalog.root_path());
        let incoming = self
            .catalog
            .oikome_dir()
            .join("backup")
            .join(format!("incoming-{}", crate::now_ms()));
        let (into, id) = (incoming.clone(), chosen.id.clone());
        let restored = tokio::task::spawn_blocking(move || repo.restore(Some(&id), &into))
            .await
            .map_err(ApiError::internal)?;
        let manifest = match restored {
            Ok(manifest) => manifest,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&incoming);
                return Err(failed(e));
            }
        };
        if let Some(seed) = seed {
            let key = incoming
                .join(".oikome")
                .join("identity")
                .join(oikome_identity::keys::NODE_KEY_FILE);
            NodeIdentity::write_key_file(&key, &seed).map_err(ApiError::internal)?;
        }
        let pending = pending_dir(&root);
        if pending.exists() {
            std::fs::remove_dir_all(&pending).map_err(ApiError::internal)?;
        }
        std::fs::rename(&incoming, &pending).map_err(ApiError::internal)?;
        Ok(RestoredBackup {
            made_at_ms: chosen.made_at_ms,
            files: manifest.files.len() as u64,
            missing_originals: missing_originals(&manifest),
        })
    }
}

/// Where a restore waits, checked, for Oikome's next start.
fn pending_dir(root: &Path) -> PathBuf {
    root.join(".oikome").join("backup").join("pending")
}

/// The backups in a folder on this computer: the folder chosen for them, or
/// its `oikome-backup` folder.
async fn folder_store(folder: &str) -> Result<(RemoteStore, String), ApiError> {
    let chosen = Path::new(folder.trim());
    if !chosen.is_absolute() {
        return Err(ApiError::invalid("give the whole path of the folder"));
    }
    for at in [chosen.to_path_buf(), chosen.join(REPOSITORY_FOLDER)] {
        if !at.is_dir() {
            continue;
        }
        let store = RemoteStore::folder(&at, "").map_err(failed)?;
        if store.size("config").await.ok().flatten().is_some() {
            return Ok((store, at.display().to_string()));
        }
    }
    Err(
        ApiError::not_found(format!("no backups were found in {}", chosen.display()))
            .because("backup_none_here"),
    )
}

/// The originals the backup's collections view where they are on this
/// computer that are not where they were.
fn missing_originals(manifest: &Manifest) -> Vec<MissingOriginals> {
    let mut missing = Vec::new();
    for entry in manifest.originals.as_array().into_iter().flatten() {
        if !entry["place"].is_null() {
            continue;
        }
        let Some(folder) = entry["path"].as_str() else {
            continue;
        };
        let elsewhere: Vec<&str> = entry["elsewhere"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m.as_str())
            .collect();
        let gone: Vec<String> = entry["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f[0].as_str())
            .filter(|p| {
                !elsewhere
                    .iter()
                    .any(|m| p == m || p.starts_with(&format!("{m}/")))
            })
            .filter(|p| !Path::new(folder).join(p).exists())
            .map(str::to_string)
            .collect();
        if !gone.is_empty() {
            missing.push(MissingOriginals {
                collection: entry["collection"].as_str().unwrap_or_default().to_string(),
                folder: folder.to_string(),
                missing: gone.len() as u64,
                some: gone.into_iter().take(5).collect(),
            });
        }
    }
    missing
}

/// What of the data folder stays as it is when a backup is put in place:
/// what a backup never holds (models, extensions, the login link, backups'
/// own folder, stored credentials) and the identity's seed unless the
/// backup brings it.
fn stays(relative: &Path, restored: &BTreeSet<PathBuf>) -> bool {
    const KEPT: [&str; 6] = [
        ".oikome/backup",
        ".oikome/models",
        ".oikome/extensions",
        ".oikome/web",
        ".oikome/identity/storage_access.json",
        ".duckdb",
    ];
    let seed = Path::new(".oikome/identity").join(oikome_identity::keys::NODE_KEY_FILE);
    KEPT.iter().any(|k| relative.starts_with(k)) || (relative == seed && !restored.contains(&seed))
}

/// Everything under `dir` that is a file, relative to `base`.
fn files_under(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            files_under(&path, base, out)?;
        } else if let Ok(relative) = path.strip_prefix(base) {
            out.push(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Put a restore made ready by [`Api::restore_from_backup`] in place, before
/// anything opens the data folder `root`. What the data center holds now
/// (its registries and settings, its tables and their folders) is moved
/// aside first and brought back if the backup cannot be put in place;
/// files the backup does not know are left where they are. Whether there
/// was one.
pub fn apply_pending_restore(root: &Path) -> Result<bool, ApiError> {
    let pending = pending_dir(root);
    if !pending.is_dir() {
        return Ok(false);
    }
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(pending.join(MANIFEST_FILE)).map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    let mut restored = Vec::new();
    files_under(&pending, &pending, &mut restored).map_err(ApiError::internal)?;
    restored.retain(|p| p != Path::new(MANIFEST_FILE));
    let restored_set: BTreeSet<PathBuf> = restored.iter().cloned().collect();
    let tops: BTreeSet<String> = restored
        .iter()
        .filter_map(|p| p.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    // What belongs to the data center now: its own folder, the folders the
    // backup brings back, the tables' namespaces and its imports.
    let mut current = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(ApiError::internal)?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let ours = name == ".oikome"
            || tops.contains(&name)
            || matches!(name.as_str(), "Imports" | "Examples" | "tmp")
            || path.join(".namespace.properties").is_file();
        if !ours {
            continue;
        }
        if path.is_dir() {
            files_under(&path, root, &mut current).map_err(ApiError::internal)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            current.push(relative.to_path_buf());
        }
    }
    current.retain(|p| !stays(p, &restored_set));

    let aside = root
        .join(".oikome")
        .join("backup")
        .join(format!("replaced-{}", crate::now_ms()));
    let step = |from: &Path, to: &Path| -> std::io::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)
    };
    let mut moved: Vec<PathBuf> = Vec::new();
    let mut put = || -> std::io::Result<()> {
        for relative in &current {
            step(&root.join(relative), &aside.join(relative))?;
            moved.push(relative.clone());
        }
        for relative in &restored {
            let to = root.join(relative);
            for side in ["-wal", "-shm", "-journal"] {
                let mut name = to.clone().into_os_string();
                name.push(side);
                let _ = std::fs::remove_file(PathBuf::from(name));
            }
            step(&pending.join(relative), &to)?;
        }
        Ok(())
    };
    if let Err(e) = put() {
        // Bring back what was there; the backup's files go back beside it.
        for relative in &restored {
            let at = root.join(relative);
            if at.is_file() && !pending.join(relative).exists() {
                let _ = step(&at, &pending.join(relative));
            }
        }
        for relative in moved.iter().rev() {
            let _ = step(&aside.join(relative), &root.join(relative));
        }
        return Err(ApiError::internal(format!(
            "putting the backup in place: {e}"
        )));
    }
    if !manifest.data_root.is_empty() && manifest.data_root != root.to_string_lossy() {
        relocate(root, &manifest.data_root).map_err(failed)?;
    }
    let _ = std::fs::remove_dir_all(&pending);
    let _ = std::fs::remove_dir_all(&aside);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_missed_is_due_and_the_next_one_follows() {
        let daily = BackupSchedule {
            every: BackupEvery::Daily,
            weekday: None,
            hour: 21,
            minute: 0,
        };
        let at = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 11, d, h, 30, 0).unwrap();
        let (last, next) = times_around(&daily, at(19, 22));
        assert_eq!(
            last,
            Local
                .with_ymd_and_hms(2026, 11, 19, 21, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            next,
            Local
                .with_ymd_and_hms(2026, 11, 20, 21, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        let (last, _) = times_around(&daily, at(19, 8));
        assert_eq!(
            last,
            Local
                .with_ymd_and_hms(2026, 11, 18, 21, 0, 0)
                .unwrap()
                .timestamp_millis()
        );

        // 2026-11-19 is a Thursday; weekly on Monday (0).
        let weekly = BackupSchedule {
            every: BackupEvery::Weekly,
            weekday: Some(0),
            ..daily
        };
        let (last, next) = times_around(&weekly, at(19, 22));
        assert_eq!(
            last,
            Local
                .with_ymd_and_hms(2026, 11, 16, 21, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            next,
            Local
                .with_ymd_and_hms(2026, 11, 23, 21, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
    }
}
