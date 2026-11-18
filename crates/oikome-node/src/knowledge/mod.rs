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

//! Knowledge an AI can read (docs/knowledge-schema.md; roadmap PR 28, K1).
//!
//! The owner prepares a collection whose files are viewed in place (or were
//! copied into one): its Markdown, text, code and PDF files are split into
//! chunks that keep their place in the original, written to the collection's
//! Lance dataset and indexed for full text. The originals are only read.
//!
//! - `catalog.rs`: the control plane (assets, conversions as the queue,
//!   chunks turned off), `.oikome/knowledge/<space>/catalog.sqlite`;
//! - `split.rs`: how an original becomes chunks;
//! - `store.rs`: the chunks in Lance, through DuckDB's `lance` extension;
//! - `skills.rs`: skills (folders with `SKILL.md`) assistants may read;
//! - `model.rs` and `embed.rs` (feature `embedding`, K2): a local embedding
//!   model, downloaded when the owner asks, that gives chunks vectors so a
//!   collection can be searched by meaning as well as by words.
//!
//! A prepared collection is kept up to date while the node serves: changed
//! files are converted again, files that went lose their chunks.

mod catalog;
#[cfg(feature = "embedding")]
mod embed;
#[cfg(feature = "embedding")]
pub mod model;
pub mod skills;
mod split;
mod store;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray};
use oikome_catalog::Location;
use oikome_engine::Capabilities;

pub use catalog::{AssetState, Stage};
pub use store::{FoundBy, StoredChunk};

use crate::error::SessionError;
use crate::inplace::{self, Place};
use crate::session::OikomeSession;
use catalog::{Catalog, Job};
use split::{Kind, Piece};
use store::Store;

/// Changes when splitting changes, so every original is converted again.
const VERSION: &str = "k1-1";
/// Text files larger than this are not read for knowledge…
const MAX_TEXT: u64 = 8 << 20;
/// …nor PDF files larger than this.
const MAX_PDF: u64 = 64 << 20;
/// A read returns at most this many characters.
const MAX_READ: usize = 20_000;
/// Chunks embedded and kept together.
#[cfg(feature = "embedding")]
const EMBED_BATCH: usize = 16;

/// Collections whose queue is being worked now, by knowledge directory: one
/// worker at a time, so no conversion is taken twice.
static WORKING: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

struct Working(PathBuf);

impl Working {
    fn start(dir: PathBuf) -> Option<Working> {
        let mut set = WORKING.lock().unwrap_or_else(|p| p.into_inner());
        set.get_or_insert_with(HashSet::new)
            .insert(dir.clone())
            .then_some(Working(dir))
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        let mut set = WORKING.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(set) = set.as_mut() {
            set.remove(&self.0);
        }
    }
}

/// Why an original could not be prepared, as a code the apps put in words.
pub mod reason {
    /// Larger than knowledge reads.
    pub const TOO_LARGE: &str = "too_large";
    /// Not UTF-8 text.
    pub const NOT_TEXT: &str = "not_text";
    /// A PDF without a text layer (a scan): needs OCR, a later step.
    pub const NO_TEXT: &str = "no_text";
    /// The file could not be read or parsed.
    pub const UNREADABLE: &str = "unreadable";
    /// The collection is searched by meaning, but its model is not on this
    /// computer (it was removed): download it again, then retry.
    pub const NO_MODEL: &str = "no_model";
}

/// A collection's knowledge: whether it was prepared, and each original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeStatus {
    pub prepared_ms: Option<i64>,
    /// The model it is searched by meaning with, if any.
    pub model: Option<String>,
    pub assets: Vec<AssetState>,
}

/// What `read_knowledge` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeText {
    pub path: String,
    pub text: String,
    pub page: Option<u32>,
    /// The characters returned, of the original's text (of the page's for a
    /// PDF), end exclusive.
    pub char_start: u64,
    pub char_end: u64,
    /// Characters of the whole text (the original's, or the chunks' when
    /// read by chunk), to say what was left out.
    pub total_chars: u64,
}

/// What part of an original `read_knowledge` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeRange {
    /// A chunk and `around` chunks on either side of it.
    Chunk { chunk_id: String, around: u32 },
    /// Characters, end exclusive; of `page` for a PDF.
    Chars {
        page: Option<u32>,
        start: u64,
        end: u64,
    },
    /// Lines from 1, both inclusive; of `page` for a PDF.
    Lines {
        page: Option<u32>,
        start: u32,
        end: u32,
    },
}

/// How knowledge is searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Chunks holding every word of the query.
    Words,
    /// Chunks nearest the query's meaning (K2).
    Meaning,
    /// Both, fused by rank.
    Hybrid,
}

/// A search's hits, best first, how it searched and how long it took.
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeSearch {
    pub hits: Vec<StoredChunk>,
    pub mode: SearchMode,
    pub took_ms: u64,
}

/// The model a collection's chunks get vectors from, and where models are.
#[cfg(feature = "embedding")]
struct Vectors {
    info: &'static model::ModelInfo,
    root: PathBuf,
}

/// Without the `embedding` feature there are none.
#[cfg(not(feature = "embedding"))]
enum Vectors {}

