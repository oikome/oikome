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
use oikome_node::knowledge::{Stage, StoredChunk};

use crate::{
    Api, ApiError, Caller, Empty, KnowledgeChunk, KnowledgeChunksRequest, KnowledgeFile,
    KnowledgeStage, KnowledgeStatus, LanceInstalled, RetrievalResults, RetrievalTest,
    SetKnowledgeChunk, SpaceRef,
};

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
    }
}

fn stage_of(stage: Stage) -> KnowledgeStage {
    match stage {
        Stage::Queued => KnowledgeStage::Waiting,
        Stage::Reading => KnowledgeStage::Reading,
        Stage::Splitting => KnowledgeStage::Splitting,
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
            ready: count(|f| f.stage == KnowledgeStage::Ready),
            failed: count(|f| f.stage == KnowledgeStage::Failed),
            waiting: count(|f| {
                matches!(
                    f.stage,
                    KnowledgeStage::Waiting | KnowledgeStage::Reading | KnowledgeStage::Splitting
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
        let found = self
            .session(None)
            .search_knowledge(&req.space, query, req.limit.unwrap_or(10))
            .await?;
        Ok(RetrievalResults {
            hits: found.hits.into_iter().map(|c| chunk_of(c, true)).collect(),
            took_ms: found.took_ms,
        })
    }

    /// Download DuckDB's Lance support (signed by DuckDB) to this computer.
    pub async fn install_lance(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<LanceInstalled, ApiError> {
        caller.require_owner()?;
        let version = self.engines.install_lance().await?;
        Ok(LanceInstalled { version })
    }
}
