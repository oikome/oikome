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

//! Oikome's API for everything above the node (architecture v3 §13.4):
//! Oikome's own apps (web now; desktop and phone later), plugins and MCP.
//! The peer protocol between nodes is not here (`oikome-protocol`,
//! `oikome-network`).
//!
//! The API has no transport. `oikome serve --web` exposes it over HTTP on
//! loopback; the desktop app will call it in process. Every call names its
//! [`Caller`]: the owner, or a principal (a DID holding a UCAN). Operations
//! that change the node refuse a principal; what a principal reads goes
//! through the semantic layer like a peer's request.
//!
//! The API never builds SQL from request fields: names are checked against
//! the catalog and the operations call the node directly. Only `run_sql`
//! runs SQL, the owner's own; `add_examples` runs this crate's constants.

pub mod error;
mod examples;
mod places;
mod sources;
pub mod types;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use oikome_catalog::{LocationFormat, OikomeCatalog, StorageKind};
use oikome_connect::SourceConfig;
use oikome_core::{validate_space_name, AccessMode, CreateSpaceRequest};
use oikome_identity::keys::NODE_KEY_FILE;
use oikome_identity::ucan::{ABILITY_QUERY, ABILITY_READ};
use oikome_identity::{
    decode_recovery_key, encode_recovery_key, unix_now, Did, NodeIdentity, ENV_NODE_KEY,
};
use oikome_node::{
    AccessLog, AccessQuery, EngineRegistry, OikomeSession, Outcome, QueryResult, SessionConfig,
    StorageCredentials,
};
use oikome_semantic::rewrite::quote_ident;
use oikome_semantic::{split_statements, IssueUcanRequest};
use serde::{Deserialize, Serialize};

pub use error::{ApiError, ErrorCode};
pub use types::*;

/// Longest `run_sql` text, in bytes.
pub const MAX_SQL_BYTES: usize = 256 * 1024;
/// Most rows a `run_sql` result carries; the rest is cut off.
pub const MAX_RESULT_ROWS: usize = 10_000;
/// Roughly the most bytes a `run_sql` result carries.
pub const MAX_RESULT_BYTES: usize = 32 * 1024 * 1024;
/// Most rows `read_rows` returns at once.
pub const MAX_PAGE_ROWS: u32 = 1_000;
const DEFAULT_PAGE_ROWS: u32 = 100;
const DEFAULT_ACCESS_ROWS: u32 = 100;
const DEFAULT_SHARE_DAYS: u32 = 365;
const MAX_SHARE_DAYS: u32 = 366;
/// The data center's name and whether its recovery key was stored, under
/// `.oikome/`.
const NODE_SETTINGS_FILE: &str = "node.json";
/// Longest data center name, in characters.
const MAX_NODE_NAME_CHARS: usize = 64;

/// What the owner set about the data center itself.
#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeSettings {
    #[serde(default)]
    name: String,
    /// When the owner said they stored the recovery key.
    #[serde(default)]
    recovery_key_saved_at_ms: Option<u64>,
    /// When the examples were added; they are added once, so removing them
    /// is for good.
    #[serde(default)]
    examples_added_at_ms: Option<u64>,
    /// The examples still there.
    #[serde(default)]
    examples: Vec<String>,
    /// Where new collections go; this computer when `None`.
    #[serde(default)]
    default_storage: Option<String>,
    /// What the owner called each storage, by its name.
    #[serde(default)]
    storage_labels: BTreeMap<String, String>,
    /// The cards on Home, in order, once the owner arranged them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    home_cards: Option<Vec<String>>,
}

/// At most this many cards on Home, each id this long.
const MAX_HOME_CARDS: usize = 32;
const MAX_CARD_ID: usize = 80;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Who is calling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The node's owner: the web app logged in on this machine, the desktop app.
    Owner,
    /// Someone the owner granted access to: an AI tool, a plugin.
    Principal { did: String },
}

impl Caller {
    fn require_owner(&self) -> Result<(), ApiError> {
        match self {
            Caller::Owner => Ok(()),
            Caller::Principal { .. } => Err(ApiError::permission_denied(
                "only the node's owner can do this",
            )),
        }
    }
}

/// Rows, as an Arrow IPC stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub ipc: Vec<u8>,
    /// Whether rows were cut off at [`MAX_RESULT_ROWS`] or [`MAX_RESULT_BYTES`].
    pub truncated: bool,
}

/// What `run_sql` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlResult {
    /// The last statement returned rows.
    Rows(Rows),
    /// It returned none.
    Done(SqlDone),
}

/// The API over one node.
#[derive(Clone)]
pub struct Api {
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    access_log: Option<AccessLog>,
    flight_url: Option<String>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("root", &self.catalog.root_path())
            .finish_non_exhaustive()
    }
}

