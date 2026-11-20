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

//! The restic repository backups are kept in. Every call is blocking (rustic
//! works on its own threads); call from a blocking task.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustic_core::jiff::Zoned;
use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, Credentials, KeepOptions, KeyOptions,
    LocalDestination, LsOptions, PathList, PruneOptions, Repository as Rustic, RepositoryBackends,
    RepositoryOptions, RestoreOptions, SnapshotOptions,
};
use serde::{Deserialize, Serialize};

use crate::meter::{Bars, Meter};
use crate::stage::{verify, Manifest};
use crate::{io, repo_err, BackupError, StoreBackend};

/// The path every backup is kept under in the repository, whatever folder
/// it was staged in, so that one backup follows the other.
const AS_PATH: &str = "/oikome-data";

/// The repository's password, derived from the identity's seed (which is
/// the recovery key): whoever holds the recovery key can open the backups,
/// and nothing else is needed.
pub fn repository_password(seed: &[u8; 32]) -> String {
    blake3::derive_key("oikome 2026-11-19 backup repository password", seed)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Which backups are kept: one a day for `daily` days, one a week for
/// `weekly` weeks, one a month for `monthly` months.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keep {
    pub daily: i32,
    pub weekly: i32,
    pub monthly: i32,
}

impl Default for Keep {
    fn default() -> Self {
        Keep {
            daily: 7,
            weekly: 4,
            monthly: 12,
        }
    }
}

/// A backup in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    /// When it was made, in milliseconds since the Unix epoch.
    pub made_at_ms: i64,
    /// The size of what it holds.
    pub bytes: u64,
    pub files: u64,
    /// What it added to the repository, after deduplication and compression.
    pub added: u64,
    /// The data folder it is a backup of, as its catalog names it.
    pub data_root: String,
}

impl From<&rustic_core::repofile::SnapshotFile> for Snapshot {
    fn from(snap: &rustic_core::repofile::SnapshotFile) -> Self {
        let summary = snap.summary.as_ref();
        Snapshot {
            id: snap.id.to_string(),
            made_at_ms: snap.time.timestamp().as_millisecond(),
            bytes: summary.map_or(0, |s| s.total_bytes_processed),
            files: summary.map_or(0, |s| s.total_files_processed),
            added: summary.map_or(0, |s| s.data_added_packed),
            data_root: snap.label.clone(),
        }
    }
}

/// A repository on a place, opened with its password.
pub struct Repository {
    backends: RepositoryBackends,
    credentials: Credentials,
    bars: Bars,
}

impl Repository {
    pub fn new(backend: StoreBackend, password: String) -> Self {
        Repository {
            backends: RepositoryBackends::new(Arc::new(backend), None),
            credentials: Credentials::Password(password),
            bars: Bars::default(),
        }
    }

    /// Count the bytes of what follows into `meter`.
    #[must_use]
    pub fn with_meter(mut self, meter: Arc<Meter>) -> Self {
        self.bars = Bars(Some(meter));
        self
    }

    fn rustic(&self) -> Result<Rustic<()>, BackupError> {
        Rustic::new_with_progress(
            &RepositoryOptions::default(),
            &self.backends,
            self.bars.clone(),
        )
        .map_err(repo_err)
    }

    /// Whether a repository is there yet.
    pub fn exists(&self) -> Result<bool, BackupError> {
        Ok(self.rustic()?.config_id().map_err(repo_err)?.is_some())
    }

