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

//! Embedding models knowledge can be searched by meaning with
//! (docs/knowledge-schema.md §4.3, K2). A model is downloaded only when the
//! owner asks, from a pinned revision, and every file is checked against a
//! pinned SHA-256 digest before it is used. The model then runs in the node:
//! no content leaves the computer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::fetch;
use crate::error::SessionError;

/// One file of a model, as published at its revision.
#[derive(Debug)]
pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// A model the node can download and run.
#[derive(Debug)]
pub struct ModelInfo {
    /// Stable id, as collections record it.
    pub id: &'static str,
    pub name: &'static str,
    /// Where it is published (Hugging Face).
    pub repo: &'static str,
    /// The commit its files are taken from.
    pub revision: &'static str,
    pub license: &'static str,
    /// Length of its vectors.
    pub dims: usize,
    pub files: &'static [ModelFile],
    /// The task a query is searched for; documents get none (Qwen3-Embedding).
    pub query_task: &'static str,
    /// The Lance column holding its vectors.
    pub column: &'static str,
}

impl ModelInfo {
    /// Bytes to download.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The id and revision a conversion records.
    pub(crate) fn tag(&self) -> String {
        format!("{}@{}", self.id, &self.revision[..7])
    }
}

/// The models this node knows. Qwen3-Embedding-0.6B: Apache-2.0, Chinese
/// and English, 1024 dimensions (decided 2026-11-17).
pub static MODELS: &[ModelInfo] = &[ModelInfo {
    id: "qwen3-embedding-0.6b",
    name: "Qwen3-Embedding-0.6B",
    repo: "Qwen/Qwen3-Embedding-0.6B",
    revision: "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
    license: "Apache-2.0",
    dims: 1024,
    files: &[
        ModelFile {
            name: "config.json",
            size: 727,
            sha256: "b5bf1f51fc45be473a54718cef92448d90a1be001bf9b9a44b8c7f10a19feaa9",
        },
        ModelFile {
            name: "tokenizer.json",
            size: 11_423_705,
            sha256: "def76fb086971c7867b829c23a26261e38d9d74e02139253b38aeb9df8b4b50a",
        },
        ModelFile {
            name: "model.safetensors",
            size: 1_191_586_416,
            sha256: "0437e45c94563b09e13cb7a64478fc406947a93cb34a7e05870fc8dcd48e23fd",
        },
    ],
    query_task: "Given a web search query, retrieve relevant passages that answer the query",
    column: "vec_qwen3_embedding_0_6b",
}];

/// The model with `id`.
pub fn model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// Where a model is on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelState {
    Absent,
    Downloading {
        done: u64,
        total: u64,
    },
    Ready,
    /// The last download failed; downloading again resumes it.
    Failed(String),
}

/// Downloads running or failed, by model directory.
static DOWNLOADS: Mutex<Option<HashMap<PathBuf, ModelState>>> = Mutex::new(None);