impl Vectors {
    /// Its Lance column and the length of its vectors.
    fn column(&self) -> (&'static str, usize) {
        #[cfg(feature = "embedding")]
        return (self.info.column, self.info.dims);
        #[cfg(not(feature = "embedding"))]
        match *self {}
    }
}

/// The conversion version: splitting's, and the model's when there is one,
/// so choosing or changing a model converts every original again.
fn knowledge_version(model: Option<&str>) -> String {
    #[cfg(feature = "embedding")]
    if let Some(m) = model.and_then(model::model) {
        return format!("{VERSION}+{}", m.tag());
    }
    #[cfg(not(feature = "embedding"))]
    let _ = model;
    VERSION.to_string()
}

impl OikomeSession {
    fn knowledge_dir(&self, space: &str) -> PathBuf {
        self.catalog.oikome_dir().join("knowledge").join(space)
    }

    /// The location of a collection whose files can be prepared: a folder
    /// viewed in place, or files copied into one.
    fn knowledge_location(&self, space: &str) -> Result<Location, SessionError> {
        self.in_place_location(space)?.ok_or_else(|| {
            SessionError::Unsupported(format!(
                "{space} holds no files to prepare; tables become knowledge in a later step"
            ))
        })
    }

    async fn knowledge_store(&self, space: &str) -> Result<Store, SessionError> {
        Store::open(
            self.engines().duckdb()?,
            space,
            &self.knowledge_dir(space).join("lance"),
        )
        .await
    }

    fn knowledge_catalog(&self, space: &str) -> Result<Catalog, SessionError> {
        Catalog::open(&self.knowledge_dir(space))
    }

    /// Whether this node can keep knowledge: it needs DuckDB's `lance`
    /// extension, installed with `install_extensions`.
    pub async fn knowledge_available(&self) -> Result<(), SessionError> {
        self.engines()
            .duckdb()?
            .enable(Capabilities::READ_LANCE)
            .await?;
        Ok(())
    }

    /// Prepare `space` for AI: list its originals and queue them. Owner
    /// only. The conversions run with [`OikomeSession::work_knowledge`].
    pub async fn prepare_knowledge(&self, space: &str) -> Result<KnowledgeStatus, SessionError> {
        self.deny_principal("preparing knowledge")?;
        let location = self.knowledge_location(space)?;
        // Fails with how to install the extension when it is missing.
        self.knowledge_store(space).await?;
        let cat = self.knowledge_catalog(space)?;
        cat.mark_prepared()?;
        self.reconcile_knowledge(space, &location, &cat).await?;
        self.knowledge_status(space)
    }

    async fn reconcile_knowledge(
        &self,
        space: &str,
        location: &Location,
        cat: &Catalog,
    ) -> Result<(), SessionError> {
        let files: Vec<_> = inplace::indexed_files(&self.catalog, location)?
            .into_iter()
            .map(|f| (f.path, f.name, f.size, f.modified_ms))
            .collect();
        let gone = cat.reconcile(
            space,
            &files,
            |name| Kind::of(name).map(Kind::converter),
            &knowledge_version(cat.model()?.as_deref()),
        )?;
        if !gone.is_empty() {
            let store = self.knowledge_store(space).await?;
            for asset in &gone {
                store.remove(asset).await?;
            }
        }
        Ok(())
    }

    /// Search `space` by meaning with `model` as well as by words, or by
    /// words only (`None`). Its originals are converted again, the chunks
    /// they have now serving until then. The model must be on this computer.
    /// Owner only.
    #[cfg(feature = "embedding")]
    pub async fn set_knowledge_model(
        &self,
        space: &str,
        model: Option<&str>,
    ) -> Result<KnowledgeStatus, SessionError> {
        self.deny_principal("changing knowledge")?;
        let location = self.knowledge_location(space)?;
        if let Some(id) = model {
            let info = model::model(id)
                .ok_or_else(|| SessionError::Unsupported(format!("no model {id}")))?;
            if model::state(&self.engines().models_dir(), info) != model::ModelState::Ready {
                return Err(SessionError::Unsupported(format!(
                    "{} is not on this computer; download it first",
                    info.name
                )));
            }
        }
        let cat = self.knowledge_catalog(space)?;
        cat.set_model(model)?;
        if cat.prepared_ms()?.is_some() {
            self.reconcile_knowledge(space, &location, &cat).await?;
        }
        self.knowledge_status(space)
    }

    /// The embedding models this node knows and where each is.
    #[cfg(feature = "embedding")]
    pub fn embedding_models(&self) -> Vec<(&'static model::ModelInfo, model::ModelState)> {
        let root = self.engines().models_dir();
        model::MODELS
            .iter()
            .map(|m| (m, model::state(&root, m)))
            .collect()
    }

    /// Start downloading model `id` in the background. Owner only.
    #[cfg(feature = "embedding")]
    pub fn download_embedding_model(&self, id: &str) -> Result<model::ModelState, SessionError> {
        self.deny_principal("downloading a model")?;
        let info =
            model::model(id).ok_or_else(|| SessionError::Unsupported(format!("no model {id}")))?;
        Ok(model::start(&self.engines().models_dir(), info))
    }