fn encode_rows(
    schema: &SchemaRef,
    batches: &[RecordBatch],
    max_rows: usize,
    max_bytes: usize,
) -> Result<Rows, ApiError> {
    let mut kept = Vec::new();
    let (mut rows, mut bytes, mut truncated) = (0usize, 0usize, false);
    for batch in batches.iter().filter(|b| b.num_rows() > 0) {
        if rows >= max_rows || bytes >= max_bytes {
            truncated = true;
            break;
        }
        let room = max_rows - rows;
        let batch = if batch.num_rows() > room {
            truncated = true;
            batch.slice(0, room)
        } else {
            batch.clone()
        };
        rows += batch.num_rows();
        bytes += batch.get_array_memory_size();
        kept.push(batch);
    }
    let mut ipc = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut ipc, schema).map_err(ApiError::internal)?;
        for batch in &kept {
            writer.write(batch).map_err(ApiError::internal)?;
        }
        writer.finish().map_err(ApiError::internal)?;
    }
    Ok(Rows { ipc, truncated })
}

/// The single number a `count(*)` returned.
fn first_count(batches: &[RecordBatch]) -> Option<u64> {
    let batch = batches.iter().find(|b| b.num_rows() > 0)?;
    let column = batch.column(0);
    if let Some(a) = column.as_any().downcast_ref::<arrow_array::Int64Array>() {
        return u64::try_from(a.value(0)).ok();
    }
    column
        .as_any()
        .downcast_ref::<arrow_array::UInt64Array>()
        .map(|a| a.value(0))
}

fn parse_did(text: &str) -> Result<String, ApiError> {
    Did::parse(text)
        .map(|d| d.to_string())
        .map_err(|_| ApiError::invalid(format!("'{text}' is not a DID")))
}

fn format_name(format: &LocationFormat) -> &'static str {
    match format {
        LocationFormat::IcebergParquet { .. } => "iceberg",
        LocationFormat::SqliteFile { .. } => "sqlite",
        other => other.name(),
    }
}

fn contact_of(c: oikome_catalog::Contact) -> Contact {
    Contact {
        did: c.did,
        name: c.name,
        added_at_ms: c.added_at_ms,
    }
}

impl Api {
    /// The API over the node made of `catalog` and `engines`.
    pub fn new(catalog: Arc<OikomeCatalog>, engines: Arc<EngineRegistry>) -> Self {
        Self {
            catalog,
            engines,
            access_log: None,
            flight_url: None,
        }
    }

    /// Read access history from `log`; without one, `list_access` is empty.
    pub fn with_access_log(mut self, log: AccessLog) -> Self {
        self.access_log = Some(log);
        self
    }

    /// Where peers reach this node's Flight service, for `node_info`.
    pub fn with_flight_url(mut self, url: impl Into<String>) -> Self {
        self.flight_url = Some(url.into());
        self
    }

