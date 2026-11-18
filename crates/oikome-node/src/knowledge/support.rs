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

//! The DuckDB extension knowledge is kept with (`lance`), installed when the
//! owner asks. The node downloads it from DuckDB's repository itself, so the
//! apps can show how far it is, and has DuckDB install the file; DuckDB
//! checks its signature whenever it loads it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::fetch;
use crate::error::SessionError;
use crate::registry::EngineRegistry;
use crate::session::OikomeSession;

/// Where the extension's installation is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportState {
    /// Not on its way (installed or not: [`OikomeSession::knowledge_available`] says).
    Idle,
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    Installing,
    /// The last try failed; trying again resumes the download.
    Failed(String),
}

/// Installations running or failed, by extension directory.
static STATE: Mutex<Option<HashMap<PathBuf, SupportState>>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut HashMap<PathBuf, SupportState>) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

fn io(path: &Path, e: std::io::Error) -> SessionError {
    SessionError::Unsupported(format!("{}: {e}", path.display()))
}

impl OikomeSession {
    /// The extension's installation, and the directory it goes to.
    pub fn knowledge_support(&self) -> (SupportState, PathBuf) {
        let dir = self.engines().extension_dir();
        let now = state(|s| s.get(&dir).cloned()).unwrap_or(SupportState::Idle);
        (now, dir)
    }

    /// Start downloading and installing the extension in the background;
    /// nothing happens while it already is. Owner only.
    pub fn install_knowledge_support(&self) -> Result<SupportState, SessionError> {
        self.deny_principal("installing an extension")?;
        let engines = self.engines_arc();
        let dir = engines.extension_dir();
        let started = state(|s| {
            if matches!(
                s.get(&dir),
                Some(SupportState::Downloading { .. } | SupportState::Installing)
            ) {
                return false;
            }
            s.insert(
                dir.clone(),
                SupportState::Downloading {
                    done: 0,
                    total: None,
                },
            );
            true
        });
        if started {
            let key = dir.clone();
            tokio::spawn(async move {
                let result = install(&engines, &key).await;
                state(|s| match result {
                    Ok(_) => {
                        s.remove(&key);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "installing the lance extension");
                        s.insert(key.clone(), SupportState::Failed(e.to_string()));
                    }
                });
            });
        }
        Ok(self.knowledge_support().0)
    }
}

async fn install(engines: &Arc<EngineRegistry>, key: &Path) -> Result<String, SessionError> {
    let (version, platform) = engines.duckdb_version_and_platform().await?;
    let url =
        format!("https://extensions.duckdb.org/{version}/{platform}/lance.duckdb_extension.gz");
    let work = engines.temp_dir().join("extension-download");
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|e| io(&work, e))?;
    let part = work.join(format!("lance-{version}-{platform}.duckdb_extension.gz"));
    let client = fetch::client()?;
    fetch::fetch(&client, &url, &part, None, |done, total| {
        state(|s| {
            s.insert(key.to_path_buf(), SupportState::Downloading { done, total });
        })
    })
    .await?;
    state(|s| {
        s.insert(key.to_path_buf(), SupportState::Installing);
    });
    // DuckDB installs `<name>.duckdb_extension` files.
    let file = work.join("lance.duckdb_extension");
    let (from, to) = (part.clone(), file.clone());
    let unpacked = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut reader = flate2::read::GzDecoder::new(std::fs::File::open(&from)?);
        let mut out = std::fs::File::create(&to)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            std::io::Write::write_all(&mut out, &buf[..n])?;
        }
    })
    .await
    .map_err(|e| SessionError::Unsupported(format!("extension: {e}")))?;
    if unpacked.is_err() {
        // A damaged download: the next try starts again.
        let _ = tokio::fs::remove_file(&part).await;
        let _ = tokio::fs::remove_file(&file).await;
        return Err(SessionError::Unsupported(
            "the downloaded extension was damaged and was deleted; try again".into(),
        ));
    }
    let installed = engines.install_lance_file(&file).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    installed
}
