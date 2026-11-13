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

//! Node-level Space / Location registries, persisted as JSON under
//! `<root>/.oikome/`. The directory starts with a dot so Iceberg namespace
//! listing ignores it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use iceberg::{Catalog, NamespaceIdent};
use oikome_core::{AccessMode, CatalogError, CreateSpaceRequest, SpaceKind};
use serde::{Deserialize, Serialize};

use crate::acl::{Grant, RowPolicy, SpaceGrant, ViewDef};
use crate::catalog::OikomeCatalog;
use crate::compute::{ComputeItem, ComputeQuota};
use crate::contact::Contact;
use crate::graph::GraphDef;
use crate::peer::Peer;
use crate::source::SourceDef;
use crate::space::{
    validate_binding, InPlaceSource, Location, LocationFormat, LocationId, Space, STORAGE_DISK,
};
use crate::storage::StorageDef;
use crate::ucan::{IssuedUcan, RemoteToken, Revocation};

/// Name of the hidden directory holding registries and SQLite files.
pub const OIKOME_DIR: &str = ".oikome";
const SPACES_FILE: &str = "spaces.json";
const LOCATIONS_FILE: &str = "locations.json";
const VIEWS_FILE: &str = "views.json";
const GRAPHS_FILE: &str = "graphs.json";
const GRANTS_FILE: &str = "grants.json";
const SPACE_GRANTS_FILE: &str = "space_grants.json";
const CONTACTS_FILE: &str = "contacts.json";
const SOURCES_FILE: &str = "sources.json";
const COMPUTE_FILE: &str = "compute.json";
const POLICIES_FILE: &str = "policies.json";
const UCANS_FILE: &str = "ucans.json";
const REVOCATIONS_FILE: &str = "revocations.json";
const PEERS_FILE: &str = "peers.json";
const STORAGES_FILE: &str = "storages.json";
/// Held UCANs live apart from the other registries, readable by the owner only.
const IDENTITY_DIR: &str = "identity";
const REMOTE_TOKENS_FILE: &str = "remote_tokens.json";
const REGISTRY_VERSION: u32 = 1;
/// Advisory lock held while registries are read from or written to disk, so
/// the processes of one node (`serve`, `sql`) never interleave them.
const LOCK_FILE: &str = "registry.lock";

/// Namespace property marking a namespace as backing an analytical Space.
pub const NS_PROP_SPACE_KIND: &str = "oikome.space.kind";

#[derive(Debug, Serialize, Deserialize)]
struct RegistryFile<T> {
    version: u32,
    items: Vec<T>,
}

/// In-memory copy of all node-level registries.
#[derive(Debug, Default, Clone)]
pub(crate) struct Registry {
    pub(crate) spaces: BTreeMap<String, Space>,
    pub(crate) locations: BTreeMap<LocationId, Location>,
    /// Keyed by `(space, view)`.
    pub(crate) views: BTreeMap<(String, String), ViewDef>,
    /// Property graphs, keyed by `(space, graph)`.
    pub(crate) graphs: BTreeMap<(String, String), GraphDef>,
    /// Keyed by `(principal, space, relation)`.
    pub(crate) grants: BTreeMap<(String, String, String), Grant>,
    /// Whole-Space grants, keyed by `(principal, space)`.
    pub(crate) space_grants: BTreeMap<(String, String), SpaceGrant>,
    /// Keyed by `(space, relation, policy name)`.
    pub(crate) policies: BTreeMap<(String, String, String), RowPolicy>,
    /// UCANs this node issued, keyed by token id.
    pub(crate) issued: BTreeMap<String, IssuedUcan>,
    /// Revoked token ids.
    pub(crate) revoked: BTreeMap<String, Revocation>,
    /// UCANs held for remote Spaces, keyed by Space name.
    pub(crate) tokens: BTreeMap<String, RemoteToken>,
    /// Discovered peers, keyed by DID.
    pub(crate) peers: BTreeMap<String, Peer>,
    /// Registered storages, keyed by name.
    pub(crate) storages: BTreeMap<String, StorageDef>,
    /// The owner's contacts, keyed by DID.
    pub(crate) contacts: BTreeMap<String, Contact>,
    /// Registered sources (connectors), keyed by name.
    pub(crate) sources: BTreeMap<String, SourceDef>,
    /// Compute methods offered to peers.
    pub(crate) compute_offers: BTreeSet<String>,
    /// Compute quotas, keyed by peer DID.
    pub(crate) compute_quotas: BTreeMap<String, ComputeQuota>,
}

