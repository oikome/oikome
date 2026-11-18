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

//! Text to vectors with a downloaded model, on this computer's CPU (K2).
//!
//! Qwen3-Embedding reads a text followed by `<|endoftext|>` (its tokenizer
//! adds it) and the vector is the last position's hidden state, normalized.
//! A query is put after an instruction naming the task; a document is not.
//! Each text runs on its own, so no padding or attention mask is needed.
//!
//! A loaded model holds about 2.4 GB of memory; it is let go after
//! [`IDLE`] without use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config, Model};
use tokenizers::Tokenizer;

use super::model::ModelInfo;
use crate::error::SessionError;

/// A text longer than this many tokens is cut; chunks are far shorter.
const MAX_TOKENS: usize = 2048;
/// A loaded model is let go after this long unused.
const IDLE: Duration = Duration::from_secs(300);

fn err(e: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("embedding model: {e}"))
}

pub(crate) struct Embedder {
    model: Model,
    tokenizer: Tokenizer,
    eos: u32,
    dims: usize,
    query_task: &'static str,
    /// One text at a time: the CPU's threads are already all in use.
    busy: Mutex<()>,
}

impl Embedder {
    fn load(dir: &Path, info: &ModelInfo) -> Result<Self, SessionError> {
        let config: Config =
            serde_json::from_slice(&std::fs::read(dir.join("config.json")).map_err(err)?)
                .map_err(err)?;
        let device = Device::Cpu;
        // The weights are published without the `model.` prefix candle's
        // Qwen3 reads them under.
        let tensors: HashMap<String, Tensor> =
            candle_core::safetensors::load(dir.join("model.safetensors"), &device)
                .map_err(err)?
                .into_iter()
                .map(|(name, t)| (format!("model.{name}"), t))
                .collect();
        let model = Model::new(
            &config,
            VarBuilder::from_tensors(tensors, DType::F32, &device),
        )
        .map_err(err)?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;
        let eos = tokenizer
            .token_to_id("<|endoftext|>")
            .ok_or_else(|| err("the tokenizer has no <|endoftext|>"))?;
        Ok(Self {
            model,
            tokenizer,
            eos,
            dims: info.dims,
            query_task: info.query_task,
            busy: Mutex::new(()),
        })
    }

    /// The vector of a document's text.
    pub(crate) fn document(&self, text: &str) -> Result<Vec<f32>, SessionError> {
        self.embed(text)
    }

    /// The vector of a query, after its task.
    pub(crate) fn query(&self, text: &str) -> Result<Vec<f32>, SessionError> {
        self.embed(&format!("Instruct: {}\nQuery:{text}", self.query_task))
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, SessionError> {
        let encoding = self.tokenizer.encode(text, true).map_err(err)?;
        let mut ids = encoding.get_ids().to_vec();
        if ids.len() > MAX_TOKENS {
            ids.truncate(MAX_TOKENS - 1);
        }
        if ids.last() != Some(&self.eos) {
            ids.push(self.eos);
        }
        let _one = self.busy.lock().unwrap_or_else(|p| p.into_inner());
        let input = Tensor::new(ids.as_slice(), &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(err)?;
        // A fresh copy each time: the model keeps a key-value cache across
        // calls, and its weights are shared, not copied.
        let mut model = self.model.clone();
        let hidden = model.forward(&input, 0).map_err(err)?;
        let last: Vec<f32> = hidden
            .i((0, ids.len() - 1))
            .and_then(|t| t.to_dtype(DType::F32))
            .and_then(|t| t.to_vec1())
            .map_err(err)?;
        if last.len() != self.dims {
            return Err(err(format!(
                "a vector of {} numbers, not {}",
                last.len(),
                self.dims
            )));
        }
        Ok(normalized(last))
    }
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

struct Loaded {
    dir: PathBuf,
    embedder: Arc<Embedder>,
    used: Instant,
}

static LOADED: tokio::sync::Mutex<Option<Loaded>> = tokio::sync::Mutex::const_new(None);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// The model in `dir`, loaded once and kept while it is used.
pub(crate) async fn embedder(
    dir: &Path,
    info: &'static ModelInfo,
) -> Result<Arc<Embedder>, SessionError> {
    let mut slot = LOADED.lock().await;
    if let Some(loaded) = slot.as_mut().filter(|l| l.dir == dir) {
        loaded.used = Instant::now();
        return Ok(loaded.embedder.clone());
    }
    // Let go of another model before reading this one.
    *slot = None;
    let path = dir.to_path_buf();
    let embedder = tokio::task::spawn_blocking(move || Embedder::load(&path, info))
        .await
        .map_err(err)??;
    let embedder = Arc::new(embedder);
    *slot = Some(Loaded {
        dir: dir.to_path_buf(),
        embedder: embedder.clone(),
        used: Instant::now(),
    });
    if !WATCHING.swap(true, Ordering::SeqCst) {
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut slot = LOADED.lock().await;
                if slot.as_ref().is_none_or(|l| l.used.elapsed() >= IDLE) {
                    *slot = None;
                    WATCHING.store(false, Ordering::SeqCst);
                    return;
                }
            }
        });
    }
    Ok(embedder)
}

/// Let go of a loaded model, as when its files are removed.
pub(crate) async fn unload() {
    *LOADED.lock().await = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_normalized() {
        let v = normalized(vec![3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        assert_eq!(normalized(vec![0.0, 0.0]), vec![0.0, 0.0]);
    }
}
