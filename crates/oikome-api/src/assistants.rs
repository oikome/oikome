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

//! AI assistants the owner connects (M3d). Each one is a principal of its
//! own, like a plugin: a DID made for it when it is connected, holding a
//! whole-Space grant on each collection the owner chose. It reaches the
//! node with a token, shown to the owner once; only the token's BLAKE3 hash
//! is kept. As itself it may list and describe what it can read, search it,
//! read a row or document, use skills, and chart analysis tables. Its search
//! finds rows through the
//! owner's index and reads them again as the assistant, so its grants
//! decide what it gets. Every search and read goes to Activity; ending its
//! access drops its grants and its token at once.

use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use oikome_core::{validate_space_name, SpaceKind};
use oikome_identity::NodeIdentity;
use oikome_node::{AccessEntry, OikomeSession, Outcome, QueryResult, SessionConfig};
use oikome_semantic::rewrite::quote_ident;
use serde::{Deserialize, Serialize};

use crate::invites::read_private;
use crate::search::{hits_wanted, terms_of, MAX_KEY_COLUMNS};
use crate::{
    encode_rows, first_count, now_ms, AnalysisView, Api, ApiError, Assistant, AssistantApp,
    AssistantChart, AssistantCollections, AssistantConnected, AssistantRef, AssistantSaveChart,
    Caller, ChartResult, ChartableTable, ColumnDescription, ConnectAssistant, Empty, ErrorCode,
    GetSkill, InstallAssistant, KnowledgePassage, KnowledgeText, ListSkills, ReadKnowledge,
    ReadRow, ReadableCollection, ReadableTable, Rows, Search, SearchResults, SkillContent,
    SkillInfo, TableDescription, TableRef,
};
use oikome_node::knowledge::{skills, KnowledgeRange};

const ASSISTANTS_FILE: &str = "assistants.json";
/// Tokens start so, to be told from other secrets in a config file.
const TOKEN_PREFIX: &str = "oik_";
const MAX_ASSISTANTS: usize = 20;
const MAX_NAME_CHARS: usize = 64;
/// Every method an assistant calls starts so; Activity knows it by it.
pub(crate) const METHOD_PREFIX: &str = "assistant_";
const SEARCH_METHOD: &str = "assistant_search";
const READ_METHOD: &str = "assistant_read";
const CHART_METHOD: &str = "assistant_chart";
const SAVE_CHART_METHOD: &str = "assistant_save_chart";
/// How often its last use is written down.
const USED_EVERY_MS: u64 = 60_000;
const MAX_READ_BYTES: usize = 32 << 20;

/// A connected assistant, as kept in `.oikome/assistants.json` (0600).
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    /// The principal it reads as.
    did: String,
    name: String,
    app: AssistantApp,
    #[serde(default)]
    plugin: Option<String>,
    #[serde(default)]
    client: Option<String>,
    collections: Vec<String>,
    /// BLAKE3 of its token, in hex; empty once its access ended.
    token_hash: String,
    connected_at_ms: u64,
    #[serde(default)]
    last_used_ms: Option<u64>,
    #[serde(default)]
    ended_at_ms: Option<u64>,
}

impl Stored {
    fn assistant(&self, api: &Api) -> Assistant {
        Assistant {
            id: self.id.clone(),
            did: self.did.clone(),
            name: self.name.clone(),
            app: self.app,
            plugin: self.plugin.clone(),
            client: self.client.clone(),
            // A collection removed since is gone from what it reads.
            collections: self
                .collections
                .iter()
                .filter(|c| api.catalog.get_space(c).is_ok())
                .cloned()
                .collect(),
            connected_at_ms: self.connected_at_ms,
            last_used_ms: self.last_used_ms,
            ended_at_ms: self.ended_at_ms,
        }
    }

    fn active(&self) -> bool {
        self.ended_at_ms.is_none() && !self.token_hash.is_empty()
    }
}

/// `n` random bytes.
fn random<const N: usize>() -> Result<[u8; N], ApiError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(bytes)
}

/// A name for an assistant: one line, within [`MAX_NAME_CHARS`].
fn name_of(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(format!(
            "give the assistant a name of 1 to {MAX_NAME_CHARS} characters on one line"
        )));
    }
    Ok(name.to_string())
}