fn registry_err(context: &str, e: impl std::fmt::Display) -> CatalogError {
    CatalogError::Registry(format!("{context}: {e}"))
}

/// Read a registry file, feeding its bytes (or their absence) to `hasher`.
fn read_items<T: for<'de> Deserialize<'de>>(
    path: &Path,
    hasher: &mut impl Hasher,
) -> Result<Vec<T>, CatalogError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            bytes.hash(hasher);
            let file: RegistryFile<T> = serde_json::from_slice(&bytes)
                .map_err(|e| registry_err(&format!("parsing {}", path.display()), e))?;
            if file.version != REGISTRY_VERSION {
                return Err(CatalogError::Registry(format!(
                    "{} has version {}, expected {}",
                    path.display(),
                    file.version,
                    REGISTRY_VERSION
                )));
            }
            Ok(file.items)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            None::<()>.hash(hasher);
            Ok(Vec::new())
        }
        Err(e) => Err(e.into()),
    }
}

fn serialise<T: Serialize>(path: &Path, items: Vec<T>) -> Result<Vec<u8>, CatalogError> {
    let file = RegistryFile {
        version: REGISTRY_VERSION,
        items,
    };
    serde_json::to_vec_pretty(&file)
        .map_err(|e| registry_err(&format!("serialising {}", path.display()), e))
}

