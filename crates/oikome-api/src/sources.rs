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

//! Sources (architecture v3 §11) in the owner's API: where a collection's
//! data comes from. A folder of photos or notes, or a CSV file, stays the
//! owner's, editable with any app; the node reads it again on a schedule
//! and when asked, into the collection of the source's name.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use oikome_catalog::SourceDef;
use oikome_connect::csv::CsvSource;
use oikome_connect::{Connector, SourceConfig, Value};

use crate::{
    Api, ApiError, Caller, Empty, ErrorCode, NewSource, PauseSource, SetSourceEvery, SourceKind,
    SourceRef, SourceRun, SourceSummary,
};

/// Sources are read again this often while the node runs.
pub(crate) const EVERY: Duration = Duration::from_secs(60);
/// How often the owner can have a source read, in seconds.
const EVERY_CHOICES: [u64; 5] = [60, 300, 1800, 3600, 86_400];

/// What the owner sees of a source's settings.
fn kind_and_place(config: &SourceConfig) -> (SourceKind, String) {
    match config {
        SourceConfig::Csv { path, .. } => (SourceKind::Csv, path.clone()),
        SourceConfig::FlomoExport { path } => (SourceKind::FlomoExport, path.clone()),
        SourceConfig::FlomoMcp { endpoint, .. } => (SourceKind::FlomoMcp, endpoint.clone()),
        SourceConfig::PhotosFolder { path } => (SourceKind::Photos, path.clone()),
        SourceConfig::NotesFolder { path } => (SourceKind::Notes, path.clone()),
    }
}

/// A source's file or folder as its removed collection is kept by: the
/// real path, so `/var/x` and `/private/var/x` are one place.
fn place_key(place: &str) -> String {
    std::fs::canonicalize(place).map_or_else(|_| place.to_string(), |p| p.display().to_string())
}

fn summary(def: &SourceDef, config: &SourceConfig, pulled_at_ms: Option<u64>) -> SourceSummary {
    let (kind, place) = kind_and_place(config);
    SourceSummary {
        name: def.name.clone(),
        kind,
        place,
        every_secs: def.every_secs,
        paused: def.paused,
        pulled_at_ms,
        failed_at_ms: def.failure.as_ref().map(|f| f.at_ms),
        failure: def.failure.as_ref().map(|f| f.message.clone()),
    }
}

/// A collection name made from a file or folder name: `Bank Export 2025`
/// reads `bank-export-2025`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    let out = &out[..out.len().min(48)];
    out.trim_end_matches('-').to_string()
}

/// A table name for a CSV file's rows.
fn table_name(stem: &str) -> String {
    let name = slug(stem).replace('-', "_");
    if oikome_connect::csv::is_table_name(&name) {
        name
    } else {
        "rows".to_string()
    }
}

/// The source of a CSV file, its rows told apart by its first column; a
/// file whose first column repeats is refused, since its rows would merge.
pub(crate) async fn csv_config(path: &Path, table: &str) -> Result<SourceConfig, ApiError> {
    let file = path.display();
    let header = CsvSource::header(path).map_err(|e| ApiError::invalid(e.to_string()))?;
    let first = header
        .first()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::invalid(format!("{file} has no header row")))?;
    let def = CsvSource::inspect(path, table, std::slice::from_ref(first))
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let source = CsvSource::new(path.to_path_buf(), def.clone());
    let pull = source
        .pull(None)
        .await
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let rows = pull
        .upserts
        .first()
        .map(|(_, rows)| rows.as_slice())
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let key = match &row[0] {
            Value::Text(t) => t.clone(),
            Value::Int64(n) => n.to_string(),
            Value::Float64(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null | Value::Blob(_) => String::new(),
        };
        if !seen.insert(key) {
            return Err(ApiError::invalid(format!(
                "the first column of {file} repeats, so its rows can't be told apart; \
                 put a column that names each row first"
            )));
        }
    }
    Ok(SourceConfig::Csv {
        path: path.to_string_lossy().into_owned(),
        table: def,
    })
}

