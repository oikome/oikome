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

//! The owner's knowledge operations (docs/knowledge-schema.md, PR 28 K1):
//! prepare a collection, see where each original is, look at and turn off
//! chunks, try a search the way an assistant would, and install the Lance
//! support knowledge needs. Assistants read knowledge in assistants.rs.

use std::time::Duration;

use oikome_core::validate_space_name;
use oikome_node::knowledge::support::SupportState;
use oikome_node::knowledge::{self as node, model, skills::Skill, Stage, StoredChunk};

use crate::{
    Api, ApiError, Caller, DownloadState, EmbeddingModel, Empty, ExtensionDownload, FoundBy,
    KnowledgeChunk, KnowledgeChunksRequest, KnowledgeFile, KnowledgeStage, KnowledgeStatus,
    ModelRef, ModelState, RetrievalResults, RetrievalTest, SearchMode, SetKnowledgeChunk,
    SetKnowledgeModel, SkillFileInfo, SkillInfo, SpaceRef,
};

/// A skill as the apps and assistants see it; `collection` is `None` for
/// Oikome's own.
pub(crate) fn skill_info(collection: Option<&str>, s: Skill) -> SkillInfo {
    SkillInfo {
        uri: format!(
            "skill://{}/{}/SKILL.md",
            collection.unwrap_or("oikome"),
            s.name
        ),
        collection: collection.map(str::to_string),
        name: s.name,
        description: s.description,
        license: s.license,
        dir: s.dir,
        files: s
            .files
            .into_iter()
            .map(|f| SkillFileInfo {
                path: f.path,
                size: f.size,
                sha256: f.sha256,
            })
            .collect(),
        problem: s.problem,
    }
}

/// How long one round of preparing runs in the background after Prepare;
/// what is left is taken by the node's minute-by-minute refresh.
const ROUND: Duration = Duration::from_secs(600);
/// A preview shows this many chunks.
const PREVIEW: usize = 5;
/// The longest query a retrieval test takes.
const MAX_QUERY_CHARS: usize = 200;

pub(crate) fn chunk_of(c: StoredChunk, enabled: bool) -> KnowledgeChunk {
    KnowledgeChunk {
        chunk_id: c.chunk_id,
        path: c.path,
        seq: c.seq,
        heading: c.heading,
        text: c.text,
        page: c.page.and_then(|p| u32::try_from(p).ok()),
        char_start: u64::try_from(c.char_start).unwrap_or(0),
        char_end: u64::try_from(c.char_end).unwrap_or(0),
        line_start: u32::try_from(c.line_start).unwrap_or(0),
        line_end: u32::try_from(c.line_end).unwrap_or(0),
        enabled,
        score: c.score,
        found_by: c.found_by.map(|f| match f {
            node::FoundBy::Words => FoundBy::Words,
            node::FoundBy::Meaning => FoundBy::Meaning,
            node::FoundBy::Both => FoundBy::Both,
        }),
    }
}

fn mode_of(mode: node::SearchMode) -> SearchMode {
    match mode {
        node::SearchMode::Words => SearchMode::Words,
        node::SearchMode::Meaning => SearchMode::Meaning,
        node::SearchMode::Hybrid => SearchMode::Hybrid,
    }
}

fn extension_of(session: &oikome_node::OikomeSession) -> ExtensionDownload {
    extension_state(session, "lance")
}

/// How far the download of the DuckDB extension `name` is.
pub(crate) fn extension_state(
    session: &oikome_node::OikomeSession,
    name: &str,
) -> ExtensionDownload {
    let (state, dir) = session.extension_support(name);
    let (state, done, total, error) = match state {
        SupportState::Idle => (DownloadState::Idle, 0, None, None),
        SupportState::Downloading { done, total } => {
            (DownloadState::Downloading, done, total, None)
        }
        SupportState::Installing => (DownloadState::Installing, 0, None, None),
        SupportState::Failed(e) => (DownloadState::Failed, 0, None, Some(e)),
    };
    ExtensionDownload {
        state,
        done,
        total,
        error,
        path: dir.to_string_lossy().into_owned(),
    }
}

fn model_of(
    info: &model::ModelInfo,
    state: model::ModelState,
    used: &[(String, String)],
    path: &std::path::Path,
) -> EmbeddingModel {
    let (state, done, error) = match state {
        model::ModelState::Absent => (ModelState::Absent, 0, None),
        model::ModelState::Downloading { done, .. } => (ModelState::Downloading, done, None),
        model::ModelState::Ready => (ModelState::Ready, info.size(), None),
        model::ModelState::Failed(e) => (ModelState::Failed, 0, Some(e)),
    };
    EmbeddingModel {
        id: info.id.to_string(),
        name: info.name.to_string(),
        license: info.license.to_string(),
        size: info.size(),
        dims: u32::try_from(info.dims).unwrap_or(u32::MAX),
        state,
        done,
        error,
        used_by: used
            .iter()
            .filter(|(m, _)| m == info.id)
            .map(|(_, space)| space.clone())
            .collect(),
        path: path.to_string_lossy().into_owned(),
    }
}