fn downloads<T>(f: impl FnOnce(&mut HashMap<PathBuf, ModelState>) -> T) -> T {
    let mut guard = DOWNLOADS.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The directory of `model` under `root`.
pub fn dir(root: &Path, model: &ModelInfo) -> PathBuf {
    root.join(model.id).join(model.revision)
}

/// Written once every file was checked, so a start need not hash 1 GB.
const VERIFIED: &str = "verified";

fn complete(dir: &Path, model: &ModelInfo) -> bool {
    dir.join(VERIFIED).exists()
        && model.files.iter().all(|f| {
            std::fs::metadata(dir.join(f.name))
                .map(|m| m.len() == f.size)
                .unwrap_or(false)
        })
}

/// Where `model` is now.
pub(crate) fn state(root: &Path, model: &ModelInfo) -> ModelState {
    let dir = dir(root, model);
    if let Some(running) = downloads(|d| d.get(&dir).cloned()) {
        if matches!(running, ModelState::Downloading { .. }) || !complete(&dir, model) {
            return running;
        }
    }
    if complete(&dir, model) {
        ModelState::Ready
    } else {
        ModelState::Absent
    }
}

/// Start downloading `model` in the background; nothing happens when it is
/// already here or on its way. Returns its state after.
pub(crate) fn start(root: &Path, model: &'static ModelInfo) -> ModelState {
    let dir = dir(root, model);
    if complete(&dir, model) {
        return ModelState::Ready;
    }
    let total = model.size();
    let claimed = downloads(|d| {
        if matches!(d.get(&dir), Some(ModelState::Downloading { .. })) {
            return false;
        }
        d.insert(dir.clone(), ModelState::Downloading { done: 0, total });
        true
    });
    if claimed {
        let task_dir = dir.clone();
        tokio::spawn(async move {
            let result = download(&task_dir, model).await;
            downloads(|d| match result {
                Ok(()) => {
                    d.remove(&task_dir);
                }
                Err(e) => {
                    tracing::warn!(model = model.id, error = %e, "downloading a model");
                    d.insert(task_dir.clone(), ModelState::Failed(e.to_string()));
                }
            });
        });
    }
    state(root, model)
}

/// Remove `model`'s files. Refused while it downloads.
pub(crate) fn remove(root: &Path, model: &ModelInfo) -> Result<(), SessionError> {
    let dir = dir(root, model);
    if matches!(
        downloads(|d| d.get(&dir).cloned()),
        Some(ModelState::Downloading { .. })
    ) {
        return Err(SessionError::Unsupported(format!(
            "{} is downloading; remove it when the download is done",
            model.name
        )));
    }
    downloads(|d| d.remove(&dir));
    let all = root.join(model.id);
    if all.exists() {
        std::fs::remove_dir_all(&all).map_err(|e| io(&all, e))?;
    }
    Ok(())
}

fn io(path: &Path, e: std::io::Error) -> SessionError {
    SessionError::Unsupported(format!("{}: {e}", path.display()))
}

fn progress(dir: &Path, done: u64, total: u64) {
    downloads(|d| {
        d.insert(dir.to_path_buf(), ModelState::Downloading { done, total });
    });
}

async fn download(dir: &Path, model: &ModelInfo) -> Result<(), SessionError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| io(dir, e))?;
    let client = fetch::client()?;
    let total = model.size();
    let mut before = 0;
    for file in model.files {
        get(&client, dir, model, file, before, total).await?;
        before += file.size;
    }
    tokio::fs::write(dir.join(VERIFIED), b"")
        .await
        .map_err(|e| io(dir, e))?;
    Ok(())
}

/// Download one file of `model`, resuming a part left by an earlier try,
/// and keep it only when its size and digest are the published ones.
async fn get(
    client: &reqwest::Client,
    dir: &Path,
    model: &ModelInfo,
    file: &ModelFile,
    before: u64,
    total: u64,
) -> Result<(), SessionError> {
    let target = dir.join(file.name);
    if tokio::fs::metadata(&target)
        .await
        .is_ok_and(|m| m.len() == file.size)
        && digest_of(&target).await? == file.sha256
    {
        progress(dir, before + file.size, total);
        return Ok(());
    }
    let part = dir.join(format!("{}.part", file.name));
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        model.repo, model.revision, file.name
    );
    let got = fetch::fetch(client, &url, &part, Some(file.size), |done, _| {
        progress(dir, before + done, total)
    })
    .await?;
    if got.size != file.size || got.sha256 != file.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(SessionError::Unsupported(format!(
            "{} of {} did not match its published size and digest; it was deleted",
            file.name, model.name
        )));
    }
    tokio::fs::rename(&part, &target)
        .await
        .map_err(|e| io(&target, e))?;
    Ok(())
}

async fn digest_of(path: &Path) -> Result<String, SessionError> {
    let mut hasher = Sha256::new();
    fetch::hash_into(path, &mut hasher).await?;
    Ok(fetch::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_pinned() {
        for m in MODELS {
            assert_eq!(m.revision.len(), 40, "{}", m.id);
            for f in m.files {
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.name);
                assert!(f.size > 0);
            }
            assert!(m.column.starts_with("vec_"));
            assert!(m
                .column
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
        }
        assert_eq!(model("qwen3-embedding-0.6b").map(|m| m.dims), Some(1024));
        assert!(model("nothing").is_none());
    }

    #[test]
    fn a_model_is_ready_only_when_every_file_was_checked() {
        let root = tempfile::tempdir().unwrap();
        let m = &MODELS[0];
        assert_eq!(state(root.path(), m), ModelState::Absent);
        let d = dir(root.path(), m);
        std::fs::create_dir_all(&d).unwrap();
        // Right sizes but never checked: not ready.
        for f in m.files {
            let file = std::fs::File::create(d.join(f.name)).unwrap();
            file.set_len(f.size).unwrap();
        }
        assert_eq!(state(root.path(), m), ModelState::Absent);
        std::fs::write(d.join(VERIFIED), b"").unwrap();
        assert_eq!(state(root.path(), m), ModelState::Ready);
        remove(root.path(), m).unwrap();
        assert_eq!(state(root.path(), m), ModelState::Absent);
        assert!(!root.path().join(m.id).exists());
    }
}