/// Show a folder, or a file in its folder, in this computer's file
/// manager. The path is passed as an argument, never through a shell.
fn reveal(path: &Path, folder: bool) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("open");
        if !folder {
            c.arg("-R");
        }
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("explorer");
        if folder {
            c.arg(path);
        } else {
            c.arg(format!("/select,{}", path.display()));
        }
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = Command::new("xdg-open");
        c.arg(if folder {
            path
        } else {
            path.parent().unwrap_or(path)
        });
        c
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap it when it exits.
    std::thread::spawn(move || child.wait());
    Ok(())
}

impl Api {
    /// The sources, by name, with how their last read went.
    pub async fn list_sources(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<SourceSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let session = self.session(None);
        let mut out = Vec::new();
        for def in self.catalog.list_sources() {
            let Ok(config) = serde_json::from_value::<SourceConfig>(def.config.clone()) else {
                continue;
            };
            let pulled = session
                .last_pull(&def.name)
                .await
                .ok()
                .flatten()
                .map(|p| p.pulled_at_ms);
            out.push(summary(&def, &config, pulled));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Bring in a folder of photos or notes, or a CSV file, as a new
    /// collection named after it, read once now and again every minute
    /// while the node runs.
    pub async fn add_source(
        &self,
        caller: &Caller,
        req: NewSource,
    ) -> Result<SourceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let given = Path::new(req.path.trim());
        if !given.is_absolute() {
            return Err(ApiError::invalid(
                "choose a folder or file on this computer",
            ));
        }
        let path: PathBuf = given
            .canonicalize()
            .map_err(|_| ApiError::invalid(format!("{} is not there", given.display())))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let config = match req.kind {
            SourceKind::Photos | SourceKind::Notes => {
                if !path.is_dir() {
                    return Err(ApiError::invalid(format!(
                        "{} is not a folder",
                        path.display()
                    )));
                }
                let path = path.to_string_lossy().into_owned();
                if req.kind == SourceKind::Photos {
                    SourceConfig::PhotosFolder { path }
                } else {
                    SourceConfig::NotesFolder { path }
                }
            }
            SourceKind::Csv => {
                let csv = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
                if !path.is_file() || !csv {
                    return Err(ApiError::invalid(format!(
                        "{} is not a CSV file",
                        path.display()
                    )));
                }
                csv_config(&path, &table_name(&stem)).await?
            }
            SourceKind::FlomoExport | SourceKind::FlomoMcp => {
                return Err(ApiError::invalid("flomo is added with `oikome source add`"));
            }
        };
        let base = match slug(&stem) {
            s if s.starts_with(|c: char| c.is_ascii_alphabetic()) => s,
            s if s.is_empty() => "files".to_string(),
            s => format!("files-{s}"),
        };
        let taken = |name: &str| {
            self.catalog.get_space(name).is_ok()
                || self.catalog.get_location(name).is_ok()
                || self.catalog.get_source(name).is_ok()
        };
        // A file or folder whose source was removed goes back into its
        // collection, while that is still there and nothing else reads into it.
        let place = place_key(&kind_and_place(&config).1);
        let back = self
            .settings()?
            .removed_sources
            .get(&place)
            .filter(|name| {
                self.catalog.get_source(name).is_err()
                    && self
                        .catalog
                        .get_space(name)
                        .is_ok_and(|s| s.kind == oikome_core::SpaceKind::Transactional)
            })
            .cloned();
        let name = back.unwrap_or_else(|| {
            std::iter::once(base.clone())
                .chain((2..).map(|i| format!("{base}-{i}")))
                .find(|n| !taken(n))
                .unwrap_or(base)
        });
        self.session(None)
            .add_source(&name, config, Some(EVERY), None)
            .await?;
        self.update_settings(|s| s.removed_sources.remove(&place))?;
        self.list_sources(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| ApiError::internal("the added source is missing"))
    }

    /// Read a source again now.
    pub async fn pull_source(
        &self,
        caller: &Caller,
        req: SourceRef,
    ) -> Result<SourceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.catalog.get_source(&req.source)?;
        self.session(None).pull_source(&req.source).await?;
        self.list_sources(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == req.source)
            .ok_or_else(|| ApiError::not_found("there is no such source"))
    }