/// Whether `token` hashes to `stored` (hex); BLAKE3 hashes compare in
/// constant time.
fn token_matches(token_hash: &blake3::Hash, stored: &str) -> bool {
    blake3::Hash::from_hex(stored).is_ok_and(|h| h == *token_hash)
}

impl Api {
    fn assistants_path(&self) -> PathBuf {
        self.catalog.oikome_dir().join(ASSISTANTS_FILE)
    }

    fn stored_assistants(&self) -> Result<Vec<Stored>, ApiError> {
        read_private(&self.assistants_path())
    }

    fn stored_assistant(&self, id: &str) -> Result<Stored, ApiError> {
        self.stored_assistants()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| ApiError::not_found(format!("there is no assistant {id}")))
    }

    /// Assistant DIDs and names, for the access log.
    pub(crate) fn assistant_names(&self) -> Vec<(String, String)> {
        self.stored_assistants()
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.did, a.name))
            .collect()
    }

    /// The owner's assistants, connected and ended, in the order connected.
    pub async fn list_assistants(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<Assistant>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        Ok(self
            .stored_assistants()?
            .iter()
            .map(|a| a.assistant(self))
            .collect())
    }

    /// Connect an assistant that may read `collections`. The token it
    /// reaches this data center with is in the answer, this once.
    pub async fn connect_assistant(
        &self,
        caller: &Caller,
        req: ConnectAssistant,
    ) -> Result<AssistantConnected, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (name, app, plugin, client) = match (req.plugin.as_deref(), req.client.as_deref()) {
            (Some(plugin), Some(client)) => {
                let capability = self.mcp_client(plugin, client)?;
                (
                    name_of(&capability.name)?,
                    AssistantApp::Other,
                    Some(plugin.to_string()),
                    Some(client.to_string()),
                )
            }
            (None, None) => (name_of(&req.name)?, req.app, None, None),
            _ => {
                return Err(ApiError::invalid(
                    "a plugin-driven MCP client names both its plugin and client capability",
                ))
            }
        };
        let collections = self.readable_by_assistants(&req.collections)?;
        if self
            .stored_assistants()?
            .iter()
            .filter(|a| a.active())
            .count()
            >= MAX_ASSISTANTS
        {
            return Err(ApiError::invalid(format!(
                "up to {MAX_ASSISTANTS} assistants can be connected; end one's access first"
            )));
        }
        let token = format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(random::<32>()?));
        let id: String = random::<8>()?.iter().map(|b| format!("{b:02x}")).collect();
        let did = NodeIdentity::generate()
            .map_err(ApiError::internal)?
            .did()
            .to_string();
        let stored = Stored {
            id,
            did,
            name,
            app,
            plugin,
            client,
            collections: Vec::new(),
            token_hash: blake3::hash(token.as_bytes()).to_hex().to_string(),
            connected_at_ms: now_ms(),
            last_used_ms: None,
            ended_at_ms: None,
        };
        let stored = self.grant_assistant(stored, collections)?;
        Ok(AssistantConnected {
            assistant: stored.assistant(self),
            token,
        })
    }

    /// Configure the selected code CLI on this computer, without showing its
    /// token or making the owner edit its configuration file.
    pub async fn install_assistant(
        &self,
        caller: &Caller,
        req: InstallAssistant,
    ) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        let stored = self.stored_assistant(&req.id)?;
        if !stored.active()
            || !token_matches(&blake3::hash(req.token.as_bytes()), &stored.token_hash)
        {
            return Err(ApiError::permission_denied(
                "the assistant's one-time token does not match",
            ));
        }
        let capability = match (stored.plugin.as_deref(), stored.client.as_deref()) {
            (Some(plugin), Some(client)) => self.mcp_client(plugin, client)?,
            (None, None) => {
                let (plugin, client) = match stored.app {
                    AssistantApp::KimiCode => ("kimi-code", "kimi-code-cli"),
                    AssistantApp::Codex => ("codex", "codex-cli"),
                    _ => {
                        return Err(ApiError::invalid(
                            "this client has no declarative one-click setup",
                        ))
                    }
                };
                self.mcp_client(plugin, client)?
            }
            _ => {
                return Err(ApiError::internal(
                    "an assistant's client record is incomplete",
                ))
            }
        };
        let config = capability.config.ok_or_else(|| {
            ApiError::invalid("this client offers manual setup rather than one-click setup")
        })?;
        crate::assistant_config::install(&config, &req.url, &req.token, req.replace)?;
        Ok(Empty {})
    }

    /// Change what a connected assistant may read.
    pub async fn set_assistant_collections(
        &self,
        caller: &Caller,
        req: AssistantCollections,
    ) -> Result<Assistant, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let stored = self.stored_assistant(&req.id)?;
        if !stored.active() {
            return Err(ApiError::invalid(format!(
                "{}'s access has ended; connect it again",
                stored.name
            )));
        }
        let collections = self.readable_by_assistants(&req.collections)?;
        Ok(self.grant_assistant(stored, collections)?.assistant(self))
    }

    /// End an assistant's access: its grants and its token go at once; it
    /// stays listed, as ended, until it is removed.
    pub async fn end_assistant_access(
        &self,
        caller: &Caller,
        req: AssistantRef,
    ) -> Result<Assistant, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let mut stored = self.grant_assistant(self.stored_assistant(&req.id)?, Vec::new())?;
        stored.token_hash = String::new();
        stored.ended_at_ms.get_or_insert_with(now_ms);
        self.keep_assistant(stored.clone())?;
        Ok(stored.assistant(self))
    }

    /// Remove an assistant: its access ends and it is no longer listed;
    /// Activity keeps its name.
    pub async fn remove_assistant(
        &self,
        caller: &Caller,
        req: AssistantRef,
    ) -> Result<Vec<Assistant>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let stored = self.grant_assistant(self.stored_assistant(&req.id)?, Vec::new())?;
        self.change_private(&self.assistants_path(), |all: &mut Vec<Stored>| {
            all.retain(|a| a.id != req.id)
        })?;
        self.remember_name(&stored.did, &stored.name);
        self.list_assistants(caller, Empty {}).await
    }

    /// The caller an assistant's token stands for, or `None` for a token
    /// of no assistant or of one whose access ended. Notes when it was used.
    pub fn assistant_by_token(&self, token: &str) -> Option<Caller> {
        let hash = blake3::hash(token.as_bytes());
        let found = self
            .stored_assistants()
            .ok()?
            .into_iter()
            .filter(Stored::active)
            .find(|a| token_matches(&hash, &a.token_hash))?;
        let now = now_ms();
        if found
            .last_used_ms
            .is_none_or(|at| now.saturating_sub(at) >= USED_EVERY_MS)
        {
            let id = found.id.clone();
            let noted = self.change_private(&self.assistants_path(), |all: &mut Vec<Stored>| {
                for a in all.iter_mut().filter(|a| a.id == id) {
                    a.last_used_ms = Some(now);
                }
            });
            if let Err(e) = noted {
                tracing::warn!(error = %e.message, "could not note when an assistant was used");
            }
        }
        Some(Caller::Principal { did: found.did })
    }

    /// What the calling assistant may read: its collections, each table's
    /// columns as it sees them, rows and key.
    pub async fn assistant_collections(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<ReadableCollection>, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let session = self.assistant_session(&me.did);
        let owner = self.session(None);
        let mut out = Vec::new();
        for name in &me.collections {
            let Ok(tables) = session.readable_relations(name).await else {
                continue;
            };
            let mut readable = Vec::with_capacity(tables.len());
            for table in tables {
                // Both names come from the catalog, and are quoted.
                let from = format!("{}.{}", quote_ident(name), quote_ident(&table));
                let columns: Vec<String> =
                    match session.sql(&format!("SELECT * FROM {from} LIMIT 0")).await {
                        Ok(QueryResult::Batches { schema, .. }) => {
                            schema.fields().iter().map(|f| f.name().clone()).collect()
                        }
                        _ => continue,
                    };
                let rows = match session.sql(&format!("SELECT count(*) FROM {from}")).await {
                    Ok(QueryResult::Batches { batches, .. }) => first_count(&batches),
                    _ => None,
                };
                // A key it can't see all of is no key for it.
                let key = owner.row_key(name, &table).await.unwrap_or_default();
                let key = if key.iter().all(|k| columns.contains(k)) {
                    key
                } else {
                    Vec::new()
                };
                readable.push(ReadableTable {
                    name: table,
                    columns,
                    rows,
                    key,
                });
            }
            out.push(ReadableCollection {
                name: name.clone(),
                tables: readable,
            });
        }
        Ok(out)
    }

    /// The columns of one table the calling assistant may read, with their
    /// physical types and the types charts read them as.
    pub async fn assistant_describe_table(
        &self,
        caller: &Caller,
        req: TableRef,
    ) -> Result<TableDescription, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        validate_space_name(&req.space)?;
        if !me.collections.contains(&req.space) {
            return Err(no_collection(&req.space));
        }
        let described = self
            .assistant_session(&me.did)
            .describe_metadata(&oikome_semantic::DescribeRequest {
                space: Some(req.space),
                table: req.table,
                detail: false,
            })
            .await?;
        Ok(TableDescription {
            space: described.space,
            table: described.table,
            table_type: described.table_type,
            format: described.format,
            columns: described
                .columns
                .into_iter()
                .map(|column| ColumnDescription {
                    name: column.name,
                    data_type: column.data_type,
                    chart_type: column.chart_type,
                    read_type: column.read_type,
                    nullable: column.nullable,
                })
                .collect(),
        })
    }

    /// The calling assistant's search: in its collections only, or one of
    /// them, as itself. A collection it may not read is one it doesn't
    /// know of.
    pub async fn assistant_search(
        &self,
        caller: &Caller,
        req: Search,
    ) -> Result<SearchResults, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let terms = terms_of(&req.query)?;
        let names = match &req.collection {
            Some(name) => {
                validate_space_name(name)?;
                if !me.collections.contains(name) {
                    self.record(AccessEntry {
                        outcome: Outcome::Denied,
                        reason: Some("not one of its collections".to_string()),
                        detail: Some(terms.join(" ")),
                        ..entry(&me, SEARCH_METHOD, vec![name.clone()])
                    });
                    return Err(no_collection(name));
                }
                vec![name.clone()]
            }
            None => me.collections.clone(),
        };
        let spaces = names
            .iter()
            .filter_map(|n| self.catalog.get_space(n).ok())
            .collect();
        let session = self.assistant_session(&me.did);
        let mut results = self
            .search_in(&session, spaces, &terms, hits_wanted(req.limit))
            .await;
        // What went wrong is the owner's to read, in the log.
        for skipped in &mut results.skipped {
            skipped.message = None;
        }
        if !terms.is_empty() {
            self.record(AccessEntry {
                detail: Some(terms.join(" ")),
                ..entry(&me, SEARCH_METHOD, names)
            });
        }
        Ok(results)
    }

    /// One row of a collection the calling assistant may read, by its key,
    /// read as the assistant.
    pub async fn assistant_read_row(
        &self,
        caller: &Caller,
        req: ReadRow,
    ) -> Result<Rows, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        validate_space_name(&req.space)?;
        if req.key.is_empty() || req.key.len() > MAX_KEY_COLUMNS {
            return Err(ApiError::invalid(format!(
                "a key has 1 to {MAX_KEY_COLUMNS} columns"
            )));
        }
        let read = entry(&me, READ_METHOD, vec![req.space.clone()]);
        if !me.collections.contains(&req.space) {
            self.record(AccessEntry {
                outcome: Outcome::Denied,
                reason: Some("not one of its collections".to_string()),
                ..read
            });
            return Err(no_collection(&req.space));
        }
        let session = self.assistant_session(&me.did);
        let rows = match session.row_by_key(&req.space, &req.table, &req.key).await {
            Ok(QueryResult::Batches { schema, batches }) => {
                if batches.iter().all(|b| b.num_rows() == 0) {
                    return Err(ApiError::not_found(format!(
                        "No such row in {}.{}",
                        req.space, req.table
                    )));
                }
                // A photo of a folder viewed in place: its thumbnail from the
                // index goes with the row it was allowed to read.
                let (schema, batches) = self.with_thumbnail(&req, schema, batches)?;
                encode_rows(&schema, &batches, 1, MAX_READ_BYTES)
            }
            Ok(_) => Err(ApiError::internal("reading a row returned no rows")),
            Err(e) => Err(e.into()),
        };
        match &rows {
            Ok(_) => self.record(AccessEntry {
                merge: true,
                ..read
            }),
            Err(e) => self.record(AccessEntry {
                outcome: if e.code == ErrorCode::PermissionDenied {
                    Outcome::Denied
                } else {
                    Outcome::Failed
                },
                reason: Some(e.message.clone()),
                ..read
            }),
        }
        rows
    }

    /// Passages of the prepared documents of the calling assistant's
    /// collections (or one of them) matching `req.query` by words, and by
    /// meaning where a collection has a model, best first. An assistant is given whole collections, so it reads their
    /// knowledge as the owner keeps it. A search that finds passages is in
    /// Activity with the search it came with.
    pub async fn assistant_search_knowledge(
        &self,
        caller: &Caller,
        req: Search,
    ) -> Result<Vec<KnowledgePassage>, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let names = match &req.collection {
            Some(name) => {
                validate_space_name(name)?;
                if !me.collections.contains(name) {
                    return Err(no_collection(name));
                }
                vec![name.clone()]
            }
            None => me.collections.clone(),
        };
        let query = req.query.trim().to_string();
        let limit = req.limit.unwrap_or(10).clamp(1, 100);
        let owner = self.session(None);
        // Each collection's hits in its own order; scores of different
        // collections may come from different searches, so they are merged
        // by rank, not by score.
        let mut found: Vec<(usize, KnowledgePassage)> = Vec::new();
        for name in &names {
            let hits = match owner.search_knowledge(name, &query, limit).await {
                Ok(found) => found.hits,
                Err(e) => {
                    tracing::warn!(collection = %name, error = %e, "searching knowledge failed");
                    continue;
                }
            };
            found.extend(hits.into_iter().enumerate().map(|(rank, c)| {
                let chunk = crate::knowledge::chunk_of(c, true);
                let passage = KnowledgePassage {
                    collection: name.clone(),
                    excerpt: chunk.text.chars().take(600).collect(),
                    path: chunk.path,
                    chunk_id: chunk.chunk_id,
                    heading: chunk.heading,
                    page: chunk.page,
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    score: chunk.score,
                    found_by: chunk.found_by,
                };
                (rank, passage)
            }));
        }
        found.sort_by_key(|(rank, _)| *rank);
        let mut found: Vec<KnowledgePassage> = found.into_iter().map(|(_, p)| p).collect();
        found.truncate(limit as usize);
        if !found.is_empty() {
            let collections: Vec<String> = names
                .iter()
                .filter(|n| found.iter().any(|p| &p.collection == *n))
                .cloned()
                .collect();
            self.record(AccessEntry {
                detail: Some(query),
                merge: true,
                ..entry(&me, SEARCH_METHOD, collections)
            });
        }
        Ok(found)
    }

    /// The skills the calling assistant may use: Oikome's own, then those of
    /// its collections prepared for AI (or one of them). A skill with a
    /// problem is left out.
    pub async fn assistant_list_skills(
        &self,
        caller: &Caller,
        req: ListSkills,
    ) -> Result<Vec<SkillInfo>, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let names = match &req.collection {
            Some(name) => {
                validate_space_name(name)?;
                if !me.collections.contains(name) {
                    return Err(no_collection(name));
                }
                vec![name.clone()]
            }
            None => me.collections.clone(),
        };
        let owner = self.session(None);
        let mut found = Vec::new();
        if req.collection.is_none() {
            found.push(crate::knowledge::skill_info(None, skills::oikome_skill()));
        }
        let mut listed = Vec::new();
        for name in &names {
            match owner.knowledge_skills(name).await {
                Ok(list) => {
                    let valid: Vec<_> = list.into_iter().filter(|s| s.problem.is_none()).collect();
                    if !valid.is_empty() {
                        listed.push(name.clone());
                    }
                    found.extend(
                        valid
                            .into_iter()
                            .map(|s| crate::knowledge::skill_info(Some(name), s)),
                    );
                }
                Err(e) => {
                    tracing::warn!(collection = %name, error = %e, "listing skills failed");
                }
            }
        }
        if !listed.is_empty() {
            self.record(AccessEntry {
                detail: Some("skills".to_string()),
                merge: true,
                ..entry(&me, READ_METHOD, listed)
            });
        }
        Ok(found)
    }

    /// A file of a skill for the calling assistant: its `SKILL.md`, or
    /// another file it lists, as text with its SHA-256. Oikome never runs it.
    pub async fn assistant_get_skill(
        &self,
        caller: &Caller,
        req: GetSkill,
    ) -> Result<SkillContent, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let Some(collection) = req.collection.clone() else {
            if req.name != skills::OIKOME_SKILL_NAME
                || req.file.as_deref().is_some_and(|f| f != skills::SKILL_FILE)
            {
                return Err(ApiError::not_found(format!(
                    "there is no skill {}",
                    req.name
                )));
            }
            let t = skills::oikome_skill_text();
            return Ok(SkillContent {
                collection: None,
                name: req.name,
                path: t.path,
                sha256: t.sha256,
                text: t.text,
            });
        };
        validate_space_name(&collection)?;
        let read = AccessEntry {
            detail: Some(format!(
                "{}/{}",
                req.name,
                req.file.as_deref().unwrap_or(skills::SKILL_FILE)
            )),
            ..entry(&me, READ_METHOD, vec![collection.clone()])
        };
        if !me.collections.contains(&collection) {
            self.record(AccessEntry {
                outcome: Outcome::Denied,
                reason: Some("not one of its collections".to_string()),
                ..read
            });
            return Err(no_collection(&collection));
        }
        match self
            .session(None)
            .read_skill(&collection, &req.name, req.file.as_deref())
            .await
        {
            Ok(t) => {
                self.record(AccessEntry {
                    merge: true,
                    ..read
                });
                Ok(SkillContent {
                    collection: Some(collection),
                    name: req.name,
                    path: t.path,
                    sha256: t.sha256,
                    text: t.text,
                })
            }
            Err(e) => {
                let e = ApiError::from(e);
                self.record(AccessEntry {
                    outcome: Outcome::Failed,
                    reason: Some(e.message.clone()),
                    ..read
                });
                Err(e)
            }
        }
    }

    /// A passage of a prepared document and what is around it, or a range
    /// of its lines or characters, for the calling assistant.
    pub async fn assistant_read_knowledge(
        &self,
        caller: &Caller,
        req: ReadKnowledge,
    ) -> Result<KnowledgeText, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        validate_space_name(&req.collection)?;
        let read = AccessEntry {
            detail: Some(req.path.clone()),
            ..entry(&me, READ_METHOD, vec![req.collection.clone()])
        };
        if !me.collections.contains(&req.collection) {
            self.record(AccessEntry {
                outcome: Outcome::Denied,
                reason: Some("not one of its collections".to_string()),
                ..read
            });
            return Err(no_collection(&req.collection));
        }
        let range = match (&req.chunk_id, req.lines, req.chars) {
            (Some(chunk_id), None, None) => KnowledgeRange::Chunk {
                chunk_id: chunk_id.clone(),
                around: req.around.unwrap_or(1),
            },
            (None, Some((start, end)), None) => KnowledgeRange::Lines {
                page: req.page,
                start,
                end,
            },
            (None, None, Some((start, end))) => KnowledgeRange::Chars {
                page: req.page,
                start,
                end,
            },
            (None, None, None) => KnowledgeRange::Lines {
                page: req.page,
                start: 1,
                end: u32::MAX,
            },
            _ => {
                return Err(ApiError::invalid(
                    "name a chunk, or lines, or characters, not more than one",
                ))
            }
        };
        let text = self
            .session(None)
            .read_knowledge(&req.collection, &req.path, range)
            .await;
        match text {
            Ok(t) => {
                self.record(AccessEntry {
                    merge: true,
                    ..read
                });
                Ok(KnowledgeText {
                    collection: req.collection,
                    path: t.path,
                    page: t.page,
                    char_start: t.char_start,
                    char_end: t.char_end,
                    total_chars: t.total_chars,
                    text: t.text,
                })
            }
            Err(e) => {
                let e = ApiError::from(e);
                self.record(AccessEntry {
                    outcome: Outcome::Failed,
                    reason: Some(e.message.clone()),
                    ..read
                });
                Err(e)
            }
        }
    }

    /// The analysis tables the calling assistant may chart: in its
    /// collections, or one of them; their columns, and the charts kept.
    pub async fn assistant_charts(
        &self,
        caller: &Caller,
        req: ListSkills,
    ) -> Result<Vec<ChartableTable>, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        let names = match &req.collection {
            Some(name) => {
                validate_space_name(name)?;
                if !me.collections.contains(name) {
                    return Err(no_collection(name));
                }
                vec![name.clone()]
            }
            None => me.collections.clone(),
        };
        let mut out = Vec::new();
        for name in names {
            // A collection that views no folder has no analysis tables.
            if self.in_place_of(&name).is_ok() {
                out.extend(self.chartable(&name).await?);
            }
        }
        Ok(out)
    }

    /// A chart of an analysis table of one of the calling assistant's
    /// collections, computed as the assistant: its grants decide.
    pub async fn assistant_chart(
        &self,
        caller: &Caller,
        req: AssistantChart,
    ) -> Result<ChartResult, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        validate_space_name(&req.collection)?;
        let done = entry(&me, CHART_METHOD, vec![req.collection.clone()]);
        if !me.collections.contains(&req.collection) {
            self.record(AccessEntry {
                outcome: Outcome::Denied,
                reason: Some("not one of its collections".to_string()),
                ..done
            });
            return Err(no_collection(&req.collection));
        }
        let session = self.assistant_session(&me.did);
        let result = self
            .compute_chart(&session, &req.collection, &req.table, &req.spec)
            .await;
        self.record(match &result {
            Ok(_) => AccessEntry {
                merge: true,
                detail: Some(req.table.clone()),
                ..done
            },
            Err(e) => AccessEntry {
                outcome: if e.code == ErrorCode::PermissionDenied {
                    Outcome::Denied
                } else {
                    Outcome::Failed
                },
                reason: Some(e.message.clone()),
                ..done
            },
        });
        result
    }

    /// Keep a chart with an analysis table of one of the calling assistant's
    /// collections, marked as the assistant's. It must be one the assistant
    /// can compute; under a name it used before, its own chart is replaced,
    /// and another's name is not taken.
    pub async fn assistant_save_chart(
        &self,
        caller: &Caller,
        req: AssistantSaveChart,
    ) -> Result<AnalysisView, ApiError> {
        self.refresh()?;
        let me = self.assistant_of(caller)?;
        validate_space_name(&req.collection)?;
        let saved = entry(&me, SAVE_CHART_METHOD, vec![req.collection.clone()]);
        if !me.collections.contains(&req.collection) {
            self.record(AccessEntry {
                outcome: Outcome::Denied,
                reason: Some("not one of its collections".to_string()),
                ..saved
            });
            return Err(no_collection(&req.collection));
        }
        let session = self.assistant_session(&me.did);
        self.compute_chart(&session, &req.collection, &req.table, &req.spec)
            .await?;
        let mine = self
            .analysis_kept(&req.collection)?
            .into_iter()
            .find(|(table, _, _)| *table == req.table)
            .and_then(|(_, _, views)| {
                views.into_iter().find(|v| {
                    v.name == req.name.trim() && v.by.as_deref() == Some(me.name.as_str())
                })
            })
            .map(|v| v.id);
        let config = serde_json::to_string(&req.spec).map_err(ApiError::internal)?;
        let view = self.keep_view(
            &req.collection,
            &req.table,
            mine.as_deref(),
            &req.name,
            &config,
            Some(&me.name),
        )?;
        self.record(AccessEntry {
            detail: Some(view.name.clone()),
            ..saved
        });
        Ok(view)
    }

    /// The connected assistant `caller` is; anyone else is refused.
    fn assistant_of(&self, caller: &Caller) -> Result<Stored, ApiError> {
        let refused = || ApiError::permission_denied("only a connected assistant may ask this");
        let Caller::Principal { did } = caller else {
            return Err(refused());
        };
        self.stored_assistants()?
            .into_iter()
            .find(|a| a.active() && a.did == *did)
            .ok_or_else(refused)
    }

    /// A read-only session acting as the assistant `did`.
    fn assistant_session(&self, did: &str) -> OikomeSession {
        OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                default_space: None,
                principal: Some(did.to_string()),
            },
        )
    }

    fn record(&self, entry: AccessEntry) {
        if let Some(log) = &self.access_log {
            log.record(entry);
        }
    }

    /// The collections given, checked: each one of this data center's own
    /// (an assistant can't be given one shared with you), each once.
    fn readable_by_assistants(&self, names: &[String]) -> Result<Vec<String>, ApiError> {
        let mut out: Vec<String> = Vec::new();
        for name in names {
            let space = self.catalog.get_space(name)?;
            if space.kind == SpaceKind::Remote {
                return Err(ApiError::invalid(format!(
                    "{name} is shared with you; an assistant reads only your own collections"
                )));
            }
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        Ok(out)
    }

    /// Give `assistant` grants on exactly `collections`, and keep it.
    fn grant_assistant(
        &self,
        mut assistant: Stored,
        collections: Vec<String>,
    ) -> Result<Stored, ApiError> {
        let session = self.session(None);
        let did = std::slice::from_ref(&assistant.did);
        for space in &assistant.collections {
            if !collections.contains(space) && self.catalog.get_space(space).is_ok() {
                session.revoke_space(space, did)?;
            }
        }
        for space in &collections {
            if !assistant.collections.contains(space)
                || !self.catalog.has_space_grant(&assistant.did, space)
            {
                session.grant_space(space, did)?;
            }
        }
        assistant.collections = collections;
        self.keep_assistant(assistant.clone())?;
        Ok(assistant)
    }

    fn keep_assistant(&self, assistant: Stored) -> Result<(), ApiError> {
        self.change_private(&self.assistants_path(), |all: &mut Vec<Stored>| {
            match all.iter_mut().find(|a| a.id == assistant.id) {
                Some(kept) => *kept = assistant,
                None => all.push(assistant),
            }
        })
    }
}