fn stage_of(stage: Stage) -> KnowledgeStage {
    match stage {
        Stage::Queued => KnowledgeStage::Waiting,
        Stage::Reading => KnowledgeStage::Reading,
        Stage::Splitting => KnowledgeStage::Splitting,
        Stage::Embedding => KnowledgeStage::Embedding,
        Stage::Ready => KnowledgeStage::Ready,
        Stage::Failed => KnowledgeStage::Failed,
    }
}

impl Api {
    /// The status of `space`'s knowledge, with whether Lance support is here.
    async fn knowledge_view(&self, space: &str) -> Result<KnowledgeStatus, ApiError> {
        let session = self.session(None);
        let status = session.knowledge_status(space)?;
        let available = session.knowledge_available().await.is_ok();
        let meaning = status.model.as_deref().is_some_and(|id| {
            session
                .embedding_models()
                .iter()
                .any(|(m, state)| m.id == id && *state == model::ModelState::Ready)
        });
        let files: Vec<KnowledgeFile> = status
            .assets
            .into_iter()
            .map(|a| KnowledgeFile {
                path: a.path,
                name: a.name,
                size: a.size,
                stage: stage_of(a.stage),
                chunks: a.chunks,
                reason: if a.stage == Stage::Failed {
                    a.error
                } else {
                    None
                },
                updated_ms: a.updated_ms,
            })
            .collect();
        let count = |f: fn(&KnowledgeFile) -> bool| {
            u32::try_from(files.iter().filter(|x| f(x)).count()).unwrap_or(u32::MAX)
        };
        Ok(KnowledgeStatus {
            space: space.to_string(),
            prepared_ms: status.prepared_ms,
            available,
            extension: extension_of(&session),
            model: status.model,
            meaning,
            ready: count(|f| f.stage == KnowledgeStage::Ready),
            failed: count(|f| f.stage == KnowledgeStage::Failed),
            waiting: count(|f| {
                matches!(
                    f.stage,
                    KnowledgeStage::Waiting
                        | KnowledgeStage::Reading
                        | KnowledgeStage::Splitting
                        | KnowledgeStage::Embedding
                )
            }),
            chunks: files.iter().map(|f| f.chunks).sum(),
            files,
        })
    }

    /// Prepare the rest of `space`'s queue in the background.
    fn work_in_background(&self, space: &str) {
        let session = self.session(None);
        let space = space.to_string();
        tokio::spawn(async move {
            if let Err(e) = session.work_knowledge(&space, ROUND).await {
                tracing::warn!(space, error = %e, "preparing knowledge");
            }
        });
    }

    /// While model `id` downloads, wait for it; once it is here, give
    /// `space`'s passages vectors at once rather than at the node's next
    /// regular refresh.
    fn vectors_when_model_arrives(&self, space: String, id: String) {
        let session = self.session(None);
        tokio::spawn(async move {
            loop {
                let state = session
                    .embedding_models()
                    .into_iter()
                    .find(|(m, _)| m.id == id)
                    .map(|(_, state)| state);
                match state {
                    Some(model::ModelState::Ready) => break,
                    Some(model::ModelState::Downloading { .. }) => {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    _ => return,
                }
            }
            if let Err(e) = session.update_knowledge(&space, ROUND).await {
                tracing::warn!(space, error = %e, "giving passages vectors");
            }
        });
    }

    /// What an AI can read of `space`: prepared or not, each original.
    pub async fn knowledge_status(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<KnowledgeStatus, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        self.knowledge_view(&req.space).await
    }

    /// Prepare `space` for AI: its Markdown, text, code and PDF files are
    /// split into chunks with their places, in Lance, indexed for full
    /// text. Runs in the background; the status says how far it is.
    pub async fn prepare_knowledge(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<KnowledgeStatus, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        self.session(None).prepare_knowledge(&req.space).await?;
        self.work_in_background(&req.space);
        self.knowledge_view(&req.space).await
    }

    /// Try every original that failed again.
    pub async fn retry_knowledge(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<KnowledgeStatus, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        self.session(None).retry_knowledge(&req.space)?;
        self.work_in_background(&req.space);
        self.knowledge_view(&req.space).await
    }

    /// The first chunks Prepare would make, without writing anything.
    pub async fn knowledge_preview(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<Vec<KnowledgeChunk>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        Ok(self
            .session(None)
            .preview_knowledge(&req.space, PREVIEW)
            .await?
            .into_iter()
            .map(|c| chunk_of(c, true))
            .collect())
    }

    /// The chunks of one original, in order, each with whether it is on.
    pub async fn knowledge_chunks(
        &self,
        caller: &Caller,
        req: KnowledgeChunksRequest,
    ) -> Result<Vec<KnowledgeChunk>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        Ok(self
            .session(None)
            .knowledge_chunks(&req.space, &req.path)
            .await?
            .into_iter()
            .map(|(c, on)| chunk_of(c, on))
            .collect())
    }