    fn session(&self, default_space: Option<String>) -> OikomeSession {
        OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                default_space,
                principal: None,
            },
        )
    }

    /// Pick up changes other processes of the node made (`oikome sql`).
    fn refresh(&self) -> Result<(), ApiError> {
        self.catalog.refresh()?;
        Ok(())
    }

    fn node_did(&self) -> Result<String, ApiError> {
        Ok(self.engines.identity()?.did().to_string())
    }

    fn settings_path(&self) -> std::path::PathBuf {
        self.catalog.oikome_dir().join(NODE_SETTINGS_FILE)
    }

    fn settings(&self) -> Result<NodeSettings, ApiError> {
        match std::fs::read(self.settings_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(ApiError::internal),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NodeSettings::default()),
            Err(e) => Err(ApiError::internal(e)),
        }
    }

    /// Replace the settings file at once (temporary file, then rename).
    fn save_settings(&self, settings: &NodeSettings) -> Result<(), ApiError> {
        let path = self.settings_path();
        let bytes = serde_json::to_vec_pretty(settings).map_err(ApiError::internal)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::create_dir_all(self.catalog.oikome_dir()).map_err(ApiError::internal)?;
        std::fs::write(&tmp, bytes).map_err(ApiError::internal)?;
        std::fs::rename(&tmp, &path).map_err(ApiError::internal)
    }

    // ----- Node -----

    /// This node: its DID, name, version, data directory and Flight address.
    pub async fn node_info(&self, caller: &Caller, _: Empty) -> Result<NodeInfo, ApiError> {
        let owner = caller.require_owner().is_ok();
        let settings = if owner {
            self.settings()?
        } else {
            NodeSettings::default()
        };
        Ok(NodeInfo {
            did: self.node_did()?,
            name: settings.name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            // Only the owner learns where the data lives.
            data_dir: if owner {
                self.catalog.root_path().to_string()
            } else {
                String::new()
            },
            recovery_key_saved: settings.recovery_key_saved_at_ms.is_some(),
            recovery_key_saved_at_ms: settings.recovery_key_saved_at_ms,
            host: if owner {
                places::host_name()
            } else {
                String::new()
            },
            owner_name: if owner {
                places::owner_name()
            } else {
                String::new()
            },
            home_cards: if owner {
                settings.home_cards.clone()
            } else {
                None
            },
            flight_url: self.flight_url.clone(),
        })
    }

    /// Give the data center a name.
    pub async fn rename_node(&self, caller: &Caller, req: NodeName) -> Result<NodeInfo, ApiError> {
        caller.require_owner()?;
        let name = req.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NODE_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(ApiError::invalid(format!(
                "a name has 1 to {MAX_NODE_NAME_CHARS} characters"
            )));
        }
        let mut settings = self.settings()?;
        settings.name = name.to_string();
        self.save_settings(&settings)?;
        self.node_info(caller, Empty {}).await
    }

    /// Keep the cards the owner arranged on Home. The ids are the apps'
    /// own; the node only checks their size and characters.
    pub async fn set_home_cards(
        &self,
        caller: &Caller,
        req: HomeCards,
    ) -> Result<NodeInfo, ApiError> {
        caller.require_owner()?;
        let ok = req.cards.len() <= MAX_HOME_CARDS
            && req.cards.iter().all(|id| {
                !id.is_empty()
                    && id.len() <= MAX_CARD_ID
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
            });
        if !ok {
            return Err(ApiError::invalid(format!(
                "up to {MAX_HOME_CARDS} cards, each named with letters, digits, ':', '-' or '_'"
            )));
        }
        let mut settings = self.settings()?;
        settings.home_cards = Some(req.cards);
        self.save_settings(&settings)?;
        self.node_info(caller, Empty {}).await
    }

    /// The node's recovery key, for the owner to store. It is never logged.
    pub async fn recovery_key(&self, caller: &Caller, _: Empty) -> Result<RecoveryKey, ApiError> {
        caller.require_owner()?;
        let identity = self.engines.identity()?;
        let key = identity.with_seed(encode_recovery_key);
        Ok(RecoveryKey {
            key: key.to_string(),
        })
    }

    /// Record that the owner stored the recovery key.
    pub async fn confirm_recovery_key(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<NodeInfo, ApiError> {
        caller.require_owner()?;
        let mut settings = self.settings()?;
        settings.recovery_key_saved_at_ms = Some(now_ms());
        self.save_settings(&settings)?;
        self.node_info(caller, Empty {}).await
    }

    // ----- Spaces -----

    /// Every Space, ordered by name, with the number of contacts it is shared with.
    pub async fn list_spaces(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<SpaceSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let examples = self.settings()?.examples;
        let mut shared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for share in self.shares()? {
            if share.status == ShareStatus::Active {
                shared.entry(share.space).or_default().insert(share.did);
            }
        }
        Ok(self
            .catalog
            .list_spaces()
            .into_iter()
            .map(|space| {
                let location = self.catalog.get_location(&space.location).ok();
                let format = location
                    .as_ref()
                    .map(|l| format_name(&l.format))
                    .unwrap_or("unknown");
                let storage = location.and_then(|l| l.storage).map(|b| b.storage);
                SpaceSummary {
                    kind: match space.kind {
                        oikome_core::SpaceKind::Analytical => SpaceKind::Analytical,
                        oikome_core::SpaceKind::Transactional => SpaceKind::Transactional,
                        oikome_core::SpaceKind::Remote => SpaceKind::Remote,
                    },
                    format: format.to_string(),
                    readonly: space.access == AccessMode::ReadOnly
                        || space.kind == oikome_core::SpaceKind::Remote,
                    shared_with: shared.get(&space.name).map_or(0, |d| d.len() as u32),
                    storage,
                    example: examples.contains(&space.name),
                    name: space.name,
                }
            })
            .collect())
    }

    /// Create a local Space.
    pub async fn create_space(
        &self,
        caller: &Caller,
        req: NewSpace,
    ) -> Result<SpaceSummary, ApiError> {
        caller.require_owner()?;
        validate_space_name(&req.name)?;
        let kind = match req.kind.unwrap_or(SpaceKind::Analytical) {
            SpaceKind::Analytical => oikome_core::SpaceKind::Analytical,
            SpaceKind::Transactional => oikome_core::SpaceKind::Transactional,
            SpaceKind::Remote => {
                return Err(ApiError::invalid(
                    "a remote space is mounted from a share, not created",
                ))
            }
        };
        self.refresh()?;
        if let Some(storage) = &req.storage {
            self.catalog.get_storage(storage)?;
        }
        // A removed Space leaves its files, and the name with them.
        if self.catalog.get_space(&req.name).is_err()
            && self.catalog.get_location(&req.name).is_ok()
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!(
                    "the files of a removed space named '{}' are still in the data directory",
                    req.name
                ),
            ));
        }
        self.session(None)
            .create_space(CreateSpaceRequest {
                name: req.name.clone(),
                kind: Some(kind),
                storage: req.storage.clone(),
                ..Default::default()
            })
            .await?;
        self.list_spaces(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name == req.name)
            .ok_or_else(|| ApiError::internal("created space is missing"))
    }

    /// Drop a Space. Its active shares are revoked first: a token names the
    /// Space by name, and would work again on a new Space of that name.
    pub async fn drop_space(&self, caller: &Caller, req: SpaceRef) -> Result<Empty, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.catalog.get_space(&req.space)?;
        let session = self.session(None);
        for share in self.shares()? {
            if share.space == req.space && share.status == ShareStatus::Active {
                session.revoke_token(&share.id)?;
            }
        }
        // Its source stops with it; the files it read stay.
        if self.catalog.get_source(&req.space).is_ok() {
            session.remove_source(&req.space)?;
        }
        session.drop_space(&req.space).await?;
        let mut settings = self.settings()?;
        if settings.examples.contains(&req.space) {
            settings.examples.retain(|name| name != &req.space);
            self.save_settings(&settings)?;
        }
        Ok(Empty {})
    }

    /// Where the examples' files go: `Examples` in the folder new
    /// collections go to when the owner picked one on this computer, or in
    /// the data folder.
    fn examples_folder(&self, settings: &NodeSettings) -> std::path::PathBuf {
        let picked = settings
            .default_storage
            .as_deref()
            .and_then(|name| self.catalog.get_storage(name).ok())
            .and_then(|def| match def.kind {
                StorageKind::Folder { path } => Some(std::path::PathBuf::from(path)),
                _ => None,
            });
        picked
            .unwrap_or_else(|| std::path::PathBuf::from(self.catalog.root_path()))
            .join("Examples")
    }

    /// Add the examples a new data center starts with (one weekend in
    /// Lisbon as photos, notes and spending; a guide; a reading list; a
    /// home's temperatures), once: their files go into `Examples` where the
    /// owner keeps their data (a folder they picked, or the data folder),
    /// and each folder becomes the source of its collection. A name already
    /// taken is left alone, and so is a file already there. Returns the
    /// examples there now.
    pub async fn add_examples(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<SpaceSummary>, ApiError> {
        caller.require_owner()?;
        let mut settings = self.settings()?;
        if settings.examples_added_at_ms.is_none() {
            self.refresh()?;
            let session = self.session(None);
            let folder = self.examples_folder(&settings);
            for example in examples::examples() {
                if self.catalog.get_space(example.name).is_ok()
                    || self.catalog.get_location(example.name).is_ok()
                    || self.catalog.get_source(example.name).is_ok()
                {
                    continue;
                }
                let dir = folder.join(example.name);
                std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
                for (i, (file, bytes)) in (0u64..).zip(&example.files) {
                    let path = dir.join(file);
                    if path.exists() {
                        continue;
                    }
                    std::fs::write(&path, bytes).map_err(ApiError::internal)?;
                    if let Some(at) = example.modified {
                        let at =
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(at + 60 * i);
                        std::fs::File::options()
                            .write(true)
                            .open(&path)
                            .and_then(|f| f.set_modified(at))
                            .map_err(ApiError::internal)?;
                    }
                }
                let place = dir.to_string_lossy().into_owned();
                let config = match example.feed {
                    examples::Feed::Photos => SourceConfig::PhotosFolder { path: place },
                    examples::Feed::Notes => SourceConfig::NotesFolder { path: place },
                    examples::Feed::Csv { file, table } => {
                        sources::csv_config(&dir.join(file), table).await?
                    }
                };
                session
                    .add_source(example.name, config, Some(sources::EVERY), None)
                    .await?;
                settings.examples.push(example.name.to_string());
            }
            settings.examples_added_at_ms = Some(now_ms());
            self.save_settings(&settings)?;
        }
        Ok(self
            .list_spaces(caller, Empty {})
            .await?
            .into_iter()
            .filter(|space| space.example)
            .collect())
    }

    /// Where Spaces can be kept: this computer first, then each registered
    /// storage, with the number of local Spaces on it.
    pub async fn list_storages(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<StorageSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let mut counts: BTreeMap<Option<String>, u32> = BTreeMap::new();
        for space in self.catalog.list_spaces() {
            if space.kind == oikome_core::SpaceKind::Remote {
                continue;
            }
            let storage = self
                .catalog
                .get_location(&space.location)
                .ok()
                .and_then(|l| l.storage)
                .map(|b| b.storage);
            *counts.entry(storage).or_default() += 1;
        }
        let settings = self.settings()?;
        let default = settings
            .default_storage
            .filter(|name| self.catalog.get_storage(name).is_ok());
        let root = self.catalog.root_path().to_string();
        let room = places::space(std::path::Path::new(&root));
        let mut out = vec![StorageSummary {
            name: None,
            label: String::new(),
            is_default: default.is_none(),
            kind: StorageType::Disk,
            place: root,
            encrypted: false,
            spaces: counts.get(&None).copied().unwrap_or(0),
            free_bytes: room.map(|r| r.0),
            total_bytes: room.map(|r| r.1),
        }];
        for def in self.catalog.list_storages() {
            let (kind, place) = match &def.kind {
                StorageKind::Webdav { endpoint, .. } => (StorageType::Webdav, endpoint.clone()),
                StorageKind::Folder { path } => (StorageType::Folder, path.clone()),
                StorageKind::Drive { path } => (StorageType::Drive, path.clone()),
            };
            // A cloud drive's folder sits on this disk, but its room is the
            // drive's quota, which the folder doesn't tell.
            let room = match kind {
                StorageType::Folder => places::space(std::path::Path::new(&place)),
                _ => None,
            };
            out.push(StorageSummary {
                spaces: counts.get(&Some(def.name.clone())).copied().unwrap_or(0),
                label: settings
                    .storage_labels
                    .get(&def.name)
                    .cloned()
                    .unwrap_or_else(|| def.name.clone()),
                is_default: default.as_deref() == Some(def.name.as_str()),
                name: Some(def.name),
                kind,
                place,
                encrypted: def.encrypted,
                free_bytes: room.map(|r| r.0),
                total_bytes: room.map(|r| r.1),
            });
        }
        Ok(out)
    }

    /// Add a place to keep Spaces in, as the owner picked it: a folder (an
    /// `Oikome` folder is made inside), a cloud drive's folder (always
    /// encrypted) or a WebDAV server. The password goes to the system
    /// keychain, and the storage is checked before it is kept.
    pub async fn add_storage(
        &self,
        caller: &Caller,
        req: NewStorage,
    ) -> Result<StorageSummary, ApiError> {
        caller.require_owner()?;
        let label = req.label.trim();
        if label.is_empty()
            || label.chars().count() > MAX_NODE_NAME_CHARS
            || label.chars().any(char::is_control)
        {
            return Err(ApiError::invalid(format!(
                "a storage's name has 1 to {MAX_NODE_NAME_CHARS} characters"
            )));
        }
        self.refresh()?;
        let home = places::home();
        let (kind, encrypted) = match req.kind {
            StorageType::Disk => {
                return Err(ApiError::invalid("this computer is already a storage"));
            }
            StorageType::Webdav => (
                StorageKind::Webdav {
                    endpoint: req.address.trim().to_string(),
                    username: req.username.clone().filter(|u| !u.is_empty()),
                },
                req.encrypted.unwrap_or(false),
            ),
            StorageType::Folder | StorageType::Drive => {
                let (path, on_drive) = places::data_folder(req.address.trim(), home.as_deref())?;
                if on_drive || req.kind == StorageType::Drive {
                    (StorageKind::Drive { path }, true)
                } else {
                    (StorageKind::Folder { path }, req.encrypted.unwrap_or(false))
                }
            }
        };
        let taken: Vec<String> = self
            .catalog
            .list_storages()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let name = places::storage_name(label, &taken);
        let credentials = StorageCredentials {
            password: req.password.filter(|p| !p.is_empty()).map(Into::into),
            token: None,
            key: None,
        };
        self.session(None)
            .add_storage(&name, kind, encrypted, credentials)
            .await
            .map_err(|e| match e {
                // It did not answer, or refused the credentials: the owner
                // can fix what they entered.
                oikome_node::SessionError::Store { .. } => ApiError::invalid(e.to_string()),
                other => other.into(),
            })?;
        let mut settings = self.settings()?;
        settings
            .storage_labels
            .insert(name.clone(), label.to_string());
        if req.make_default.unwrap_or(false) {
            settings.default_storage = Some(name.clone());
        }
        self.save_settings(&settings)?;
        self.list_storages(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name.as_deref() == Some(name.as_str()))
            .ok_or_else(|| ApiError::internal("the added storage is missing"))
    }

    /// Choose where new collections go: a registered storage, or this
    /// computer (`None`).
    pub async fn set_default_storage(
        &self,
        caller: &Caller,
        req: DefaultStorage,
    ) -> Result<Vec<StorageSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if let Some(name) = &req.storage {
            self.catalog.get_storage(name)?;
        }
        let mut settings = self.settings()?;
        settings.default_storage = req.storage;
        self.save_settings(&settings)?;
        self.list_storages(caller, Empty {}).await
    }

    /// Stop keeping collections on a place added before. Refused while a
    /// collection is on it; the files there stay where they are. New
    /// collections go to Local when it was the default.
    pub async fn remove_storage(
        &self,
        caller: &Caller,
        req: StorageRef,
    ) -> Result<Vec<StorageSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let storages = self.list_storages(caller, Empty {}).await?;
        let place = storages
            .iter()
            .find(|s| s.name.as_deref() == Some(req.storage.as_str()))
            .ok_or_else(|| ApiError::not_found("there is no such storage"))?;
        if place.spaces > 0 {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!(
                    "{} still holds {} collection{}; remove {} first",
                    place.label,
                    place.spaces,
                    if place.spaces == 1 { "" } else { "s" },
                    if place.spaces == 1 { "it" } else { "them" },
                ),
            ));
        }
        self.session(None).remove_storage(&req.storage)?;
        let mut settings = self.settings()?;
        settings.storage_labels.remove(&req.storage);
        if settings.default_storage.as_deref() == Some(req.storage.as_str()) {
            settings.default_storage = None;
        }
        self.save_settings(&settings)?;
        self.list_storages(caller, Empty {}).await
    }

    /// Cloud drives synced on this computer and drives plugged into it,
    /// not yet added.
    pub async fn find_places(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<FoundPlace>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let added: Vec<String> = self
            .catalog
            .list_storages()
            .into_iter()
            .filter_map(|d| match d.kind {
                StorageKind::Folder { path } | StorageKind::Drive { path } => Some(path),
                StorageKind::Webdav { .. } => None,
            })
            .collect();
        let home = places::home();
        Ok(places::find_places(home.as_deref(), &places::volumes())
            .into_iter()
            .filter(|p| !added.iter().any(|a| a.starts_with(&p.path)))
            .collect())
    }

    /// The folders in a folder of this computer, to pick one.
    pub async fn list_folders(
        &self,
        caller: &Caller,
        req: FolderRef,
    ) -> Result<FolderListing, ApiError> {
        caller.require_owner()?;
        places::list_folders(req.path.as_deref())
    }

    /// Tables and views of a Space.
    pub async fn list_tables(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<Vec<TableSummary>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let views: BTreeSet<String> = self
            .catalog
            .list_views(&req.space)
            .into_iter()
            .map(|v| v.name)
            .collect();
        let local = self.catalog.get_space(&req.space)?.kind != oikome_core::SpaceKind::Remote;
        let session = self.session(None);
        let names = session.readable_relations(&req.space).await?;
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let rows = if local {
                // Both names were found in the catalog, and are quoted on top.
                let sql = format!(
                    "SELECT count(*) FROM {}.{}",
                    quote_ident(&req.space),
                    quote_ident(&name)
                );
                match session.sql(&sql).await {
                    Ok(QueryResult::Batches { batches, .. }) => first_count(&batches),
                    _ => None,
                }
            } else {
                None
            };
            out.push(TableSummary {
                kind: if views.contains(&name) {
                    RelationKind::View
                } else {
                    RelationKind::Table
                },
                name,
                rows,
            });
        }
        Ok(out)
    }

    /// A page of a table or view.
    pub async fn read_rows(&self, caller: &Caller, req: ReadRows) -> Result<Rows, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let limit = req
            .limit
            .unwrap_or(DEFAULT_PAGE_ROWS)
            .clamp(1, MAX_PAGE_ROWS);
        let offset = req.offset.unwrap_or(0);
        let session = self.session(None);
        let relations = session.readable_relations(&req.space).await?;
        if !relations.contains(&req.table) {
            return Err(ApiError::not_found(format!(
                "Table not found: {}.{}",
                req.space, req.table
            )));
        }
        // Both names were found in the catalog, and are quoted on top.
        let sql = format!(
            "SELECT * FROM {}.{} LIMIT {limit} OFFSET {offset}",
            quote_ident(&req.space),
            quote_ident(&req.table)
        );
        match session.sql(&sql).await? {
            QueryResult::Batches { schema, batches } => {
                encode_rows(&schema, &batches, usize::MAX, usize::MAX)
            }
            _ => Err(ApiError::internal("a query returned no rows")),
        }
    }

    /// Run the owner's statements in one session; returns the last result.
    /// A transaction left open is rolled back when the call ends.
    pub async fn run_sql(&self, caller: &Caller, req: RunSql) -> Result<SqlResult, ApiError> {
        caller.require_owner()?;
        if req.sql.len() > MAX_SQL_BYTES {
            return Err(ApiError::invalid(format!(
                "SQL is longer than {} KiB",
                MAX_SQL_BYTES / 1024
            )));
        }
        let statements = split_statements(&req.sql);
        if statements.is_empty() {
            return Err(ApiError::invalid("no statement to run"));
        }
        self.refresh()?;
        if let Some(space) = &req.space {
            self.catalog.get_space(space)?;
        }
        let session = self.session(req.space.clone());
        let mut last = QueryResult::Empty;
        for statement in &statements {
            last = session.sql(statement).await?;
        }
        Ok(match last {
            QueryResult::Batches { schema, batches } => SqlResult::Rows(encode_rows(
                &schema,
                &batches,
                MAX_RESULT_ROWS,
                MAX_RESULT_BYTES,
            )?),
            QueryResult::RowsAffected(n) => SqlResult::Done(SqlDone {
                rows_affected: Some(n),
            }),
            QueryResult::Empty => SqlResult::Done(SqlDone {
                rows_affected: None,
            }),
        })
    }

    // ----- Contacts -----

    /// Every contact, ordered by DID.
    pub async fn list_contacts(&self, caller: &Caller, _: Empty) -> Result<Vec<Contact>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        Ok(self
            .catalog
            .list_contacts()
            .into_iter()
            .map(contact_of)
            .collect())
    }

    /// Add a contact by the DID of their node.
    pub async fn add_contact(
        &self,
        caller: &Caller,
        req: ContactInput,
    ) -> Result<Contact, ApiError> {
        caller.require_owner()?;
        let did = parse_did(&req.did)?;
        if did == self.node_did()? {
            return Err(ApiError::invalid("this is your own node"));
        }
        self.refresh()?;
        if self.catalog.get_contact(&did).is_some() {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!("{did} is already a contact"),
            ));
        }
        Ok(contact_of(self.catalog.add_contact(&did, &req.name)?))
    }

    /// Give a contact another name.
    pub async fn rename_contact(
        &self,
        caller: &Caller,
        req: ContactInput,
    ) -> Result<Contact, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        Ok(contact_of(
            self.catalog.rename_contact(&req.did, &req.name)?,
        ))
    }

    /// Remove a contact and stop sharing with them: their active shares are
    /// revoked and their grants on whole Spaces removed.
    pub async fn remove_contact(
        &self,
        caller: &Caller,
        req: DidRef,
    ) -> Result<RemovedContact, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if self.catalog.get_contact(&req.did).is_none() {
            return Err(ApiError::not_found(format!(
                "Contact not found: {}",
                req.did
            )));
        }
        let session = self.session(None);
        let mut revoked_shares = 0;
        for share in self.shares()? {
            if share.did == req.did && share.status == ShareStatus::Active {
                session.revoke_token(&share.id)?;
                revoked_shares += 1;
            }
        }
        for grant in self.catalog.list_space_grants() {
            if grant.principal == req.did {
                session.revoke_space(&grant.space, std::slice::from_ref(&req.did))?;
            }
        }
        let contact = self.catalog.remove_contact(&req.did)?;
        Ok(RemovedContact {
            contact: contact_of(contact),
            revoked_shares,
        })
    }

    // ----- Shares -----

    /// Every share this node made: UCANs whose capabilities all name one
    /// Space of this node (tokens issued with SQL for several Spaces or `*`
    /// are not shares), newest first.
    fn shares(&self) -> Result<Vec<Share>, ApiError> {
        let prefix = format!("space://{}/", self.node_did()?);
        let revoked = self.catalog.revoked_ucans();
        let names: BTreeMap<String, String> = self
            .catalog
            .list_contacts()
            .into_iter()
            .map(|c| (c.did, c.name))
            .collect();
        let now = unix_now();
        let mut shares = Vec::new();
        for ucan in self.catalog.list_issued_ucans() {
            let spaces: Option<BTreeSet<&str>> = ucan
                .capabilities
                .iter()
                .map(|c| c.with.strip_prefix(prefix.as_str()))
                .collect();
            let Some(spaces) = spaces else { continue };
            let abilities_ok = ucan
                .capabilities
                .iter()
                .all(|c| c.can == ABILITY_QUERY || c.can == ABILITY_READ);
            let space = match spaces.into_iter().collect::<Vec<_>>().as_slice() {
                [space] if *space != "*" && abilities_ok => space.to_string(),
                _ => continue,
            };
            let access = if ucan.capabilities.iter().any(|c| c.can == ABILITY_READ) {
                ShareAccess::Copy
            } else {
                ShareAccess::Query
            };
            let status = if revoked.contains(&ucan.id) {
                ShareStatus::Revoked
            } else if ucan.expires_at <= now {
                ShareStatus::Expired
            } else {
                ShareStatus::Active
            };
            shares.push(Share {
                contact_name: names.get(&ucan.audience).cloned(),
                id: ucan.id,
                space,
                did: ucan.audience,
                access,
                issued_at_ms: ucan.issued_at_ms,
                expires_at: ucan.expires_at,
                status,
            });
        }
        shares.sort_by(|a, b| b.issued_at_ms.cmp(&a.issued_at_ms).then(a.id.cmp(&b.id)));
        Ok(shares)
    }

    /// Share a whole Space with a contact: grant them every table of it
    /// (now and later) and issue the token they mount it with.
    pub async fn share_space(
        &self,
        caller: &Caller,
        req: ShareSpace,
    ) -> Result<NewShare, ApiError> {
        caller.require_owner()?;
        let did = parse_did(&req.did)?;
        let days = req.expires_in_days.unwrap_or(DEFAULT_SHARE_DAYS);
        if !(1..=MAX_SHARE_DAYS).contains(&days) {
            return Err(ApiError::invalid(format!(
                "a share lasts 1 to {MAX_SHARE_DAYS} days"
            )));
        }
        self.refresh()?;
        if self.catalog.get_contact(&did).is_none() {
            return Err(ApiError::not_found(format!(
                "{did} is not a contact; add it before sharing with it"
            )));
        }
        let space = self.catalog.get_space(&req.space)?;
        let allow_copy = req.allow_copy.unwrap_or(false);
        if allow_copy && space.kind != oikome_core::SpaceKind::Analytical {
            return Err(ApiError::invalid(
                "only analytical spaces can be copied; publish a transactional space and share the publication",
            ));
        }
        let session = self.session(None);
        let had_grant = self.catalog.has_space_grant(&did, &req.space);
        session.grant_space(&req.space, std::slice::from_ref(&did))?;
        let issued = session.issue_token(&IssueUcanRequest {
            spaces: vec![req.space.clone()],
            audience: did.clone(),
            ability: if allow_copy {
                ABILITY_READ
            } else {
                ABILITY_QUERY
            }
            .to_string(),
            lifetime_secs: u64::from(days) * 86_400,
        });
        let issued = match issued {
            Ok(issued) => issued,
            Err(e) => {
                if !had_grant {
                    let _ = session.revoke_space(&req.space, std::slice::from_ref(&did));
                }
                return Err(e.into());
            }
        };
        Ok(NewShare {
            id: issued.id,
            space: req.space,
            did,
            access: if allow_copy {
                ShareAccess::Copy
            } else {
                ShareAccess::Query
            },
            expires_at: issued.expires_at,
            token: issued.token,
        })
    }

    /// Shares, newest first, optionally of one Space.
    pub async fn list_shares(
        &self,
        caller: &Caller,
        req: ListShares,
    ) -> Result<Vec<Share>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        Ok(self
            .shares()?
            .into_iter()
            .filter(|s| req.space.as_ref().is_none_or(|space| &s.space == space))
            .collect())
    }

    /// Revoke a share. When it was the contact's last active share of the
    /// Space, their grant on the Space is removed too.
    pub async fn revoke_share(
        &self,
        caller: &Caller,
        req: ShareRef,
    ) -> Result<RevokedShare, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let shares = self.shares()?;
        let share = shares
            .iter()
            .find(|s| s.id == req.id)
            .ok_or_else(|| ApiError::not_found(format!("Share not found: {}", req.id)))?;
        let session = self.session(None);
        session.revoke_token(&share.id)?;
        let others = shares.iter().any(|s| {
            s.id != share.id
                && s.did == share.did
                && s.space == share.space
                && s.status == ShareStatus::Active
        });
        let grant_removed = !others && self.catalog.has_space_grant(&share.did, &share.space);
        if grant_removed {
            session.revoke_space(&share.space, std::slice::from_ref(&share.did))?;
        }
        Ok(RevokedShare {
            id: share.id.clone(),
            grant_removed,
        })
    }

    // ----- Access log -----

    /// Requests peers and browsers sent this node, newest first.
    pub async fn list_access(
        &self,
        caller: &Caller,
        req: ListAccess,
    ) -> Result<Vec<AccessItem>, ApiError> {
        caller.require_owner()?;
        let Some(log) = self.access_log.clone() else {
            return Ok(Vec::new());
        };
        if let Some(space) = &req.space {
            validate_space_name(space)?;
        }
        let query = AccessQuery {
            before_id: req.before_id,
            limit: req.limit.unwrap_or(DEFAULT_ACCESS_ROWS),
            caller: req.did,
            space: req.space,
        };
        let records = tokio::task::spawn_blocking(move || log.list(&query))
            .await
            .map_err(ApiError::internal)??;
        let names: BTreeMap<String, String> = self
            .catalog
            .list_contacts()
            .into_iter()
            .map(|c| (c.did, c.name))
            .collect();
        Ok(records
            .into_iter()
            .map(|r| AccessItem {
                contact_name: r.caller.as_ref().and_then(|d| names.get(d).cloned()),
                id: r.id,
                at_ms: r.at_ms,
                did: r.caller,
                method: r.method,
                spaces: r.spaces,
                share_id: r.token_id,
                outcome: match r.outcome {
                    Outcome::Allowed => AccessOutcome::Allowed,
                    Outcome::Denied => AccessOutcome::Denied,
                    Outcome::Failed => AccessOutcome::Failed,
                },
                reason: r.reason,
                count: r.count,
            })
            .collect())
    }
}