/// A line of Activity for what `assistant` asked, allowed unless changed.
fn entry(assistant: &Stored, method: &str, spaces: Vec<String>) -> AccessEntry {
    AccessEntry {
        at_ms: now_ms(),
        caller: Some(assistant.did.clone()),
        method: method.to_string(),
        spaces,
        token_id: None,
        outcome: Outcome::Allowed,
        reason: None,
        detail: None,
        merge: false,
    }
}

/// What an assistant hears of a collection it may not read: that there is
/// none by that name.
fn no_collection(name: &str) -> ApiError {
    ApiError::not_found(format!("there is no collection {name}"))
}

impl Api {
    /// `batches` (one row of a folder's `files`, read as the assistant) with
    /// a `photo` column holding the photo's thumbnail, when it is a photo of
    /// a folder viewed in place; as they are otherwise.
    fn with_thumbnail(
        &self,
        req: &ReadRow,
        schema: arrow_schema::SchemaRef,
        batches: Vec<arrow_array::RecordBatch>,
    ) -> Result<(arrow_schema::SchemaRef, Vec<arrow_array::RecordBatch>), ApiError> {
        use std::sync::Arc;

        use arrow_array::{Array, BinaryArray, RecordBatch, StringArray};
        use arrow_schema::{DataType, Field, Schema};
        if req.table != oikome_node::inplace::FILES {
            return Ok((schema, batches));
        }
        let Some(location) = self.session(None).in_place_location(&req.space)? else {
            return Ok((schema, batches));
        };
        let Some(batch) = batches.into_iter().find(|b| b.num_rows() > 0) else {
            return Ok((schema, Vec::new()));
        };
        let text = |name: &str| {
            batch
                .column_by_name(name)
                .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
                .filter(|c| !c.is_null(0))
                .map(|c| c.value(0).to_string())
        };
        let thumb = match (text("kind").as_deref(), text("path")) {
            (Some(oikome_node::inplace::IMAGE), Some(path)) => {
                oikome_node::inplace::thumb(&self.catalog, &location, &path)?
            }
            _ => None,
        };
        let Some(thumb) = thumb else {
            return Ok((schema, vec![batch.slice(0, 1)]));
        };
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        fields.push(Field::new("photo", DataType::Binary, true));
        let schema = Arc::new(Schema::new(fields));
        let mut columns: Vec<arrow_array::ArrayRef> = batch.slice(0, 1).columns().to_vec();
        columns.push(Arc::new(BinaryArray::from(vec![Some(thumb.as_slice())])));
        let batch = RecordBatch::try_new(schema.clone(), columns).map_err(ApiError::internal)?;
        Ok((schema, vec![batch]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_one_short_line() {
        assert_eq!(name_of("  Claude Code ").unwrap(), "Claude Code");
        assert!(name_of("").is_err());
        assert!(name_of("two\nlines").is_err());
        assert!(name_of(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn a_token_matches_only_its_own_hash() {
        let hash = blake3::hash(b"oik_one");
        assert!(token_matches(&hash, &hash.to_hex()));
        assert!(!token_matches(&blake3::hash(b"oik_two"), &hash.to_hex()));
        assert!(!token_matches(&hash, ""));
        assert!(!token_matches(&hash, "not hex"));
    }
}