    /// Turn a chunk off (kept, never returned to an assistant) or on.
    pub async fn set_knowledge_chunk(
        &self,
        caller: &Caller,
        req: SetKnowledgeChunk,
    ) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        if req.chunk_id.len() != 32 || !req.chunk_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("not a chunk id"));
        }
        self.session(None)
            .set_knowledge_chunk(&req.space, &req.chunk_id, req.enabled)?;
        Ok(Empty {})
    }

    /// Search `space`'s knowledge as an assistant would, with the scores
    /// and how long it took.
    pub async fn retrieval_test(
        &self,
        caller: &Caller,
        req: RetrievalTest,
    ) -> Result<RetrievalResults, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        let query = req.query.trim();
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(ApiError::invalid(format!(
                "a query has at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let mode = req.mode.map(|m| match m {
            SearchMode::Words => node::SearchMode::Words,
            SearchMode::Meaning => node::SearchMode::Meaning,
            SearchMode::Hybrid => node::SearchMode::Hybrid,
        });
        let found = self
            .session(None)
            .search_knowledge_by(&req.space, query, req.limit.unwrap_or(10), mode)
            .await?;
        Ok(RetrievalResults {
            hits: found.hits.into_iter().map(|c| chunk_of(c, true)).collect(),
            mode: mode_of(found.mode),
            took_ms: found.took_ms,
        })
    }

    /// Search `space` by meaning with a model as well as by words, or by
    /// words only. Its originals are prepared again in the background; the
    /// passages they have now serve until then.
    pub async fn set_knowledge_model(
        &self,
        caller: &Caller,
        req: SetKnowledgeModel,
    ) -> Result<KnowledgeStatus, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        let session = self.session(None);
        let status = session
            .set_knowledge_model(&req.space, req.model.as_deref())
            .await?;
        if status.prepared_ms.is_some() {
            self.work_in_background(&req.space);
        }
        if let Some(id) = req.model {
            self.vectors_when_model_arrives(req.space.clone(), id);
        }
        self.knowledge_view(&req.space).await
    }

    /// The embedding models, where each is, and the collections using it.
    pub async fn list_models(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<EmbeddingModel>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let session = self.session(None);
        let mut used: Vec<(String, String)> = Vec::new();
        for space in self.catalog.list_spaces() {
            if let Ok(status) = session.knowledge_status(&space.name) {
                if let Some(m) = status.model {
                    used.push((m, space.name.clone()));
                }
            }
        }
        Ok(session
            .embedding_models()
            .into_iter()
            .map(|(info, state)| {
                let path = model::dir(&self.engines.models_dir(), info);
                model_of(info, state, &used, &path)
            })
            .collect())
    }

    /// Start downloading a model, from its pinned revision, checked against
    /// its published digests. It runs on this computer.
    pub async fn download_model(
        &self,
        caller: &Caller,
        req: ModelRef,
    ) -> Result<EmbeddingModel, ApiError> {
        caller.require_owner()?;
        let state = self.session(None).download_embedding_model(&req.id)?;
        let info = model::model(&req.id).ok_or_else(|| ApiError::invalid("no such model"))?;
        let path = model::dir(&self.engines.models_dir(), info);
        Ok(model_of(info, state, &[], &path))
    }

    /// Remove a model's files. Collections searched by meaning with it are
    /// searched by words until it is downloaded again.
    pub async fn remove_model(&self, caller: &Caller, req: ModelRef) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        self.session(None).remove_embedding_model(&req.id).await?;
        Ok(Empty {})
    }

    /// The skills of a collection prepared for AI, each with its files and
    /// digests, and any problem that keeps it from assistants.
    pub async fn knowledge_skills(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<Vec<SkillInfo>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.space)?;
        Ok(self
            .session(None)
            .knowledge_skills(&req.space)
            .await?
            .into_iter()
            .map(|s| skill_info(Some(&req.space), s))
            .collect())
    }

    /// Start downloading the DuckDB extension knowledge needs, from
    /// DuckDB's repository, and installing it; DuckDB checks its signature
    /// when it loads it. `knowledge_status` says how far it is.
    pub async fn install_lance(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<ExtensionDownload, ApiError> {
        caller.require_owner()?;
        let session = self.session(None);
        session.install_knowledge_support()?;
        Ok(extension_of(&session))
    }
}