/// Restore a node's identity from its recovery key (`oikome identity
/// restore`): write the seed as the node key of `catalog`'s data directory.
/// Only a node that holds nothing yet (no Spaces, contacts or issued tokens)
/// can take another identity; a running node keeps its old one until it is
/// restarted. Returns the restored DID.
pub fn restore_identity(catalog: &OikomeCatalog, recovery_key: &str) -> Result<String, ApiError> {
    if std::env::var_os(ENV_NODE_KEY).is_some_and(|v| !v.is_empty()) {
        return Err(ApiError::invalid(format!(
            "{ENV_NODE_KEY} is set and overrides the key file; unset it to restore"
        )));
    }
    let seed = decode_recovery_key(recovery_key).map_err(|e| ApiError::invalid(e.to_string()))?;
    catalog.refresh()?;
    let busy = !catalog.list_spaces().is_empty()
        || !catalog.list_contacts().is_empty()
        || !catalog.list_issued_ucans().is_empty();
    if busy {
        return Err(ApiError::new(
            ErrorCode::Conflict,
            "this node already holds data; restore into a new, empty data directory",
        ));
    }
    let path = catalog.oikome_dir().join("identity").join(NODE_KEY_FILE);
    let identity = NodeIdentity::write_key_file(&path, &seed).map_err(ApiError::internal)?;
    Ok(identity.did().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Int64Array;
    use arrow_ipc::reader::StreamReader;
    use arrow_schema::{DataType, Field, Schema};

    fn batch(n: i64) -> (SchemaRef, RecordBatch) {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from_iter_values(0..n))],
        )
        .unwrap();
        (schema, batch)
    }

    fn rows_in(rows: &Rows) -> usize {
        StreamReader::try_new(rows.ipc.as_slice(), None)
            .unwrap()
            .map(|b| b.unwrap().num_rows())
            .sum()
    }

    #[test]
    fn results_are_cut_at_the_row_limit() {
        let (schema, b) = batch(600);
        let rows = encode_rows(&schema, &[b.clone(), b.clone()], 1000, usize::MAX).unwrap();
        assert!(rows.truncated);
        assert_eq!(rows_in(&rows), 1000);

        let rows = encode_rows(&schema, std::slice::from_ref(&b), 1000, usize::MAX).unwrap();
        assert!(!rows.truncated);
        assert_eq!(rows_in(&rows), 600);

        // Empty batches after the limit do not count as cut-off rows.
        let (_, empty) = batch(0);
        let rows = encode_rows(&schema, &[b, empty], 600, usize::MAX).unwrap();
        assert!(!rows.truncated);
    }

    #[test]
    fn results_are_cut_at_the_byte_limit() {
        let (schema, b) = batch(1000);
        let rows = encode_rows(&schema, &[b.clone(), b.clone(), b], usize::MAX, 1).unwrap();
        assert!(rows.truncated);
        assert_eq!(rows_in(&rows), 1000);
    }

    #[test]
    fn empty_results_keep_their_schema() {
        let (schema, _) = batch(0);
        let rows = encode_rows(&schema, &[], 10, 10).unwrap();
        let reader = StreamReader::try_new(rows.ipc.as_slice(), None).unwrap();
        assert_eq!(reader.schema().field(0).name(), "n");
    }
}