    /// Keep the staged copy at `staging` as a new backup (making the
    /// repository first if there is none), then let go of the backups
    /// `keep` does not keep and of what only they held.
    /// `data_root`, the data folder as its catalog names it, is kept as the
    /// backup's label, so a backup says whose it is without being read.
    pub fn back_up(
        &self,
        staging: &Path,
        data_root: &str,
        keep: &Keep,
    ) -> Result<Snapshot, BackupError> {
        let rustic = self.rustic()?;
        let repo = if rustic.config_id().map_err(repo_err)?.is_some() {
            rustic.open(&self.credentials).map_err(repo_err)?
        } else {
            rustic
                .init(
                    &self.credentials,
                    &KeyOptions::default(),
                    &ConfigOptions::default(),
                )
                .map_err(repo_err)?
        };
        let repo = repo.to_indexed_ids().map_err(repo_err)?;
        let staging = staging
            .to_str()
            .ok_or_else(|| io("the staging folder's path is not UTF-8"))?;
        let source = PathList::from_string(staging)
            .map_err(repo_err)?
            .sanitize()
            .map_err(repo_err)?;
        let options = BackupOptions::default().as_path(Some(PathBuf::from(AS_PATH)));
        let snapshot = SnapshotOptions::default()
            .label(Some(data_root.to_string()))
            .to_snapshot()
            .map_err(repo_err)?;
        let made = repo.backup(&options, &source, snapshot).map_err(repo_err)?;

        let keep = KeepOptions::default()
            .keep_daily(keep.daily)
            .keep_weekly(keep.weekly)
            .keep_monthly(keep.monthly);
        let all = repo.get_all_snapshots().map_err(repo_err)?;
        let gone: Vec<_> = keep
            .apply(all, &Zoned::now())
            .map_err(repo_err)?
            .into_iter()
            .filter(|s| !s.keep)
            .map(|s| s.snapshot.id)
            .collect();
        if !gone.is_empty() {
            repo.delete_snapshots(&gone).map_err(repo_err)?;
            let options = PruneOptions::default();
            let plan = repo.prune_plan(&options).map_err(repo_err)?;
            repo.prune(&options, plan).map_err(repo_err)?;
        }
        Ok(Snapshot::from(&made))
    }

    /// The backups there, the newest first.
    pub fn snapshots(&self) -> Result<Vec<Snapshot>, BackupError> {
        let repo = self.rustic()?.open(&self.credentials).map_err(repo_err)?;
        let mut snapshots: Vec<Snapshot> = repo
            .get_all_snapshots()
            .map_err(repo_err)?
            .iter()
            .map(Snapshot::from)
            .collect();
        snapshots.sort_by_key(|s| std::cmp::Reverse(s.made_at_ms));
        Ok(snapshots)
    }

    /// Read every file of the repository and check that it is whole.
    pub fn check(&self) -> Result<(), BackupError> {
        let repo = self.rustic()?.open(&self.credentials).map_err(repo_err)?;
        repo.check(CheckOptions::default().read_data(true))
            .map_err(repo_err)?
            .is_ok()
            .map_err(|e| BackupError::Damaged(e.to_string()))
    }

    /// Restore the backup `id` (the newest when `None`) into the empty
    /// folder `target`, and check it against its manifest.
    pub fn restore(&self, id: Option<&str>, target: &Path) -> Result<Manifest, BackupError> {
        let repo = self
            .rustic()?
            .open(&self.credentials)
            .map_err(repo_err)?
            .to_indexed()
            .map_err(repo_err)?;
        let node = repo
            .node_from_snapshot_path(&format!("{}:{AS_PATH}", id.unwrap_or("latest")), |_| true)
            .map_err(repo_err)?;
        let entries = repo.ls(&node, &LsOptions::default()).map_err(repo_err)?;
        std::fs::create_dir_all(target).map_err(io)?;
        let target = target
            .to_str()
            .ok_or_else(|| io("the restore folder's path is not UTF-8"))?;
        let destination =
            LocalDestination::new(&format!("{target}/"), true, false).map_err(repo_err)?;
        let options = RestoreOptions::default();
        let plan = repo
            .prepare_restore(&options, entries.clone(), &destination, false)
            .map_err(repo_err)?;
        repo.restore(plan, &options, entries, &destination)
            .map_err(repo_err)?;
        verify(Path::new(target))
    }
}
