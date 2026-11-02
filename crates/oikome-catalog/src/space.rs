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

//! Space (logical) and Location (physical) definitions plus the binding rules
//! of architecture v3 §3.2.

use oikome_core::{
    AccessMode, CatalogError, CreateSpaceRequest, EngineKind, SpaceFormat, SpaceKind, SpaceUri,
};
use serde::{Deserialize, Serialize};

use crate::storage::{validate_storage_path, StorageBinding};

/// Identifier of a [`Location`]; defaults to the name of the Space that created it.
pub type LocationId = String;

/// Residency token of a remote Location.
pub const RESIDENCY_REMOTE: &str = "remote";

/// Relative (to the catalog root) directory that holds SQLite Space files.
pub const SQLITE_DIR: &str = ".oikome/sqlite";

/// Relative (to the catalog root) directory that holds Lance Locations, one
/// directory of `<table>.lance` datasets each. It is under `.oikome/` so it
/// is never listed as an Iceberg namespace.
pub const LANCE_DIR: &str = ".oikome/lance";

/// `STORAGE` of a Space on this node's disk only (the default); any other
/// value names a registered storage (§7.5).
pub const STORAGE_DISK: &str = "disk";

/// A logical Space: the unit of sovereignty, authorization and engine binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    /// Node-local name; also the SQL qualifier (`<name>.<table>`).
    pub name: String,
    /// Global identity once the node has a DID (3.1).
    pub uri: Option<SpaceUri>,
    pub kind: SpaceKind,
    pub engine: EngineKind,
    /// The physical Location this Space is bound to.
    pub location: LocationId,
    pub access: AccessMode,
    /// Creation time, milliseconds since the Unix epoch.
    pub created_at_ms: u64,
    /// Set on an analytical Space written only by `PUBLISH SPACE <source>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_from: Option<String>,
}

/// The on-disk format of a Location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum LocationFormat {
    /// An Iceberg namespace (directory of tables) under the catalog root.
    IcebergParquet { namespace: String },
    /// A single SQLite database file, path relative to the catalog root.
    SqliteFile { path: String },
    /// A directory of Lance datasets (`<table>.lance`), path relative to the
    /// catalog root (§7.6).
    Lance { root: String },
    /// A Space on another node, served over Arrow Flight.
    Remote {
        /// Flight endpoint, `https://host:port`; `None` looks the owner's
        /// endpoint up among the discovered peers on every connection.
        #[serde(default)]
        peer: Option<String>,
        /// The Space's name on the peer.
        space: String,
        /// DID of the owning node (root issuer of the UCAN).
        owner: String,
    },
    /// A local copy of a Space on another node (subscription, §5.3): the
    /// owner's snapshot files under `.oikome/replicas/<location id>/`.
    Replica {
        /// Flight endpoint, as for [`LocationFormat::Remote`].
        #[serde(default)]
        peer: Option<String>,
        /// The Space's name on the peer.
        space: String,
        /// DID of the owning node (root issuer of the UCAN).
        owner: String,
    },
    /// A local copy of a Space another node writes on a storage (§7.5):
    /// the newest complete snapshot, under `.oikome/replicas/<location id>/`.
    /// The storage and folder are in [`Location::storage`].
    StorageCopy,
}

impl LocationFormat {
    /// Short name for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            LocationFormat::IcebergParquet { .. } => "iceberg_parquet",
            LocationFormat::SqliteFile { .. } => "sqlite_file",
            LocationFormat::Lance { .. } => "lance",
            LocationFormat::Remote { .. } => "remote",
            LocationFormat::Replica { .. } => "replica",
            LocationFormat::StorageCopy => "storage_copy",
        }
    }

    /// Whether the Location is a local copy of a Space written elsewhere (a
    /// subscription or a Space on a storage), kept under `replica_dir`.
    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            LocationFormat::Replica { .. } | LocationFormat::StorageCopy
        )
    }

    /// Whether the Location belongs to another node's Space (queried
    /// remotely or copied), so dropping its Space forgets it.
    pub fn is_peer_space(&self) -> bool {
        matches!(
            self,
            LocationFormat::Remote { .. } | LocationFormat::Replica { .. }
        )
    }
}