fn write_items<T: Serialize>(path: &Path, items: Vec<T>) -> Result<(), CatalogError> {
    let bytes = serialise(path, items)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Like [`write_items`], but the file is created readable by the owner only.
fn write_items_private<T: Serialize>(path: &Path, items: Vec<T>) -> Result<(), CatalogError> {
    let bytes = serialise(path, items)?;
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Registry {
    /// Load every registry file, returning a fingerprint of their contents
    /// so later changes (by another process) can be detected.
    pub(crate) fn load(root: &Path) -> Result<(Self, u64), CatalogError> {
        let dir = root.join(OIKOME_DIR);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let spaces: Vec<Space> = read_items(&dir.join(SPACES_FILE), &mut h)?;
        let locations: Vec<Location> = read_items(&dir.join(LOCATIONS_FILE), &mut h)?;
        let views: Vec<ViewDef> = read_items(&dir.join(VIEWS_FILE), &mut h)?;
        let graphs: Vec<GraphDef> = read_items(&dir.join(GRAPHS_FILE), &mut h)?;
        let grants: Vec<Grant> = read_items(&dir.join(GRANTS_FILE), &mut h)?;
        let space_grants: Vec<SpaceGrant> = read_items(&dir.join(SPACE_GRANTS_FILE), &mut h)?;
        let policies: Vec<RowPolicy> = read_items(&dir.join(POLICIES_FILE), &mut h)?;
        let issued: Vec<IssuedUcan> = read_items(&dir.join(UCANS_FILE), &mut h)?;
        let revoked: Vec<Revocation> = read_items(&dir.join(REVOCATIONS_FILE), &mut h)?;
        let tokens: Vec<RemoteToken> =
            read_items(&dir.join(IDENTITY_DIR).join(REMOTE_TOKENS_FILE), &mut h)?;
        let peers: Vec<Peer> = read_items(&dir.join(PEERS_FILE), &mut h)?;
        let storages: Vec<StorageDef> = read_items(&dir.join(STORAGES_FILE), &mut h)?;
        let contacts: Vec<Contact> = read_items(&dir.join(CONTACTS_FILE), &mut h)?;
        let sources: Vec<SourceDef> = read_items(&dir.join(SOURCES_FILE), &mut h)?;
        let compute: Vec<ComputeItem> = read_items(&dir.join(COMPUTE_FILE), &mut h)?;
        let mut compute_offers = BTreeSet::new();
        let mut compute_quotas = BTreeMap::new();
        for item in compute {
            match item {
                ComputeItem::Offer { method } => {
                    compute_offers.insert(method);
                }
                ComputeItem::Quota(quota) => {
                    compute_quotas.insert(quota.peer.clone(), quota);
                }
            }
        }
        let registry = Self {
            spaces: spaces.into_iter().map(|s| (s.name.clone(), s)).collect(),
            locations: locations.into_iter().map(|l| (l.id.clone(), l)).collect(),
            views: views.into_iter().map(|v| (v.key(), v)).collect(),
            graphs: graphs.into_iter().map(|g| (g.key(), g)).collect(),
            grants: grants.into_iter().map(|g| (g.key(), g)).collect(),
            space_grants: space_grants.into_iter().map(|g| (g.key(), g)).collect(),
            policies: policies.into_iter().map(|p| (p.key(), p)).collect(),
            issued: issued.into_iter().map(|u| (u.id.clone(), u)).collect(),
            revoked: revoked.into_iter().map(|r| (r.id.clone(), r)).collect(),
            tokens: tokens.into_iter().map(|t| (t.space.clone(), t)).collect(),
            peers: peers.into_iter().map(|p| (p.did.clone(), p)).collect(),
            storages: storages.into_iter().map(|s| (s.name.clone(), s)).collect(),
            contacts: contacts.into_iter().map(|c| (c.did.clone(), c)).collect(),
            sources: sources.into_iter().map(|s| (s.name.clone(), s)).collect(),
            compute_offers,
            compute_quotas,
        };
        Ok((registry, h.finish()))
    }

    pub(crate) fn save(&self, root: &Path) -> Result<(), CatalogError> {
        let dir = root.join(OIKOME_DIR);
        std::fs::create_dir_all(&dir)?;
        write_items(
            &dir.join(SPACES_FILE),
            self.spaces.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(LOCATIONS_FILE),
            self.locations.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(VIEWS_FILE),
            self.views.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(GRAPHS_FILE),
            self.graphs.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(GRANTS_FILE),
            self.grants.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(SPACE_GRANTS_FILE),
            self.space_grants.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(POLICIES_FILE),
            self.policies.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(UCANS_FILE),
            self.issued.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(REVOCATIONS_FILE),
            self.revoked.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(PEERS_FILE),
            self.peers.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(STORAGES_FILE),
            self.storages.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(CONTACTS_FILE),
            self.contacts.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(SOURCES_FILE),
            self.sources.values().cloned().collect(),
        )?;
        write_items(
            &dir.join(COMPUTE_FILE),
            self.compute_offers
                .iter()
                .map(|method| ComputeItem::Offer {
                    method: method.clone(),
                })
                .chain(
                    self.compute_quotas
                        .values()
                        .cloned()
                        .map(ComputeItem::Quota),
                )
                .collect(),
        )?;
        if !self.tokens.is_empty() || dir.join(IDENTITY_DIR).join(REMOTE_TOKENS_FILE).exists() {
            let identity = dir.join(IDENTITY_DIR);
            std::fs::create_dir_all(&identity)?;
            write_items_private(
                &identity.join(REMOTE_TOKENS_FILE),
                self.tokens.values().cloned().collect(),
            )?;
        }
        Ok(())
    }
}

/// An advisory lock on `.oikome/registry.lock`, released when dropped.
///
/// Writers hold it exclusively from reading the files to the last rename;
/// readers hold it shared while they read. Without it, one process could
/// save a copy loaded before another process's write (a `REVOKE UCAN` run by
/// `oikome sql` while `serve` records a grant) and undo that write.
struct RegistryLock {
    _file: std::fs::File,
}

impl RegistryLock {
    fn acquire(root: &Path, exclusive: bool) -> Result<Self, CatalogError> {
        let dir = root.join(OIKOME_DIR);
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(LOCK_FILE))?;
        if exclusive {
            file.lock()?;
        } else {
            file.lock_shared()?;
        }
        Ok(Self { _file: file })
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl OikomeCatalog {
    /// The hidden directory holding registries and SQLite files.
    /// Where a serving node keeps its signed WebRTC access record (§10),
    /// for `SHOW WEBRTC` in any process of the node.
    pub fn webrtc_record_path(&self) -> PathBuf {
        self.oikome_dir().join("webrtc.record")
    }

    pub fn oikome_dir(&self) -> PathBuf {
        Path::new(self.root_path()).join(OIKOME_DIR)
    }

    pub(crate) fn with_registry<T>(&self, f: impl FnOnce(&Registry) -> T) -> T {
        let guard = self.registry.read().unwrap_or_else(|p| p.into_inner());
        f(&guard)
    }

    /// Apply `f` to the registry and persist it if `f` succeeds.
    ///
    /// Under the registry lock, the files are read again first, so a write
    /// by another process of the node since the last [`Self::refresh`] is
    /// kept rather than overwritten. Locks are always taken in the order
    /// file lock, fingerprint, registry (as in `refresh`).
    pub(crate) fn mutate_registry<T>(
        &self,
        f: impl FnOnce(&mut Registry) -> Result<T, CatalogError>,
    ) -> Result<T, CatalogError> {
        let root = Path::new(self.root_path());
        let _lock = RegistryLock::acquire(root, true)?;
        let mut fp = self.registry_fp.lock().unwrap_or_else(|p| p.into_inner());
        let mut guard = self.registry.write().unwrap_or_else(|p| p.into_inner());
        let (mut next, _) = Registry::load(root)?;
        let out = f(&mut next)?;
        next.save(root)?;
        *guard = next;
        // Unknown until re-read: the next refresh() reloads from disk.
        *fp = None;
        Ok(out)
    }

    /// Reload the registries if their files changed since they were last
    /// read, e.g. because `oikome sql` ran a `GRANT` or `REVOKE UCAN` in
    /// another process. Returns whether anything was reloaded.
    ///
    /// Long-running servers call this before authorizing a request, so a
    /// revocation takes effect on the next call.
    pub fn refresh(&self) -> Result<bool, CatalogError> {
        let root = Path::new(self.root_path());
        let _lock = RegistryLock::acquire(root, false)?;
        let mut fp = self.registry_fp.lock().unwrap_or_else(|p| p.into_inner());
        let (fresh, fingerprint) = Registry::load(root)?;
        if *fp == Some(fingerprint) {
            return Ok(false);
        }
        *self.registry.write().unwrap_or_else(|p| p.into_inner()) = fresh;
        *fp = Some(fingerprint);
        Ok(true)
    }

    /// Check a `CREATE SPACE` request against the registry without
    /// registering anything. Returns the Space and Location it would
    /// create, and whether the Location is new.
    pub fn plan_space(
        &self,
        request: &CreateSpaceRequest,
    ) -> Result<(Space, Location, bool), CatalogError> {
        self.with_registry(|reg| {
            if reg.spaces.contains_key(&request.name) {
                return Err(CatalogError::SpaceExists(request.name.clone()));
            }
            let existing = match &request.location {
                Some(id) => reg.locations.get(id),
                None => None,
            };
            if let Some(storage) = request.storage.as_deref().filter(|s| *s != STORAGE_DISK) {
                if !reg.storages.contains_key(storage) {
                    return Err(CatalogError::StorageNotFound(storage.to_string()));
                }
            }
            if let Some(place) = request
                .place
                .as_deref()
                .filter(|p| *p != crate::space::PLACE_LOCAL)
            {
                if !reg.storages.contains_key(place) {
                    return Err(CatalogError::StorageNotFound(place.to_string()));
                }
            }
            if request.location.is_none() && reg.locations.contains_key(&request.name) {
                return Err(CatalogError::Registry(format!(
                    "a location named '{}' already exists; bind it with LOCATION = '{}'",
                    request.name, request.name
                )));
            }
            let (space, location) = validate_binding(request, existing, now_ms())?;
            Ok((space, location, existing.is_none()))
        })
    }

    /// Create a Space (and, unless `LOCATION` names an existing one, its Location).
    ///
    /// Analytical Spaces get an Iceberg namespace of the same name; the
    /// database file of a transactional Space is created by its engine on
    /// first open.
    pub async fn create_space(&self, request: CreateSpaceRequest) -> Result<Space, CatalogError> {
        let (space, location, is_new_location) = self.plan_space(&request)?;

        if is_new_location {
            match &location.format {
                LocationFormat::IcebergParquet { namespace } => {
                    let ns = NamespaceIdent::new(namespace.clone());
                    let mut props = HashMap::new();
                    props.insert(
                        NS_PROP_SPACE_KIND.to_string(),
                        SpaceKind::Analytical.to_string(),
                    );
                    self.create_namespace(&ns, props)
                        .await
                        .map_err(|e| CatalogError::Iceberg(e.to_string()))?;
                }
                LocationFormat::SqliteFile { path } => {
                    if let Some(parent) = Path::new(self.root_path()).join(path).parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                LocationFormat::Lance { root } => {
                    std::fs::create_dir_all(Path::new(self.root_path()).join(root))?;
                }
                // validate_binding never creates these (see create_peer_space).
                LocationFormat::Remote { .. } | LocationFormat::Replica { .. } => {}
                // The node fills the copy (`replica_dir`) from the storage.
                LocationFormat::StorageCopy => {}
                // The folder is the owner's; the node keeps only its index.
                LocationFormat::InPlace { .. } => {}
            }
        }

        self.mutate_registry(|reg| {
            // Checked again on the files just read: another process may have
            // created the Space meanwhile.
            if reg.spaces.contains_key(&space.name) {
                return Err(CatalogError::SpaceExists(space.name.clone()));
            }
            reg.spaces.insert(space.name.clone(), space.clone());
            reg.locations.insert(location.id.clone(), location);
            Ok(())
        })?;
        Ok(space)
    }

    /// Look up a Space by name.
    pub fn get_space(&self, name: &str) -> Result<Space, CatalogError> {
        self.with_registry(|reg| {
            reg.spaces
                .get(name)
                .cloned()
                .ok_or_else(|| CatalogError::SpaceNotFound(name.to_string()))
        })
    }

    /// All Spaces, ordered by name.
    pub fn list_spaces(&self) -> Vec<Space> {
        self.with_registry(|reg| reg.spaces.values().cloned().collect())
    }

    /// Remove a Space from the registry, together with its views, grants
    /// (table and whole-Space) and row policies.
    ///
    /// Data is left in place: the Iceberg namespace or SQLite file stays on
    /// disk and the Location remains registered (with its writer cleared if
    /// this Space owned it) so it can be re-bound later.
    pub fn drop_space(&self, name: &str) -> Result<(), CatalogError> {
        self.mutate_registry(|reg| {
            let space = reg
                .spaces
                .remove(name)
                .ok_or_else(|| CatalogError::SpaceNotFound(name.to_string()))?;
            if let Some(location) = reg.locations.get_mut(&space.location) {
                if location.writer.as_deref() == Some(name) {
                    location.writer = None;
                }
            }
            reg.views.retain(|(s, _), _| s != name);
            reg.graphs.retain(|(s, _), _| s != name);
            reg.grants.retain(|(_, s, _), _| s != name);
            reg.space_grants.retain(|(_, s), _| s != name);
            reg.policies.retain(|(s, _, _), _| s != name);
            // Another node's Space, or a copy of one on a storage: forget its
            // Location with the token. The caller removes a copy's files (see
            // `replica_dir`).
            // A folder viewed in place: its Location is the Space's alone.
            // The caller removes its index (see `index_dir`).
            if reg.locations.get(&space.location).is_some_and(|l| {
                l.format.is_peer_space()
                    || l.format.is_copy()
                    || matches!(l.format, LocationFormat::InPlace { .. })
            }) {
                reg.locations.remove(&space.location);
            }
            reg.tokens.remove(name);
            Ok(())
        })
    }

    /// Forget a Location no Space is bound to (its files stay where they
    /// are); used when a Space's data moves elsewhere and its name is taken
    /// again, as when a folder source becomes a folder viewed in place.
    pub fn forget_location(&self, id: &str) -> Result<(), CatalogError> {
        self.mutate_registry(|reg| {
            if reg.spaces.values().any(|s| s.location == id) {
                return Err(CatalogError::Registry(format!(
                    "location '{id}' is still in use"
                )));
            }
            reg.locations.remove(id);
            Ok(())
        })
    }

    /// Where the index of a folder viewed in place is kept: a cache under
    /// `.oikome/index/<location id>/`, rebuilt when it is missing.
    pub fn index_dir(&self, location: &Location) -> PathBuf {
        self.oikome_dir().join("index").join(&location.id)
    }

    /// Mark `target` as the publication Space of `source`: from now on it is
    /// only written by `PUBLISH SPACE source`.
    pub fn mark_published(&self, target: &str, source: &str) -> Result<Space, CatalogError> {
        self.mutate_registry(|reg| {
            let space = reg
                .spaces
                .get_mut(target)
                .ok_or_else(|| CatalogError::SpaceNotFound(target.to_string()))?;
            space.published_from = Some(source.to_string());
            Ok(space.clone())
        })
    }

    /// Turn an empty analytical Space into a file-backed one on its first
    /// import, keeping its name and whole-Space grants. The old Location is
    /// left registered so a later removal still leaves the name taken.
    pub fn make_space_in_place(
        &self,
        name: &str,
        path: String,
        sources: Option<Vec<InPlaceSource>>,
    ) -> Result<Location, CatalogError> {
        self.mutate_registry(|reg| {
            let space = reg
                .spaces
                .get_mut(name)
                .ok_or_else(|| CatalogError::SpaceNotFound(name.to_string()))?;
            let old = reg
                .locations
                .get(&space.location)
                .ok_or_else(|| CatalogError::LocationNotFound(space.location.clone()))?;
            if !matches!(old.format, LocationFormat::IcebergParquet { .. }) {
                return Err(CatalogError::Registry(format!(
                    "space '{name}' is not an empty analytical Space"
                )));
            }
            let id = format!("{name}-files");
            if reg.locations.contains_key(&id) {
                return Err(CatalogError::Registry(format!(
                    "location '{id}' already exists"
                )));
            }
            let location = Location {
                id: id.clone(),
                format: LocationFormat::InPlace {
                    place: None,
                    path,
                    sources,
                },
                residency: vec![STORAGE_DISK.to_string()],
                writer: None,
                storage: None,
            };
            reg.locations.insert(id.clone(), location.clone());
            space.location = id;
            space.access = AccessMode::ReadOnly;
            reg.views.retain(|(space, _), _| space != name);
            reg.graphs.retain(|(space, _), _| space != name);
            reg.grants.retain(|(_, space, _), _| space != name);
            reg.policies.retain(|(space, _, _), _| space != name);
            Ok(location)
        })
    }

    /// Add local file references to an existing file collection, keeping its
    /// name and replacing its index with the expanded set of sources.
    pub fn add_in_place_files(
        &self,
        name: &str,
        path: String,
        sources: Vec<InPlaceSource>,
    ) -> Result<Location, CatalogError> {
        self.mutate_registry(|reg| {
            let space = reg
                .spaces
                .get(name)
                .ok_or_else(|| CatalogError::SpaceNotFound(name.to_string()))?;
            let location = reg
                .locations
                .get_mut(&space.location)
                .ok_or_else(|| CatalogError::LocationNotFound(space.location.clone()))?;
            if !matches!(location.format, LocationFormat::InPlace { place: None, .. }) {
                return Err(CatalogError::Registry(format!(
                    "space '{name}' is not a local file collection"
                )));
            }
            location.format = LocationFormat::InPlace {
                place: None,
                path,
                sources: Some(sources),
            };
            Ok(location.clone())
        })
    }

    /// Look up a Location by id.
    pub fn get_location(&self, id: &str) -> Result<Location, CatalogError> {
        self.with_registry(|reg| {
            reg.locations
                .get(id)
                .cloned()
                .ok_or_else(|| CatalogError::LocationNotFound(id.to_string()))
        })
    }

    /// All Locations, ordered by id.
    pub fn list_locations(&self) -> Vec<Location> {
        self.with_registry(|reg| reg.locations.values().cloned().collect())
    }

    /// The Iceberg namespace backing an analytical Space.
    pub fn space_namespace(&self, space: &Space) -> Result<NamespaceIdent, CatalogError> {
        match self.get_location(&space.location)?.format {
            LocationFormat::IcebergParquet { namespace } => Ok(NamespaceIdent::new(namespace)),
            other => Err(CatalogError::KindFormatMismatch {
                kind: space.kind.to_string(),
                format: other.name().to_string(),
            }),
        }
    }

    /// Absolute path of a Lance Location's directory of datasets.
    pub fn lance_root(&self, location: &Location) -> Result<PathBuf, CatalogError> {
        match &location.format {
            LocationFormat::Lance { root } => Ok(Path::new(self.root_path()).join(root)),
            other => Err(CatalogError::KindFormatMismatch {
                kind: "lance".to_string(),
                format: other.name().to_string(),
            }),
        }
    }

    /// Absolute path of a SQLite Location's database file.
    pub fn sqlite_path(&self, location: &Location) -> Result<PathBuf, CatalogError> {
        match &location.format {
            LocationFormat::SqliteFile { path } => Ok(Path::new(self.root_path()).join(path)),
            other => Err(CatalogError::KindFormatMismatch {
                kind: "sqlite".to_string(),
                format: other.name().to_string(),
            }),
        }
    }
}