    /// How often a source is read, from the choices the apps offer.
    pub async fn set_source_every(
        &self,
        caller: &Caller,
        req: SetSourceEvery,
    ) -> Result<SourceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if req.every_secs.is_some_and(|s| !EVERY_CHOICES.contains(&s)) {
            return Err(ApiError::invalid(
                "read it every minute, 5 or 30 minutes, hour or day, or only when asked",
            ));
        }
        self.catalog.get_source(&req.source)?;
        self.catalog.set_source_every(&req.source, req.every_secs)?;
        self.list_sources(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == req.source)
            .ok_or_else(|| ApiError::not_found("there is no such source"))
    }

    /// A source's recent syncs, newest first.
    pub async fn list_source_runs(
        &self,
        caller: &Caller,
        req: SourceRef,
    ) -> Result<Vec<SourceRun>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.catalog.get_source(&req.source)?;
        Ok(self
            .session(None)
            .source_runs(&req.source)
            .await?
            .into_iter()
            .map(|r| SourceRun {
                first_ms: r.first_ms,
                at_ms: r.at_ms,
                failure: r.failure,
                upserted: r.upserted,
                deleted: r.deleted,
                count: r.count,
            })
            .collect())
    }

    /// Pause a source, or resume it. Paused, it isn't read on its schedule
    /// (Sync Now still reads it); its collection stays as it is.
    pub async fn pause_source(
        &self,
        caller: &Caller,
        req: PauseSource,
    ) -> Result<SourceSummary, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.catalog.get_source(&req.source)?;
        self.catalog.set_source_paused(&req.source, req.paused)?;
        self.list_sources(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == req.source)
            .ok_or_else(|| ApiError::not_found("there is no such source"))
    }

    /// Stop reading a source; its collection stays as it is, and so do the
    /// files it read. Added again, they go back into that collection.
    pub async fn remove_source(
        &self,
        caller: &Caller,
        req: SourceRef,
    ) -> Result<Vec<SourceSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let def = self.catalog.get_source(&req.source)?;
        self.session(None).remove_source(&req.source)?;
        if let Ok(config) = serde_json::from_value::<SourceConfig>(def.config) {
            let place = place_key(&kind_and_place(&config).1);
            self.update_settings(|s| s.removed_sources.insert(place, def.name))?;
        }
        self.list_sources(caller, Empty {}).await
    }

    /// Open the folder a source reads, or show its file, in this computer's
    /// file manager. Only a registered source's own path is opened.
    pub async fn show_source_files(
        &self,
        caller: &Caller,
        req: SourceRef,
    ) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let def = self.catalog.get_source(&req.source)?;
        let config: SourceConfig = serde_json::from_value(def.config)
            .map_err(|e| ApiError::internal(format!("source settings: {e}")))?;
        let (path, folder) = match &config {
            SourceConfig::PhotosFolder { path } | SourceConfig::NotesFolder { path } => {
                (PathBuf::from(path), true)
            }
            SourceConfig::Csv { path, .. } | SourceConfig::FlomoExport { path } => {
                (PathBuf::from(path), false)
            }
            SourceConfig::FlomoMcp { .. } => {
                return Err(ApiError::new(
                    ErrorCode::Invalid,
                    "this source is a service, not files",
                ))
            }
        };
        if !path.exists() {
            return Err(ApiError::invalid(format!(
                "{} is not there any more",
                path.display()
            )));
        }
        reveal(&path, folder).map_err(ApiError::internal)?;
        Ok(Empty {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_files() {
        assert_eq!(slug("Bank Export 2025"), "bank-export-2025");
        assert_eq!(slug("  photos/"), "photos");
        assert_eq!(slug("Pastéis"), "past-is");
        assert_eq!(table_name("2025 export"), "rows");
        assert_eq!(table_name("Bank export"), "bank_export");
    }
}