    /// Remove model `id`'s files. Collections searched by meaning with it
    /// keep their vectors but are searched by words until it is back. Owner
    /// only.
    #[cfg(feature = "embedding")]
    pub async fn remove_embedding_model(&self, id: &str) -> Result<(), SessionError> {
        self.deny_principal("removing a model")?;
        let info =
            model::model(id).ok_or_else(|| SessionError::Unsupported(format!("no model {id}")))?;
        embed::unload().await;
        model::remove(&self.engines().models_dir(), info)
    }

    /// Where `space`'s knowledge is: prepared or not, and each original.
    pub fn knowledge_status(&self, space: &str) -> Result<KnowledgeStatus, SessionError> {
        self.space(space)?;
        if !self.knowledge_dir(space).join("catalog.sqlite").exists() {
            return Ok(KnowledgeStatus {
                prepared_ms: None,
                model: None,
                assets: Vec::new(),
            });
        }
        let cat = self.knowledge_catalog(space)?;
        Ok(KnowledgeStatus {
            prepared_ms: cat.prepared_ms()?,
            model: cat.model()?,
            assets: cat.states()?,
        })
    }

    /// Queue every original that failed again. Owner only.
    pub fn retry_knowledge(&self, space: &str) -> Result<u64, SessionError> {
        self.deny_principal("preparing knowledge")?;
        let cat = self.knowledge_catalog(space)?;
        cat.retry_failed(&knowledge_version(cat.model()?.as_deref()))
    }

    /// The model `space`'s chunks get vectors from, whether or not its
    /// files are on this computer yet.
    fn knowledge_vectors(&self, cat: &Catalog) -> Result<Option<Vectors>, SessionError> {
        #[cfg(feature = "embedding")]
        return Ok(cat
            .model()?
            .as_deref()
            .and_then(model::model)
            .map(|info| Vectors {
                info,
                root: self.engines().models_dir(),
            }));
        #[cfg(not(feature = "embedding"))]
        {
            let _ = cat;
            Ok(None)
        }
    }

    /// The model `space` is searched by meaning with, when its files are here.
    fn knowledge_meaning(&self, cat: &Catalog) -> Result<Option<Vectors>, SessionError> {
        #[cfg(feature = "embedding")]
        return Ok(self
            .knowledge_vectors(cat)?
            .filter(|v| model::state(&v.root, v.info) == model::ModelState::Ready));
        #[cfg(not(feature = "embedding"))]
        self.knowledge_vectors(cat)
    }

    /// Run `space`'s queued conversions for at most `budget`, then index
    /// what was written. Returns how many originals were converted.
    pub async fn work_knowledge(&self, space: &str, budget: Duration) -> Result<u64, SessionError> {
        self.deny_principal("preparing knowledge")?;
        let location = self.knowledge_location(space)?;
        let Some(_working) = Working::start(self.knowledge_dir(space)) else {
            // Another worker has it.
            return Ok(0);
        };
        let place = inplace::place_of(&self.catalog, self.engines(), &location)?;
        if !place.reachable().await {
            // Nothing is tried, so nothing fails, while the place is away.
            return Ok(0);
        }
        let cat = self.knowledge_catalog(space)?;
        let vectors = self.knowledge_vectors(&cat)?;
        let started = Instant::now();
        let mut store = None;
        let mut done = 0;
        while started.elapsed() < budget {
            let Some(job) = cat.next_job()? else {
                break;
            };
            if store.is_none() {
                let opened = self.knowledge_store(space).await?;
                if let Some(v) = &vectors {
                    let (column, dims) = v.column();
                    opened.ensure_vectors(column, dims).await?;
                }
                store = Some(opened);
            }
            let store = store.as_ref().expect("opened above");
            match convert(&cat, store, &place, &job, vectors.as_ref()).await {
                Ok(()) => done += 1,
                Err(Failure::Reason(reason)) => cat.fail(&job, reason)?,
                Err(Failure::Error(e)) => {
                    tracing::warn!(space, path = %job.path, error = %e, "preparing knowledge");
                    cat.fail(&job, reason::UNREADABLE)?;
                }
            }
        }
        if let Some(store) = store {
            store.reindex().await?;
        }
        Ok(done)
    }

    /// Keep every prepared collection up to date: list its originals again
    /// and convert what changed, within `budget` overall.
    pub async fn refresh_knowledge(&self, budget: Duration) {
        let started = Instant::now();
        for space in self.catalog.list_spaces() {
            if started.elapsed() >= budget {
                break;
            }
            let dir = self.knowledge_dir(&space.name);
            if !dir.join("catalog.sqlite").exists() {
                continue;
            }
            let result = async {
                let cat = self.knowledge_catalog(&space.name)?;
                if cat.prepared_ms()?.is_none() {
                    return Ok(0);
                }
                let location = self.knowledge_location(&space.name)?;
                self.reconcile_knowledge(&space.name, &location, &cat)
                    .await?;
                self.work_knowledge(&space.name, budget.saturating_sub(started.elapsed()))
                    .await
            }
            .await;
            if let Err(e) = result {
                tracing::warn!(space = %space.name, error = %e, "keeping knowledge up to date");
            }
        }
    }

