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

//! The owner API on a local node (M1b): Spaces, rows, SQL, contacts and
//! shares, with the same data a real node keeps.

use std::sync::Arc;

use arrow_ipc::reader::StreamReader;
use iceberg::io::FileIO;
use oikome_api::*;
use oikome_catalog::OikomeCatalog;
use oikome_identity::NodeIdentity;
use oikome_node::{EngineRegistry, NodeConfig};

struct Node {
    _dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    api: Api,
}

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), NodeConfig::default()).unwrap());
    let api = Api::new(catalog.clone(), engines);
    Node {
        _dir: dir,
        catalog,
        api,
    }
}

/// The DID of another node, made from a fixed seed.
fn did(seed: u8) -> String {
    NodeIdentity::from_seed(&[seed; 32]).did().to_string()
}

fn row_count(rows: &Rows) -> usize {
    StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum()
}

const OWNER: Caller = Caller::Owner;

async fn sql(n: &Node, text: &str) -> SqlResult {
    n.api
        .run_sql(
            &OWNER,
            RunSql {
                sql: text.to_string(),
                space: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

async fn notes(n: &Node) {
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "notes".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    sql(
        n,
        "CREATE TABLE notes.memos (id BIGINT NOT NULL, body VARCHAR); \
         INSERT INTO notes.memos VALUES (1, 'a'), (2, 'b'), (3, 'c')",
    )
    .await;
}

async fn share(n: &Node, contact: &str, days: Option<u32>) -> NewShare {
    n.api
        .share_space(
            &OWNER,
            ShareSpace {
                space: "notes".into(),
                did: contact.to_string(),
                allow_copy: None,
                expires_in_days: days,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn spaces_rows_and_sql() {
    let n = node();
    notes(&n).await;
    let spaces = n.api.list_spaces(&OWNER, Empty {}).await.unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].name, "notes");
    assert_eq!(spaces[0].kind, SpaceKind::Analytical);
    assert_eq!(spaces[0].format, "iceberg");

    let tables = n
        .api
        .list_tables(
            &OWNER,
            SpaceRef {
                space: "notes".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        tables,
        vec![TableSummary {
            name: "memos".into(),
            kind: RelationKind::Table,
            rows: Some(3),
        }]
    );

    let page = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "notes".into(),
                table: "memos".into(),
                limit: Some(2),
                offset: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&page), 2);

    let missing = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "notes".into(),
                table: "nope".into(),
                limit: None,
                offset: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);

    match sql(&n, "SELECT count(*) FROM notes.memos").await {
        SqlResult::Rows(rows) => assert_eq!(row_count(&rows), 1),
        other => panic!("expected rows, got {other:?}"),
    }
    match sql(&n, "INSERT INTO notes.memos VALUES (4, 'd')").await {
        SqlResult::Done(done) => assert_eq!(done.rows_affected, Some(1)),
        other => panic!("expected no rows, got {other:?}"),
    }

    let long = n
        .api
        .run_sql(
            &OWNER,
            RunSql {
                sql: "x".repeat(MAX_SQL_BYTES + 1),
                space: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(long.code, ErrorCode::Invalid);
}

#[tokio::test]
async fn bad_names_are_refused() {
    let n = node();
    for name in ["../escape", "a/b", "", "1st"] {
        let e = n
            .api
            .create_space(
                &OWNER,
                NewSpace {
                    name: name.into(),
                    kind: None,
                    storage: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid, "{name}");
    }
}

#[tokio::test]
async fn sharing_needs_a_contact_and_revoking_the_last_share_removes_the_grant() {
    let n = node();
    notes(&n).await;
    let bob = did(2);

    let e = n
        .api
        .share_space(
            &OWNER,
            ShareSpace {
                space: "notes".into(),
                did: bob.clone(),
                allow_copy: None,
                expires_in_days: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    n.api
        .add_contact(
            &OWNER,
            ContactInput {
                did: bob.clone(),
                name: "Bob".into(),
            },
        )
        .await
        .unwrap();
    let again = n
        .api
        .add_contact(
            &OWNER,
            ContactInput {
                did: bob.clone(),
                name: "Bob".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::Conflict);

    for days in [Some(0), Some(367)] {
        let e = n
            .api
            .share_space(
                &OWNER,
                ShareSpace {
                    space: "notes".into(),
                    did: bob.clone(),
                    allow_copy: None,
                    expires_in_days: days,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid);
    }

    let first = share(&n, &bob, None).await;
    let second = share(&n, &bob, Some(7)).await;
    assert_eq!(first.access, ShareAccess::Query);
    assert!(n.catalog.has_space_grant(&bob, "notes"));
    let spaces = n.api.list_spaces(&OWNER, Empty {}).await.unwrap();
    assert_eq!(spaces[0].shared_with, 1);

    let shares = n
        .api
        .list_shares(&OWNER, ListShares::default())
        .await
        .unwrap();
    assert_eq!(shares.len(), 2);
    assert!(shares.iter().all(|s| s.status == ShareStatus::Active));
    assert_eq!(shares[0].contact_name.as_deref(), Some("Bob"));

    let revoked = n
        .api
        .revoke_share(
            &OWNER,
            ShareRef {
                id: first.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(!revoked.grant_removed, "another share is still active");
    assert!(n.catalog.has_space_grant(&bob, "notes"));

    let revoked = n
        .api
        .revoke_share(
            &OWNER,
            ShareRef {
                id: second.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(revoked.grant_removed);
    assert!(!n.catalog.has_space_grant(&bob, "notes"));
    let shares = n
        .api
        .list_shares(&OWNER, ListShares::default())
        .await
        .unwrap();
    assert!(shares.iter().all(|s| s.status == ShareStatus::Revoked));
}

#[tokio::test]
async fn copies_are_for_analytical_spaces() {
    let n = node();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "orders".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    let bob = did(2);
    n.api
        .add_contact(
            &OWNER,
            ContactInput {
                did: bob.clone(),
                name: "Bob".into(),
            },
        )
        .await
        .unwrap();
    let e = n
        .api
        .share_space(
            &OWNER,
            ShareSpace {
                space: "orders".into(),
                did: bob,
                allow_copy: Some(true),
                expires_in_days: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
}

#[tokio::test]
async fn dropping_a_space_or_a_contact_revokes_their_shares() {
    let n = node();
    notes(&n).await;
    let bob = did(2);
    let carol = did(3);
    for (d, name) in [(&bob, "Bob"), (&carol, "Carol")] {
        n.api
            .add_contact(
                &OWNER,
                ContactInput {
                    did: d.clone(),
                    name: name.into(),
                },
            )
            .await
            .unwrap();
    }
    let for_bob = share(&n, &bob, None).await;
    let removed = n
        .api
        .remove_contact(&OWNER, DidRef { did: bob.clone() })
        .await
        .unwrap();
    assert_eq!(removed.revoked_shares, 1);
    assert!(n.catalog.revoked_ucans().contains(&for_bob.id));
    assert!(!n.catalog.has_space_grant(&bob, "notes"));

    let for_carol = share(&n, &carol, None).await;
    n.api
        .drop_space(
            &OWNER,
            SpaceRef {
                space: "notes".into(),
            },
        )
        .await
        .unwrap();
    assert!(n.catalog.revoked_ucans().contains(&for_carol.id));
    assert!(n
        .api
        .list_spaces(&OWNER, Empty {})
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn principals_cannot_use_owner_operations() {
    let n = node();
    notes(&n).await;
    let principal = Caller::Principal { did: did(9) };
    let e = n.api.list_spaces(&principal, Empty {}).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    let e = n
        .api
        .run_sql(
            &principal,
            RunSql {
                sql: "SELECT 1".into(),
                space: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    let e = n
        .api
        .add_contact(
            &principal,
            ContactInput {
                did: did(2),
                name: "x".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    // Who the node is, anyone may ask.
    let info = n.api.node_info(&principal, Empty {}).await.unwrap();
    assert!(info.did.starts_with("did:key:z"));
}

#[tokio::test]
async fn naming_the_node_and_backing_up_its_recovery_key() {
    let n = node();
    let info = n.api.node_info(&OWNER, Empty {}).await.unwrap();
    assert_eq!(info.name, "");
    assert!(!info.recovery_key_saved);
    assert!(!info.data_dir.is_empty());

    let info = n
        .api
        .rename_node(
            &OWNER,
            NodeName {
                name: "  Home ".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(info.name, "Home");
    let e = n
        .api
        .rename_node(&OWNER, NodeName { name: " ".into() })
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // The key decodes to this node's identity.
    let key = n.api.recovery_key(&OWNER, Empty {}).await.unwrap();
    assert_eq!(key.key.split('-').count(), 14);
    let seed = oikome_identity::decode_recovery_key(&key.key).unwrap();
    assert_eq!(NodeIdentity::from_seed(&seed).did().to_string(), info.did);
    let info = n.api.confirm_recovery_key(&OWNER, Empty {}).await.unwrap();
    assert!(info.recovery_key_saved);

    let principal = Caller::Principal { did: did(9) };
    let e = n.api.recovery_key(&principal, Empty {}).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    let seen = n.api.node_info(&principal, Empty {}).await.unwrap();
    assert!(seen.data_dir.is_empty() && seen.name.is_empty());
}

#[tokio::test]
async fn an_identity_is_restored_only_into_an_empty_node() {
    let old = node();
    let key = old.api.recovery_key(&OWNER, Empty {}).await.unwrap();
    let old_did = old.api.node_info(&OWNER, Empty {}).await.unwrap().did;

    let fresh = node();
    let restored = restore_identity(&fresh.catalog, &key.key).unwrap();
    assert_eq!(restored, old_did);
    let key_file = fresh
        .catalog
        .oikome_dir()
        .join("identity")
        .join("node_ed25519");
    assert!(key_file.is_file());

    let e = restore_identity(&fresh.catalog, "not a key").unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    notes(&old).await;
    let e = restore_identity(&old.catalog, &key.key).unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
}

#[tokio::test]
async fn a_new_data_center_gets_examples_once() {
    let n = node();
    let added = n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    let names: Vec<&str> = added.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "home-climate",
            "lisbon-notes",
            "lisbon-photos",
            "lisbon-spending",
            "oikome-guide",
            "reading-list"
        ]
    );
    assert!(added.iter().all(|s| s.example && s.storage.is_none()));

    // Photos are images, kept whole.
    let rows = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "lisbon-photos".into(),
                table: "photos".into(),
                limit: None,
                offset: None,
            },
        )
        .await
        .unwrap();
    let batch = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batch.num_rows(), 6);
    let photo = batch
        .column_by_name("photo")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .unwrap();
    assert!(photo.value(0).starts_with(&[0xff, 0xd8, 0xff]), "a JPEG");
    let tables = n
        .api
        .list_tables(
            &OWNER,
            SpaceRef {
                space: "lisbon-photos".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].rows, Some(6));
    for (space, table, count) in [
        ("lisbon-notes", "notes", 7),
        ("lisbon-spending", "spending", 10),
        ("oikome-guide", "guide", 7),
        ("reading-list", "books", 10),
        ("home-climate", "readings", 14 * 24),
    ] {
        let rows = n
            .api
            .read_rows(
                &OWNER,
                ReadRows {
                    space: space.into(),
                    table: table.into(),
                    limit: Some(1000),
                    offset: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(row_count(&rows), count, "{space}");
    }

    // Removing one is for good, and the rest stay examples.
    n.api
        .drop_space(
            &OWNER,
            SpaceRef {
                space: "lisbon-notes".into(),
            },
        )
        .await
        .unwrap();
    let again = n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    assert_eq!(again.len(), 5);
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(storages.len(), 1);
    assert_eq!(storages[0].name, None);
    assert_eq!(storages[0].kind, StorageType::Disk);
    assert_eq!(storages[0].spaces, 5);
    if cfg!(unix) {
        assert!(storages[0].total_bytes.is_some_and(|t| t > 0));
    }

    // Its files stay, and keep the name.
    let err = n
        .api
        .create_space(
            &OWNER,
            NewSpace {
                name: "lisbon-notes".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err}");

    // An unknown storage is refused before anything is made.
    let err = n
        .api
        .create_space(
            &OWNER,
            NewSpace {
                name: "elsewhere".into(),
                kind: None,
                storage: Some("nas".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err}");
}

#[tokio::test]
async fn a_folder_the_owner_picks_keeps_new_collections() {
    let n = node();
    let picked = tempfile::tempdir().unwrap();
    let added = n
        .api
        .add_storage(
            &OWNER,
            NewStorage {
                label: "Documents".into(),
                kind: StorageType::Folder,
                address: picked.path().to_str().unwrap().into(),
                username: None,
                password: None,
                encrypted: None,
                make_default: Some(true),
            },
        )
        .await
        .unwrap();
    assert_eq!(added.name.as_deref(), Some("documents"));
    assert_eq!(added.label, "Documents");
    assert!(added.is_default);
    assert!(added.place.ends_with("/Oikome"));
    assert!(picked.path().join("Oikome").is_dir());

    // The examples go where new collections go.
    let examples = n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    assert!(examples
        .iter()
        .all(|s| s.storage.as_deref() == Some("documents")));
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(storages[0].spaces, 0);
    assert!(!storages[0].is_default);
    assert_eq!(storages[1].spaces, 6);

    // Back to this computer.
    let storages = n
        .api
        .set_default_storage(&OWNER, DefaultStorage { storage: None })
        .await
        .unwrap();
    assert!(storages[0].is_default && !storages[1].is_default);

    // A folder that is not there, or not a path, is refused.
    let err = n
        .api
        .add_storage(
            &OWNER,
            NewStorage {
                label: "Nowhere".into(),
                kind: StorageType::Folder,
                address: "relative/folder".into(),
                username: None,
                password: None,
                encrypted: None,
                make_default: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Invalid, "{err}");

    // The folder browser lists folders, not files.
    std::fs::create_dir(picked.path().join("Photos")).unwrap();
    let listing = n
        .api
        .list_folders(
            &OWNER,
            FolderRef {
                path: Some(picked.path().to_str().unwrap().into()),
            },
        )
        .await
        .unwrap();
    let names: Vec<&str> = listing.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Oikome", "Photos"]);
}