/// A physical Location. Several Spaces may bind to it, at most one as writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub id: LocationId,
    #[serde(flatten)]
    pub format: LocationFormat,
    /// Storage strategy tokens (`"disk"` only in 3.0).
    pub residency: Vec<String>,
    /// Name of the Space that owns the Location, if any.
    pub writer: Option<String>,
    /// The storage the Location is kept on besides this node's disk (§7.5):
    /// the writer pushes each snapshot there; a [`LocationFormat::StorageCopy`]
    /// pulls from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageBinding>,
}

/// Whether `engine` can read `format` at all.
pub fn engine_reads(engine: EngineKind, format: &LocationFormat) -> bool {
    match (engine, format) {
        (EngineKind::Flight, LocationFormat::Remote { .. }) => true,
        (_, LocationFormat::Remote { .. }) | (EngineKind::Flight, _) => false,
        // Host and WebRTC engines live in the browser node, which has its
        // own registry.
        (EngineKind::Host | EngineKind::WebRtc, _) => false,
        (EngineKind::DuckDb, _) => true,
        (EngineKind::Sqlite, LocationFormat::SqliteFile { .. }) => true,
        (
            EngineKind::Sqlite,
            LocationFormat::IcebergParquet { .. }
            | LocationFormat::Lance { .. }
            | LocationFormat::Replica { .. }
            | LocationFormat::StorageCopy,
        ) => false,
    }
}

/// Apply the defaults and binding rules to a `CREATE SPACE` request.
///
/// `existing` is the Location named by `request.location`, if the request
/// names one. Returns the Space to register together with the Location as it
/// must be stored afterwards (new, or updated with the new writer).
pub fn validate_binding(
    request: &CreateSpaceRequest,
    existing: Option<&Location>,
    now_ms: u64,
) -> Result<(Space, Location), CatalogError> {
    if request.name.is_empty() {
        return Err(CatalogError::InvalidProperty(
            "space name must not be empty".to_string(),
        ));
    }
    if request.location.is_some() && existing.is_none() {
        return Err(CatalogError::LocationNotFound(
            request.location.clone().unwrap_or_default(),
        ));
    }

    let kind = request.kind.unwrap_or(SpaceKind::Analytical);
    if kind == SpaceKind::Remote || request.peer.is_some() || request.token.is_some() {
        return Err(CatalogError::InvalidProperty(
            "a remote space needs PEER and TOKEN and is created with create_remote_space"
                .to_string(),
        ));
    }
    let engine = request.engine.unwrap_or_else(|| kind.default_engine());
    if request.format.is_some() && (kind != SpaceKind::Analytical || existing.is_some()) {
        return Err(CatalogError::InvalidProperty(
            "FORMAT applies to a new analytical space only".to_string(),
        ));
    }
    let storage = storage_binding(request, kind)?;
    let copy = storage.is_some() && request.access == Some(AccessMode::ReadOnly);
    if copy && request.format.is_some() {
        return Err(CatalogError::InvalidProperty(
            "a read-only space on a storage takes its FORMAT from the storage".to_string(),
        ));
    }

    let mut location = match existing {
        Some(loc) => loc.clone(),
        None if copy => Location {
            id: request.name.clone(),
            format: LocationFormat::StorageCopy,
            residency: vec![STORAGE_DISK.to_string()],
            writer: None,
            storage,
        },
        None => Location {
            id: request.name.clone(),
            format: match kind {
                SpaceKind::Analytical => match request.format.unwrap_or_default() {
                    SpaceFormat::Iceberg => LocationFormat::IcebergParquet {
                        namespace: request.name.clone(),
                    },
                    SpaceFormat::Lance => LocationFormat::Lance {
                        root: format!("{LANCE_DIR}/{}", request.name),
                    },
                },
                SpaceKind::Transactional | SpaceKind::Remote => LocationFormat::SqliteFile {
                    path: format!("{SQLITE_DIR}/{}.sqlite", request.name),
                },
            },
            residency: vec![STORAGE_DISK.to_string()],
            writer: None,
            storage,
        },
    };

    let access = match (kind, &location.format) {
        (SpaceKind::Analytical, LocationFormat::StorageCopy) => {
            if request.access == Some(AccessMode::Writable) {
                return Err(CatalogError::InvalidProperty(format!(
                    "location '{}' is a copy of a space written on a storage and is read-only",
                    location.id
                )));
            }
            AccessMode::ReadOnly
        }
        (
            _,
            LocationFormat::Remote { .. }
            | LocationFormat::Replica { .. }
            | LocationFormat::StorageCopy,
        )
        | (SpaceKind::Remote, _) => {
            return Err(CatalogError::KindFormatMismatch {
                kind: kind.to_string(),
                format: location.format.name().to_string(),
            })
        }
        (
            SpaceKind::Transactional,
            LocationFormat::IcebergParquet { .. } | LocationFormat::Lance { .. },
        ) => {
            return Err(CatalogError::KindFormatMismatch {
                kind: kind.to_string(),
                format: location.format.name().to_string(),
            })
        }
        (SpaceKind::Transactional, LocationFormat::SqliteFile { .. }) => {
            if request.access == Some(AccessMode::ReadOnly) {
                return Err(CatalogError::InvalidProperty(
                    "transactional spaces are always writable".to_string(),
                ));
            }
            AccessMode::Writable
        }
        (SpaceKind::Analytical, LocationFormat::SqliteFile { .. }) => {
            // An analytical view over a transactional file never writes to it.
            if request.access == Some(AccessMode::Writable) {
                return Err(CatalogError::KindFormatMismatch {
                    kind: kind.to_string(),
                    format: location.format.name().to_string(),
                });
            }
            AccessMode::ReadOnly
        }
        (
            SpaceKind::Analytical,
            LocationFormat::IcebergParquet { .. } | LocationFormat::Lance { .. },
        ) => request.access.unwrap_or(if existing.is_some() {
            AccessMode::ReadOnly
        } else {
            AccessMode::Writable
        }),
    };

    if !engine_reads(engine, &location.format) {
        return Err(CatalogError::IncompatibleFormat {
            engine: engine.to_string(),
            format: location.format.name().to_string(),
        });
    }

    if access == AccessMode::Writable {
        if let Some(writer) = &location.writer {
            return Err(CatalogError::LocationHasWriter {
                location: location.id.clone(),
                writer: writer.clone(),
            });
        }
        location.writer = Some(request.name.clone());
    }

    let space = Space {
        name: request.name.clone(),
        uri: None,
        kind,
        engine,
        location: location.id.clone(),
        access,
        created_at_ms: now_ms,
        published_from: None,
    };
    Ok((space, location))
}

