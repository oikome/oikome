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

//! Backups of a data folder.
//!
//! A backup is made in two steps. [`stage`] copies the data folder to a
//! staging folder so that the copy is consistent (every SQLite file through
//! `VACUUM INTO`, the registries before the tables they name) and leaves out
//! what is not the owner's to keep there: what can be rebuilt (indexes,
//! caches), what is downloaded again (models, extensions), the identity's
//! seed (it is the recovery key) and stored credentials. It writes a
//! [`Manifest`] with each file's BLAKE3 hash and each SQLite table's rows.
//! [`Repository::back_up`] then keeps the staging folder in a restic-format
//! repository (encrypted, deduplicated, readable by the standard `restic`
//! program) on any of Oikome's places, through [`StoreBackend`].
//!
//! A restore ([`Repository::restore`]) goes to a scratch folder and is
//! checked against its manifest ([`verify`]) before anything uses it.

mod meter;
mod relocate;
mod repo;
mod stage;
mod store;

pub use meter::Meter;
pub use relocate::relocate;
pub use repo::{repository_password, Keep, Repository, Snapshot};
pub use stage::{stage, verify, Manifest, ManifestFile, SqliteTable, MANIFEST_FILE};
pub use store::StoreBackend;

/// Why a backup, a check or a restore failed.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Repository(String),
    /// The backup is not what its manifest says.
    #[error("the backup does not match its manifest: {0}")]
    Damaged(String),
}

pub(crate) fn io(e: impl std::fmt::Display) -> BackupError {
    BackupError::Io(e.to_string())
}

pub(crate) fn repo_err(e: impl std::fmt::Display) -> BackupError {
    BackupError::Repository(e.to_string())
}