    /// The current chunks of the original at `path`, in order, each with
    /// whether it is turned on.
    pub async fn knowledge_chunks(
        &self,
        space: &str,
        path: &str,
    ) -> Result<Vec<(StoredChunk, bool)>, SessionError> {
        let cat = self.knowledge_catalog(space)?;
        let Some(asset) = cat.asset_at(path)? else {
            return Ok(Vec::new());
        };
        let Some(conversion) = cat.current()?.remove(&asset) else {
            return Ok(Vec::new());
        };
        let off = cat.disabled()?;
        Ok(self
            .knowledge_store(space)
            .await?
            .conversion(&conversion)
            .await?
            .into_iter()
            .map(|c| {
                let on = !off.contains(&c.chunk_id);
                (c, on)
            })
            .collect())
    }

    /// Turn a chunk off (it is kept, but never returned) or on. Owner only.
    pub fn set_knowledge_chunk(
        &self,
        space: &str,
        chunk_id: &str,
        enabled: bool,
    ) -> Result<(), SessionError> {
        self.deny_principal("changing knowledge")?;
        self.knowledge_catalog(space)?
            .set_enabled(chunk_id, enabled)
    }

    /// Up to `limit` current, turned-on chunks of `space` matching `query`,
    /// best first: by words and meaning when the collection has a model
    /// whose files are here, by words otherwise.
    pub async fn search_knowledge(
        &self,
        space: &str,
        query: &str,
        limit: u32,
    ) -> Result<KnowledgeSearch, SessionError> {
        self.search_knowledge_by(space, query, limit, None).await
    }