/// Where a `CREATE SPACE` request puts the Space on a storage, if it names
/// one (`STORAGE` other than `'disk'`). Whether the storage is registered
/// is checked by the registry.
fn storage_binding(
    request: &CreateSpaceRequest,
    kind: SpaceKind,
) -> Result<Option<StorageBinding>, CatalogError> {
    let Some(storage) = request.storage.as_deref().filter(|s| *s != STORAGE_DISK) else {
        if request.path.is_some() {
            return Err(CatalogError::InvalidProperty(
                "PATH applies to a space on a storage (STORAGE = '<name>')".to_string(),
            ));
        }
        return Ok(None);
    };
    if kind != SpaceKind::Analytical {
        return Err(CatalogError::InvalidProperty(format!(
            "a {kind} space cannot live on storage '{storage}': SQLite needs this node's disk; \
             create it without STORAGE"
        )));
    }
    if request.location.is_some() {
        return Err(CatalogError::InvalidProperty(
            "LOCATION and STORAGE cannot be combined".to_string(),
        ));
    }
    let path = request.path.clone().unwrap_or_else(|| request.name.clone());
    validate_storage_path(&path)?;
    Ok(Some(StorageBinding {
        storage: storage.to_string(),
        path,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str) -> CreateSpaceRequest {
        CreateSpaceRequest::new(name)
    }

    fn parquet_location(id: &str, writer: Option<&str>) -> Location {
        Location {
            id: id.to_string(),
            format: LocationFormat::IcebergParquet {
                namespace: id.to_string(),
            },
            residency: vec![STORAGE_DISK.to_string()],
            writer: writer.map(str::to_string),
            storage: None,
        }
    }

    fn sqlite_location(id: &str, writer: Option<&str>) -> Location {
        Location {
            id: id.to_string(),
            format: LocationFormat::SqliteFile {
                path: format!("{SQLITE_DIR}/{id}.sqlite"),
            },
            residency: vec![STORAGE_DISK.to_string()],
            writer: writer.map(str::to_string),
            storage: None,
        }
    }

    #[test]
    fn defaults_from_kind() {
        let (space, loc) = validate_binding(&req("blog"), None, 7).unwrap();
        assert_eq!(space.kind, SpaceKind::Analytical);
        assert_eq!(space.engine, EngineKind::DuckDb);
        assert_eq!(space.access, AccessMode::Writable);
        assert_eq!(space.location, "blog");
        assert_eq!(space.created_at_ms, 7);
        assert_eq!(loc, parquet_location("blog", Some("blog")));

        let mut r = req("orders");
        r.kind = Some(SpaceKind::Transactional);
        let (space, loc) = validate_binding(&r, None, 0).unwrap();
        assert_eq!(space.engine, EngineKind::Sqlite);
        assert_eq!(space.access, AccessMode::Writable);
        assert_eq!(loc, sqlite_location("orders", Some("orders")));
    }

    #[test]
    fn transactional_requires_sqlite_file() {
        let mut r = req("orders");
        r.kind = Some(SpaceKind::Transactional);
        r.location = Some("blog".to_string());
        let err = validate_binding(&r, Some(&parquet_location("blog", None)), 0).unwrap_err();
        assert!(matches!(err, CatalogError::KindFormatMismatch { .. }));

        r.access = Some(AccessMode::ReadOnly);
        r.location = None;
        assert!(matches!(
            validate_binding(&r, None, 0),
            Err(CatalogError::InvalidProperty(_))
        ));
    }

    #[test]
    fn analytical_over_sqlite_must_be_readonly() {
        let mut r = req("orders_ro");
        r.location = Some("orders".to_string());
        let loc = sqlite_location("orders", Some("orders"));

        let (space, updated) = validate_binding(&r, Some(&loc), 0).unwrap();
        assert_eq!(space.access, AccessMode::ReadOnly);
        assert_eq!(space.engine, EngineKind::DuckDb);
        assert_eq!(
            updated.writer.as_deref(),
            Some("orders"),
            "writer unchanged"
        );

        r.access = Some(AccessMode::Writable);
        assert!(matches!(
            validate_binding(&r, Some(&loc), 0),
            Err(CatalogError::KindFormatMismatch { .. })
        ));
    }

    #[test]
    fn second_writer_rejected_and_readonly_second_binding_allowed() {
        let owned = parquet_location("blog", Some("blog"));

        let mut r = req("blog_rw");
        r.location = Some("blog".to_string());
        r.access = Some(AccessMode::Writable);
        assert!(matches!(
            validate_binding(&r, Some(&owned), 0),
            Err(CatalogError::LocationHasWriter { ref writer, .. }) if writer == "blog"
        ));

        r.access = None; // defaults to readonly when binding an existing location
        let (space, loc) = validate_binding(&r, Some(&owned), 0).unwrap();
        assert_eq!(space.access, AccessMode::ReadOnly);
        assert_eq!(loc.writer.as_deref(), Some("blog"));

        // An unowned existing location can be claimed explicitly.
        let free = parquet_location("archive", None);
        let mut r = req("arch");
        r.location = Some("archive".to_string());
        r.access = Some(AccessMode::Writable);
        let (_, loc) = validate_binding(&r, Some(&free), 0).unwrap();
        assert_eq!(loc.writer.as_deref(), Some("arch"));
    }

    #[test]
    fn engine_format_matrix() {
        assert!(engine_reads(
            EngineKind::DuckDb,
            &parquet_location("a", None).format
        ));
        assert!(engine_reads(
            EngineKind::DuckDb,
            &sqlite_location("a", None).format
        ));
        assert!(engine_reads(
            EngineKind::Sqlite,
            &sqlite_location("a", None).format
        ));
        assert!(!engine_reads(
            EngineKind::Sqlite,
            &parquet_location("a", None).format
        ));

        let mut r = req("x");
        r.engine = Some(EngineKind::Sqlite);
        assert!(matches!(
            validate_binding(&r, None, 0),
            Err(CatalogError::IncompatibleFormat { .. })
        ));
    }

    #[test]
    fn spaces_on_a_storage_and_missing_location_errors() {
        // The writer keeps its tables on disk and pushes them to the storage.
        let mut r = req("photos");
        r.storage = Some("nas".to_string());
        let (space, loc) = validate_binding(&r, None, 0).unwrap();
        assert_eq!(space.access, AccessMode::Writable);
        assert_eq!(loc.format, parquet_location("photos", None).format);
        let binding = loc.storage.clone().unwrap();
        assert_eq!(
            (binding.storage.as_str(), binding.path.as_str()),
            ("nas", "photos")
        );

        // Another node copies it, read-only.
        r.access = Some(AccessMode::ReadOnly);
        r.path = Some("family/photos".to_string());
        let (space, loc) = validate_binding(&r, None, 0).unwrap();
        assert_eq!(space.access, AccessMode::ReadOnly);
        assert_eq!(loc.format, LocationFormat::StorageCopy);
        assert_eq!(loc.writer, None);
        assert_eq!(loc.storage.unwrap().path, "family/photos");

        // Binding the copy again stays read-only.
        let copy = validate_binding(&r, None, 0).unwrap().1;
        let mut again = req("photos2");
        again.location = Some("photos".to_string());
        again.access = Some(AccessMode::Writable);
        assert!(validate_binding(&again, Some(&copy), 0).is_err());

        for (field, value) in [
            ("kind", "transactional"),
            ("path", "../x"),
            ("format", "lance"),
        ] {
            let mut bad = req("x");
            bad.storage = Some("nas".to_string());
            bad.access = Some(AccessMode::ReadOnly);
            match field {
                "kind" => bad.kind = Some(SpaceKind::Transactional),
                "path" => bad.path = Some(value.to_string()),
                _ => bad.format = Some(SpaceFormat::Lance),
            }
            assert!(
                matches!(
                    validate_binding(&bad, None, 0),
                    Err(CatalogError::InvalidProperty(_))
                ),
                "{field}"
            );
        }
        let mut bad = req("x");
        bad.path = Some("p".to_string());
        assert!(
            validate_binding(&bad, None, 0).is_err(),
            "PATH without STORAGE"
        );

        let mut r = req("x");
        r.location = Some("ghost".to_string());
        assert!(matches!(
            validate_binding(&r, None, 0),
            Err(CatalogError::LocationNotFound(l)) if l == "ghost"
        ));
    }

    #[test]
    fn remote_locations_only_bind_through_flight() {
        let remote = Location {
            id: "pub".to_string(),
            format: LocationFormat::Remote {
                peer: Some("https://a:50051".to_string()),
                space: "blog".to_string(),
                owner: "did:key:z6Mk".to_string(),
            },
            residency: vec![RESIDENCY_REMOTE.to_string()],
            writer: None,
            storage: None,
        };
        assert!(engine_reads(EngineKind::Flight, &remote.format));
        assert!(!engine_reads(EngineKind::DuckDb, &remote.format));
        assert!(!engine_reads(
            EngineKind::Flight,
            &parquet_location("a", None).format
        ));

        let mut r = req("pub_ro");
        r.location = Some("pub".to_string());
        assert!(matches!(
            validate_binding(&r, Some(&remote), 0),
            Err(CatalogError::KindFormatMismatch { .. })
        ));
        let mut r = req("pub");
        r.kind = Some(SpaceKind::Remote);
        assert!(matches!(
            validate_binding(&r, None, 0),
            Err(CatalogError::InvalidProperty(_))
        ));
    }

    #[test]
    fn replica_locations_are_read_by_duckdb_and_never_rebound() {
        let replica = Location {
            id: "copy".to_string(),
            format: LocationFormat::Replica {
                peer: Some("https://a:50051".to_string()),
                space: "blog".to_string(),
                owner: "did:key:z6Mk".to_string(),
            },
            residency: vec![RESIDENCY_REMOTE.to_string()],
            writer: None,
            storage: None,
        };
        assert!(replica.format.is_peer_space());
        assert!(!parquet_location("a", None).format.is_peer_space());
        assert!(engine_reads(EngineKind::DuckDb, &replica.format));
        assert!(!engine_reads(EngineKind::Sqlite, &replica.format));
        assert!(!engine_reads(EngineKind::Flight, &replica.format));

        let mut r = req("copy_ro");
        r.location = Some("copy".to_string());
        assert!(matches!(
            validate_binding(&r, Some(&replica), 0),
            Err(CatalogError::KindFormatMismatch { .. })
        ));
    }

    #[test]
    fn location_serde_is_tagged_by_format() {
        let json = serde_json::to_string(&sqlite_location("o", Some("o"))).unwrap();
        assert!(json.contains("\"format\":\"sqlite_file\""));
        let back: Location = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sqlite_location("o", Some("o")));
    }
}
