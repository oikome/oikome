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
//! is kept. It may do three things, as itself: list what it can read,
//! search it, and read a row of it. Its search finds rows through the
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
    encode_rows, first_count, now_ms, Api, ApiError, Assistant, AssistantApp, AssistantCollections,
    AssistantConnected, AssistantRef, Caller, ConnectAssistant, Empty, ErrorCode, ReadRow,
    ReadableCollection, ReadableTable, Rows, Search, SearchResults,
};

const ASSISTANTS_FILE: &str = "assistants.json";
/// Tokens start so, to be told from other secrets in a config file.
const TOKEN_PREFIX: &str = "oik_";
const MAX_ASSISTANTS: usize = 20;
const MAX_NAME_CHARS: usize = 64;
/// Every method an assistant calls starts so; Activity knows it by it.
pub(crate) const METHOD_PREFIX: &str = "assistant_";
const SEARCH_METHOD: &str = "assistant_search";
const READ_METHOD: &str = "assistant_read";
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
        let name = name_of(&req.name)?;
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
            app: req.app,
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