    /// [`OikomeSession::search_knowledge`] the way `mode` says; meaning needs
    /// the collection's model.
    pub async fn search_knowledge_by(
        &self,
        space: &str,
        query: &str,
        limit: u32,
        mode: Option<SearchMode>,
    ) -> Result<KnowledgeSearch, SessionError> {
        let started = Instant::now();
        let status = self.knowledge_status(space)?;
        let cat = self.knowledge_catalog(space)?;
        let meaning = self.knowledge_meaning(&cat)?;
        let mode = match (mode, meaning.is_some()) {
            (Some(SearchMode::Words), _) | (None, false) => SearchMode::Words,
            (None, true) => SearchMode::Hybrid,
            (Some(m), true) => m,
            (Some(_), false) => {
                return Err(SessionError::Unsupported(format!(
                    "{space} is not searched by meaning; turn it on with a model first"
                )))
            }
        };
        let query = query.trim();
        if status.prepared_ms.is_none() || query.is_empty() {
            return Ok(KnowledgeSearch {
                hits: Vec::new(),
                mode,
                took_ms: 0,
            });
        }
        let current = cat.current()?;
        let off = cat.disabled()?;
        let keep = |c: &StoredChunk| {
            current.get(&c.asset_id) == Some(&c.conversion_id) && !off.contains(&c.chunk_id)
        };
        // Rows of replaced versions and turned-off chunks are dropped, so
        // ask for more than are wanted.
        let wanted = limit.clamp(1, 100);
        let ask = wanted * 3 + off.len().min(300) as u32;
        let store = self.knowledge_store(space).await?;
        let by_words = if mode == SearchMode::Meaning {
            Vec::new()
        } else {
            // The ngram index also scores chunks that share only a few
            // letters with the query; by words means each word is there.
            let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
            store
                .search(query, ask)
                .await?
                .into_iter()
                .filter(|c| keep(c))
                .filter(|c| {
                    let text = c.text.to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .collect()
        };
        let by_meaning = match (&meaning, mode) {
            (Some(v), SearchMode::Meaning | SearchMode::Hybrid) => {
                let (column, dims) = v.column();
                store.ensure_vectors(column, dims).await?;
                let vector = query_vector(v, query).await?;
                store
                    .nearest(column, &vector, ask)
                    .await?
                    .into_iter()
                    .filter(|c| keep(c))
                    .collect()
            }
            _ => Vec::new(),
        };
        let hits = match mode {
            SearchMode::Words => by_words.into_iter().take(wanted as usize).collect(),
            SearchMode::Meaning => by_meaning.into_iter().take(wanted as usize).collect(),
            SearchMode::Hybrid => fuse(by_words, by_meaning, wanted as usize),
        };
        Ok(KnowledgeSearch {
            hits,
            mode,
            took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// Part of an original's text: a chunk and its neighbours, or a range
    /// of its characters or lines, at most [`MAX_READ`] characters.
    pub async fn read_knowledge(
        &self,
        space: &str,
        path: &str,
        range: KnowledgeRange,
    ) -> Result<KnowledgeText, SessionError> {
        let cat = self.knowledge_catalog(space)?;
        if let KnowledgeRange::Chunk { chunk_id, around } = &range {
            let store = self.knowledge_store(space).await?;
            let current = cat.current()?;
            let chunk = store
                .chunk(chunk_id)
                .await?
                .filter(|c| c.path == path)
                .filter(|c| current.get(&c.asset_id) == Some(&c.conversion_id))
                .ok_or_else(|| not_found(path))?;
            // A chunk turned off is never given to an assistant.
            let off = cat.disabled()?;
            if off.contains(&chunk.chunk_id) {
                return Err(not_found(path));
            }
            let around = i32::try_from(*around).unwrap_or(0).min(5);
            let near: Vec<_> = store
                .conversion(&chunk.conversion_id)
                .await?
                .into_iter()
                .filter(|c| c.page == chunk.page)
                .filter(|c| (c.seq - chunk.seq).abs() <= around)
                .filter(|c| !off.contains(&c.chunk_id))
                .collect();
            let text = near
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let total = near.iter().map(|c| c.text.chars().count() as u64).sum();
            return Ok(KnowledgeText {
                path: path.to_string(),
                page: chunk.page.and_then(|p| u32::try_from(p).ok()),
                char_start: near.first().map_or(0, |c| c.char_start as u64),
                char_end: near.last().map_or(0, |c| c.char_end as u64),
                total_chars: total,
                text: cut(&text, MAX_READ),
            });
        }
        if cat.asset_at(path)?.is_none() {
            return Err(not_found(path));
        }
        let location = self.knowledge_location(space)?;
        let place = inplace::place_of(&self.catalog, self.engines(), &location)?;
        let page = match &range {
            KnowledgeRange::Chars { page, .. } | KnowledgeRange::Lines { page, .. } => *page,
            KnowledgeRange::Chunk { .. } => None,
        };
        let text = original_text(&place, path, page)
            .await
            .map_err(|f| match f {
                Failure::Reason(r) => SessionError::Unsupported(format!("{path}: {r}")),
                Failure::Error(e) => e,
            })?;
        let chars: Vec<char> = text.chars().collect();
        let total = chars.len() as u64;
        let (start, end) = match range {
            KnowledgeRange::Chars { start, end, .. } => {
                (start.min(total), end.min(total).max(start.min(total)))
            }
            KnowledgeRange::Lines { start, end, .. } => line_span(&chars, start, end),
            KnowledgeRange::Chunk { .. } => unreachable!("handled above"),
        };
        let end = end.min(start + MAX_READ as u64);
        Ok(KnowledgeText {
            path: path.to_string(),
            text: chars[start as usize..end as usize].iter().collect(),
            page,
            char_start: start,
            char_end: end,
            total_chars: total,
        })
    }

    /// The first chunks of the first original that can be prepared, without
    /// writing anything: what Prepare will make.
    pub async fn preview_knowledge(
        &self,
        space: &str,
        limit: usize,
    ) -> Result<Vec<StoredChunk>, SessionError> {
        self.deny_principal("preparing knowledge")?;
        let location = self.knowledge_location(space)?;
        let place = inplace::place_of(&self.catalog, self.engines(), &location)?;
        for file in inplace::indexed_files(&self.catalog, &location)? {
            let Some(kind) = Kind::of(&file.name) else {
                continue;
            };
            if file.size > max_size(kind) {
                continue;
            }
            let Ok(pages) = pages_of(&place, &file.path, kind, file.size).await else {
                continue;
            };
            let pieces: Vec<_> = pages
                .iter()
                .flat_map(|(page, text)| split::split(text, kind, *page))
                .take(limit)
                .collect();
            if pieces.is_empty() {
                continue;
            }
            return Ok(pieces
                .into_iter()
                .enumerate()
                .map(|(seq, p)| stored(&file.path, i32::try_from(seq).unwrap_or(i32::MAX), p))
                .collect());
        }
        Ok(Vec::new())
    }

    /// Forget `space`'s knowledge, with the collection.
    pub(crate) async fn forget_knowledge(&self, space: &str) -> Result<(), SessionError> {
        let dir = self.knowledge_dir(space);
        if dir.exists() {
            self.engines()
                .duckdb()?
                .detach_space(&format!("oikome-knowledge:{space}"))
                .await?;
            std::fs::remove_dir_all(&dir)
                .map_err(|e| SessionError::Unsupported(format!("forgetting knowledge: {e}")))?;
        }
        Ok(())
    }
}

fn not_found(path: &str) -> SessionError {
    SessionError::Unsupported(format!("{path} is not part of this collection's knowledge"))
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Characters of lines `start..=end` (from 1).
fn line_span(chars: &[char], start: u32, end: u32) -> (u64, u64) {
    let (start, end) = (start.max(1), end.max(start.max(1)));
    let mut line = 1;
    let mut from = None;
    for (i, &c) in chars.iter().enumerate() {
        if line == start && from.is_none() {
            from = Some(i);
        }
        if c == '\n' {
            if line == end {
                return (from.unwrap_or(i) as u64, i as u64);
            }
            line += 1;
        }
    }
    let len = chars.len() as u64;
    (from.map_or(len, |f| f as u64), len)
}

enum Failure {
    /// A known reason, kept for the owner to see.
    Reason(&'static str),
    Error(SessionError),
}

impl From<SessionError> for Failure {
    fn from(e: SessionError) -> Self {
        Failure::Error(e)
    }
}

fn max_size(kind: Kind) -> u64 {
    if kind == Kind::Pdf {
        MAX_PDF
    } else {
        MAX_TEXT
    }
}

/// The text of an original, page by page (one page without a number for
/// anything but a PDF).
async fn pages_of(
    place: &Place,
    path: &str,
    kind: Kind,
    size: u64,
) -> Result<Vec<(Option<u32>, String)>, Failure> {
    let max = max_size(kind);
    if size > max {
        return Err(Failure::Reason(reason::TOO_LARGE));
    }
    let bytes = place.read(path, 0..size, max).await?;
    decode(kind, bytes).await
}

async fn decode(kind: Kind, bytes: Vec<u8>) -> Result<Vec<(Option<u32>, String)>, Failure> {
    if kind != Kind::Pdf {
        let text = String::from_utf8(bytes).map_err(|_| Failure::Reason(reason::NOT_TEXT))?;
        return Ok(vec![(None, text)]);
    }
    // The parser can panic on a damaged file; it runs off the async threads.
    let pages = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text_from_mem_by_pages(&bytes)
        }))
    })
    .await
    .map_err(|_| Failure::Reason(reason::UNREADABLE))?
    .map_err(|_| Failure::Reason(reason::UNREADABLE))?
    .map_err(|_| Failure::Reason(reason::UNREADABLE))?;
    if pages.iter().all(|p| p.trim().is_empty()) {
        return Err(Failure::Reason(reason::NO_TEXT));
    }
    Ok(pages
        .into_iter()
        .enumerate()
        .map(|(i, text)| (Some(u32::try_from(i + 1).unwrap_or(u32::MAX)), text))
        .collect())
}

/// The whole text of an original (of one page of a PDF).
async fn original_text(place: &Place, path: &str, page: Option<u32>) -> Result<String, Failure> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let kind = Kind::of(name).ok_or(Failure::Reason(reason::NOT_TEXT))?;
    let max = max_size(kind);
    let bytes = place.read(path, 0..max, max).await?;
    let pages = decode(kind, bytes).await?;
    let wanted = if kind == Kind::Pdf {
        page.or(Some(1))
    } else {
        None
    };
    pages
        .into_iter()
        .find(|(p, _)| *p == wanted)
        .map(|(_, text)| text)
        .ok_or(Failure::Reason(reason::NO_TEXT))
}

fn stored(path: &str, seq: i32, p: Piece) -> StoredChunk {
    StoredChunk {
        chunk_id: String::new(),
        asset_id: String::new(),
        conversion_id: String::new(),
        path: path.to_string(),
        seq,
        heading: p.heading,
        text: p.text,
        page: p.page.and_then(|n| i32::try_from(n).ok()),
        char_start: i64::try_from(p.char_start).unwrap_or(i64::MAX),
        char_end: i64::try_from(p.char_end).unwrap_or(i64::MAX),
        line_start: i32::try_from(p.line_start).unwrap_or(i32::MAX),
        line_end: i32::try_from(p.line_end).unwrap_or(i32::MAX),
        score: None,
        found_by: None,
    }
}

/// Convert one original: read it, split it, give its chunks vectors when
/// the collection has a model, commit them.
async fn convert(
    cat: &Catalog,
    store: &Store,
    place: &Place,
    job: &Job,
    vectors: Option<&Vectors>,
) -> Result<(), Failure> {
    cat.stage(&job.conversion_id, Stage::Reading)?;
    let kind = Kind::of(&job.name).ok_or(Failure::Reason(reason::NOT_TEXT))?;
    let max = max_size(kind);
    if job.size > max {
        return Err(Failure::Reason(reason::TOO_LARGE));
    }
    let bytes = place.read(&job.path, 0..job.size, max).await?;
    let version = blake3::hash(&bytes).to_hex().to_string();
    cat.read_version(&job.conversion_id, &job.asset_id, &version)?;
    cat.stage(&job.conversion_id, Stage::Splitting)?;
    let pages = decode(kind, bytes).await?;
    let pieces: Vec<Piece> = pages
        .iter()
        .flat_map(|(page, text)| split::split(text, kind, *page))
        .collect();
    let made = match vectors {
        Some(v) => {
            cat.stage(&job.conversion_id, Stage::Embedding)?;
            Some(vectors_of(cat, v, &pieces).await?)
        }
        None => None,
    };
    let batch = batch_of(job, &version, &pieces, made)?;
    let n = batch.num_rows() as u64;
    store
        .replace(
            &job.asset_id,
            &job.conversion_id,
            batch,
            vectors.map(Vectors::column),
        )
        .await?;
    cat.finish(job, n)?;
    Ok(())
}

/// What a chunk's vector is made from: its headings, then its text (as
/// RAGFlow and FastGPT do), so a passage is found by what it is under.
#[cfg_attr(not(feature = "embedding"), allow(dead_code))]
fn embedding_input(p: &Piece) -> String {
    if p.heading.is_empty() {
        p.text.clone()
    } else {
        format!("{}\n{}", p.heading.join(" > "), p.text)
    }
}

/// A vector as the text DuckDB casts to `FLOAT[n]`.
#[cfg_attr(not(feature = "embedding"), allow(dead_code))]
fn vector_text(v: &[f32]) -> String {
    let numbers: Vec<String> = v.iter().map(|x| format!("{x:?}")).collect();
    format!("[{}]", numbers.join(","))
}

/// Each piece's vector, as text: from the cache when the same input was
/// embedded before, otherwise made now and kept.
#[cfg(feature = "embedding")]
async fn vectors_of(cat: &Catalog, v: &Vectors, pieces: &[Piece]) -> Result<Vec<String>, Failure> {
    if model::state(&v.root, v.info) != model::ModelState::Ready {
        return Err(Failure::Reason(reason::NO_MODEL));
    }
    let tag = v.info.tag();
    let inputs: Vec<String> = pieces.iter().map(embedding_input).collect();
    let hashes: Vec<String> = inputs
        .iter()
        .map(|t| blake3::hash(t.as_bytes()).to_hex().to_string())
        .collect();
    let mut found = cat.cached_vectors(&tag, &hashes)?;
    let missing: Vec<usize> = (0..inputs.len())
        .filter(|&i| !found.contains_key(&hashes[i]))
        .collect();
    if !missing.is_empty() {
        let embedder = embed::embedder(&model::dir(&v.root, v.info), v.info).await?;
        // A few at a time, each kept as it is made, so a long original
        // interrupted half way does not start again from nothing.
        for batch in missing.chunks(EMBED_BATCH) {
            let texts: Vec<String> = batch.iter().map(|&i| inputs[i].clone()).collect();
            let model = embedder.clone();
            let made = tokio::task::spawn_blocking(move || {
                texts
                    .iter()
                    .map(|t| model.document(t))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(|e| SessionError::Unsupported(format!("embedding: {e}")))??;
            for (&i, vector) in batch.iter().zip(made) {
                cat.cache_vector(&tag, &hashes[i], &vector)?;
                found.insert(hashes[i].clone(), vector);
            }
        }
    }
    Ok(hashes.iter().map(|h| vector_text(&found[h])).collect())
}

#[cfg(not(feature = "embedding"))]
async fn vectors_of(_: &Catalog, v: &Vectors, _: &[Piece]) -> Result<Vec<String>, Failure> {
    match *v {}
}

/// The vector of a query, with the model that made the collection's.
#[cfg(feature = "embedding")]
async fn query_vector(v: &Vectors, query: &str) -> Result<Vec<f32>, SessionError> {
    let embedder = embed::embedder(&model::dir(&v.root, v.info), v.info).await?;
    let query = query.to_string();
    tokio::task::spawn_blocking(move || embedder.query(&query))
        .await
        .map_err(|e| SessionError::Unsupported(format!("embedding: {e}")))?
}

#[cfg(not(feature = "embedding"))]
async fn query_vector(v: &Vectors, _: &str) -> Result<Vec<f32>, SessionError> {
    match *v {}
}

/// Hits by words and by meaning in one list, by reciprocal rank fusion:
/// each list adds 1 / (60 + rank), so a chunk both find comes first.
fn fuse(
    by_words: Vec<StoredChunk>,
    by_meaning: Vec<StoredChunk>,
    wanted: usize,
) -> Vec<StoredChunk> {
    const K: f64 = 60.0;
    let mut fused: HashMap<String, (StoredChunk, f64, bool, bool)> = HashMap::new();
    for (meaning, list) in [(false, by_words), (true, by_meaning)] {
        for (rank, chunk) in list.into_iter().enumerate() {
            let add = 1.0 / (K + rank as f64 + 1.0);
            let entry = fused
                .entry(chunk.chunk_id.clone())
                .or_insert_with(|| (chunk, 0.0, false, false));
            entry.1 += add;
            if meaning {
                entry.3 = true;
            } else {
                entry.2 = true;
            }
        }
    }
    let mut hits: Vec<_> = fused
        .into_values()
        .map(|(mut chunk, score, words, meaning)| {
            chunk.score = Some(score);
            chunk.found_by = Some(match (words, meaning) {
                (true, true) => FoundBy::Both,
                (false, true) => FoundBy::Meaning,
                _ => FoundBy::Words,
            });
            chunk
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    hits.truncate(wanted);
    hits
}

fn batch_of(
    job: &Job,
    version: &str,
    pieces: &[Piece],
    vectors: Option<Vec<String>>,
) -> Result<RecordBatch, SessionError> {
    let n = pieces.len();
    let same = |v: &str| StringArray::from(vec![v; n]);
    let ids: Vec<String> = pieces
        .iter()
        .map(|p| {
            let key = format!(
                "{version}\0{VERSION}\0{}\0{}\0{}",
                p.page.unwrap_or(0),
                p.char_start,
                p.char_end
            );
            blake3::hash(key.as_bytes()).to_hex()[..32].to_string()
        })
        .collect();
    let hashes: Vec<String> = pieces
        .iter()
        .map(|p| {
            let normal = p.text.split_whitespace().collect::<Vec<_>>().join(" ");
            blake3::hash(normal.as_bytes()).to_hex().to_string()
        })
        .collect();
    let headings: Vec<String> = pieces
        .iter()
        .map(|p| serde_json::to_string(&p.heading).unwrap_or_else(|_| "[]".into()))
        .collect();
    let int32 = |f: &dyn Fn(&Piece) -> i64| {
        Int32Array::from(
            pieces
                .iter()
                .map(|p| i32::try_from(f(p)).unwrap_or(i32::MAX))
                .collect::<Vec<_>>(),
        )
    };
    let columns: Vec<arrow_array::ArrayRef> = vec![
        std::sync::Arc::new(StringArray::from(ids)),
        std::sync::Arc::new(same(&job.asset_id)),
        std::sync::Arc::new(same(version)),
        std::sync::Arc::new(same(&job.conversion_id)),
        std::sync::Arc::new(same("text")),
        std::sync::Arc::new(same(&job.path)),
        std::sync::Arc::new(Int32Array::from(
            (0..n)
                .map(|i| i32::try_from(i).unwrap_or(i32::MAX))
                .collect::<Vec<_>>(),
        )),
        std::sync::Arc::new(StringArray::from(headings)),
        std::sync::Arc::new(StringArray::from(
            pieces.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
        )),
        std::sync::Arc::new(StringArray::from(hashes)),
        std::sync::Arc::new(Int32Array::from(
            pieces
                .iter()
                .map(|p| p.page.and_then(|n| i32::try_from(n).ok()))
                .collect::<Vec<_>>(),
        )),
        std::sync::Arc::new(Int64Array::from(
            pieces
                .iter()
                .map(|p| i64::try_from(p.char_start).unwrap_or(i64::MAX))
                .collect::<Vec<_>>(),
        )),
        std::sync::Arc::new(Int64Array::from(
            pieces
                .iter()
                .map(|p| i64::try_from(p.char_end).unwrap_or(i64::MAX))
                .collect::<Vec<_>>(),
        )),
        std::sync::Arc::new(int32(&|p| i64::from(p.line_start))),
        std::sync::Arc::new(int32(&|p| i64::from(p.line_end))),
    ];
    match vectors {
        None => Ok(RecordBatch::try_new(store::schema(), columns)?),
        Some(vectors) => {
            let mut fields: Vec<_> = store::schema().fields().iter().cloned().collect();
            fields.push(std::sync::Arc::new(arrow_schema::Field::new(
                "vector",
                arrow_schema::DataType::Utf8,
                false,
            )));
            let mut columns = columns;
            columns.push(std::sync::Arc::new(StringArray::from(vectors)));
            Ok(RecordBatch::try_new(
                std::sync::Arc::new(arrow_schema::Schema::new(fields)),
                columns,
            )?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_found_by_number() {
        let chars: Vec<char> = "one\ntwo\nthree".chars().collect();
        assert_eq!(line_span(&chars, 2, 2), (4, 7));
        assert_eq!(line_span(&chars, 2, 3), (4, 13));
        assert_eq!(line_span(&chars, 9, 9), (13, 13));
    }

    #[tokio::test]
    async fn damaged_pdfs_and_binary_files_say_why() {
        assert!(matches!(
            decode(Kind::Pdf, b"%PDF-1.4\nnot a document".to_vec()).await,
            Err(Failure::Reason(reason::UNREADABLE))
        ));
        assert!(matches!(
            decode(Kind::Text, vec![0xff, 0xfe, 0x00]).await,
            Err(Failure::Reason(reason::NOT_TEXT))
        ));
    }

    fn hit(id: &str) -> StoredChunk {
        StoredChunk {
            chunk_id: id.into(),
            asset_id: "a".into(),
            conversion_id: "c".into(),
            path: "p.md".into(),
            seq: 0,
            heading: Vec::new(),
            text: String::new(),
            page: None,
            char_start: 0,
            char_end: 0,
            line_start: 1,
            line_end: 1,
            score: Some(1.0),
            found_by: None,
        }
    }

    #[test]
    fn words_and_meaning_are_fused_by_rank() {
        let fused = fuse(
            vec![hit("w1"), hit("both"), hit("w3")],
            vec![hit("both"), hit("m2")],
            3,
        );
        let ids: Vec<_> = fused.iter().map(|c| c.chunk_id.as_str()).collect();
        // Found both ways first; then the best of each list by rank.
        assert_eq!(ids, vec!["both", "w1", "m2"]);
        assert_eq!(fused[0].found_by, Some(FoundBy::Both));
        assert_eq!(fused[1].found_by, Some(FoundBy::Words));
        assert_eq!(fused[2].found_by, Some(FoundBy::Meaning));
        assert!(fused[0].score > fused[1].score);
    }

    #[test]
    fn a_chunk_is_embedded_under_its_headings() {
        let piece = Piece {
            text: "Body.".into(),
            heading: vec!["Guide".into(), "Setup".into()],
            page: None,
            char_start: 0,
            char_end: 5,
            line_start: 1,
            line_end: 1,
        };
        assert_eq!(embedding_input(&piece), "Guide > Setup\nBody.");
        assert_eq!(vector_text(&[0.5, -1.0, 1e-8]), "[0.5,-1.0,1e-8]");
    }
}
