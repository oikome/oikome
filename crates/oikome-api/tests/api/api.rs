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
    engines: Arc<EngineRegistry>,
    api: Api,
}

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), NodeConfig::default()).unwrap());
    let api = Api::new(catalog.clone(), engines.clone());
    Node {
        _dir: dir,
        catalog,
        engines,
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
            key: vec![],
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
    let e = n
        .api
        .add_invite(
            &principal,
            InviteCode {
                invite: "x.y.z".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    // Who the node is, anyone may ask.
    let info = n.api.node_info(&principal, Empty {}).await.unwrap();
    assert!(info.did.starts_with("did:key:z"));
}

async fn share_for(n: &Api, did: &str, days: u32) -> NewShare {
    n.share_space(
        &OWNER,
        ShareSpace {
            space: "notes".into(),
            did: did.to_string(),
            allow_copy: None,
            expires_in_days: Some(days),
        },
    )
    .await
    .unwrap()
}

async fn add(n: &Node, invite: &str) -> Result<Invite, ApiError> {
    n.api
        .add_invite(
            &OWNER,
            InviteCode {
                invite: invite.to_string(),
            },
        )
        .await
}

#[tokio::test]
async fn an_invite_is_added_then_accepted_renewed_or_declined() {
    // Rosh's data center, reached at an address of its own, shares notes with Sam.
    let rosh = node();
    let rosh_api = rosh.api.clone().with_flight_url("https://127.0.0.1:9");
    rosh_api
        .rename_node(
            &OWNER,
            NodeName {
                name: "Rosh's data center".into(),
            },
        )
        .await
        .unwrap();
    notes(&rosh).await;
    let sam = node();
    let sam_did = sam.api.node_info(&OWNER, Empty {}).await.unwrap().did;
    rosh_api
        .add_contact(
            &OWNER,
            ContactInput {
                did: sam_did.clone(),
                name: "Sam".into(),
            },
        )
        .await
        .unwrap();
    let made = share_for(&rosh_api, &sam_did, 30).await;
    assert!(made.invite.starts_with("oik1-"));

    // It is not for the one who made it, nor for anyone but Sam, and it
    // has to be whole.
    let e = add(&rosh, &made.invite).await.unwrap_err();
    assert!(e.message.contains("one you made"), "{e}");
    let other = node();
    let e = add(&other, &made.invite).await.unwrap_err();
    assert!(e.message.contains("another data center"), "{e}");
    let e = add(&sam, &made.invite[..60]).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // Sam adds it as a message wrapped it; the same code twice is one invite.
    let wrapped = format!("{}\n  {}", &made.invite[..40], &made.invite[40..]);
    let invite = add(&sam, &wrapped).await.unwrap();
    assert_eq!(invite.from_name, "Rosh's data center");
    assert!(!invite.from_contact);
    assert_eq!(invite.space, "notes");
    assert_eq!(invite.access, ShareAccess::Query);
    assert_eq!(invite.expires_at, made.expires_at);
    assert_eq!(invite.name, "notes");
    assert!(!invite.renews && !invite.ended);
    add(&sam, &made.invite).await.unwrap();
    assert_eq!(
        sam.api.list_invites(&OWNER, Empty {}).await.unwrap().len(),
        1
    );

    // Accepted: notes is here, from Rosh, who is a contact now.
    let added = sam
        .api
        .accept_invite(
            &OWNER,
            AcceptInvite {
                id: invite.id.clone(),
                name: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(added.kind, InviteKind::Collection);
    assert_eq!(added.name, "notes");
    let space_of = |name: &str, spaces: Vec<SpaceSummary>| {
        spaces.into_iter().find(|s| s.name == name).unwrap()
    };
    let notes = space_of(
        "notes",
        sam.api.list_spaces(&OWNER, Empty {}).await.unwrap(),
    );
    assert_eq!(notes.kind, SpaceKind::Remote);
    assert_eq!(notes.from_did.as_deref(), Some(invite.from_did.as_str()));
    assert_eq!(notes.from_name.as_deref(), Some("Rosh's data center"));
    assert_eq!(notes.expires_at, Some(made.expires_at));
    assert!(sam
        .api
        .list_invites(&OWNER, Empty {})
        .await
        .unwrap()
        .is_empty());
    let e = add(&sam, &made.invite).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);

    // A new share of it renews the one here instead of adding another.
    let again = share_for(&rosh_api, &sam_did, 60).await;
    let renewal = add(&sam, &again.invite).await.unwrap();
    assert!(renewal.renews && renewal.from_contact);
    assert_eq!(renewal.name, "notes");
    let renewed = sam
        .api
        .accept_invite(
            &OWNER,
            AcceptInvite {
                id: renewal.id,
                name: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(renewed.name, "notes");
    let notes = space_of(
        "notes",
        sam.api.list_spaces(&OWNER, Empty {}).await.unwrap(),
    );
    assert_eq!(notes.expires_at, Some(again.expires_at));
    let remote: Vec<_> = sam
        .api
        .list_spaces(&OWNER, Empty {})
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.kind == SpaceKind::Remote)
        .collect();
    assert_eq!(remote.len(), 1);

    // Declined: forgotten here.
    let third = share_for(&rosh_api, &sam_did, 7).await;
    let waiting = add(&sam, &third.invite).await.unwrap();
    let left = sam
        .api
        .decline_invite(&OWNER, InviteRef { id: waiting.id })
        .await
        .unwrap();
    assert!(left.is_empty());

    // Invites hold tokens: only the owner may read the file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = sam
            .catalog
            .oikome_dir()
            .join("identity")
            .join("invites.json");
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
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

    // Home's cards, as the owner arranged them; ids are checked, not read.
    assert_eq!(info.home_cards, None);
    let card = |id: &str, width| HomeCard {
        id: id.into(),
        width: Some(width),
    };
    let cards = vec![card("recent", 8), card("totals:spending", 4)];
    let info = n
        .api
        .set_home_cards(
            &OWNER,
            HomeCards {
                cards: cards.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(info.home_cards, Some(cards));
    let e = n
        .api
        .set_home_cards(
            &OWNER,
            HomeCards {
                cards: vec![card("<script>", 4)],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let e = n
        .api
        .set_home_cards(
            &OWNER,
            HomeCards {
                cards: vec![card("recent", 13)],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // Home kept before cards had widths still reads: each card takes the
    // app's own width.
    let path = n.catalog.oikome_dir().join("node.json");
    let mut kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    kept["home_cards"] = serde_json::json!(["photo", "recent"]);
    std::fs::write(&path, serde_json::to_vec(&kept).unwrap()).unwrap();
    let info = n.api.node_info(&OWNER, Empty {}).await.unwrap();
    let unsized_card = |id: &str| HomeCard {
        id: id.into(),
        width: None,
    };
    assert_eq!(
        info.home_cards,
        Some(vec![unsized_card("photo"), unsized_card("recent")])
    );

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
        ["home-climate", "lisbon-photos", "oikome-guide", "spending"]
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
        ("spending", "spending", 29),
        ("oikome-guide", "notes", 7),
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

    // They are files, read by sources: edit one and read it again.
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let spending = root.join("Examples/spending/spending.csv");
    assert!(root.join("Examples/lisbon-photos/captions.csv").is_file());
    assert!(root.join("Examples/oikome-guide/1 Welcome.md").is_file());
    let sources = n.api.list_sources(&OWNER, Empty {}).await.unwrap();
    assert_eq!(sources.len(), 4);
    let photos = sources.iter().find(|s| s.name == "lisbon-photos").unwrap();
    assert_eq!(photos.kind, SourceKind::Photos);
    assert!(photos.pulled_at_ms.is_some() && photos.failure.is_none());
    let mut text = std::fs::read_to_string(&spending).unwrap();
    text.push_str("Coffee at the airport,2025-11-09,Food,2.50\n");
    std::fs::write(&spending, text).unwrap();
    n.api
        .pull_source(
            &OWNER,
            SourceRef {
                source: "spending".into(),
            },
        )
        .await
        .unwrap();
    let tables = n
        .api
        .list_tables(
            &OWNER,
            SpaceRef {
                space: "spending".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(tables.len(), 1, "{tables:?}");
    assert_eq!(tables[0].rows, Some(30));
    // A file that no longer reads keeps its rows and says why.
    std::fs::write(&spending, "what,day\nonly,two\n").unwrap();
    assert!(n
        .api
        .pull_source(
            &OWNER,
            SourceRef {
                source: "spending".into(),
            },
        )
        .await
        .is_err());
    let sources = n.api.list_sources(&OWNER, Empty {}).await.unwrap();
    let failed = sources.iter().find(|s| s.name == "spending").unwrap();
    assert!(
        failed
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("columns")),
        "{failed:?}"
    );

    // Removing one is for good, and the rest stay examples.
    n.api
        .drop_space(
            &OWNER,
            SpaceRef {
                space: "oikome-guide".into(),
            },
        )
        .await
        .unwrap();
    let again = n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    assert_eq!(again.len(), 3);
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(storages.len(), 1);
    assert_eq!(storages[0].name, None);
    assert_eq!(storages[0].kind, StorageType::Disk);
    assert_eq!(storages[0].spaces, 3);
    if cfg!(unix) {
        assert!(storages[0].total_bytes.is_some_and(|t| t > 0));
    }

    // Its files stay, and keep the name.
    let err = n
        .api
        .create_space(
            &OWNER,
            NewSpace {
                name: "oikome-guide".into(),
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

    // The examples' files go into the picked folder; their collections,
    // read from them, are kept with the data center.
    let examples = n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    assert_eq!(examples.len(), 4);
    assert!(picked.path().join("Oikome/Examples/oikome-guide").is_dir());
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "plans".into(),
                kind: None,
                storage: Some("documents".into()),
            },
        )
        .await
        .unwrap();
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert!(!storages[0].is_default);
    assert_eq!(storages[1].spaces, 1);

    // Back to this computer.
    let storages = n
        .api
        .set_default_storage(&OWNER, DefaultStorage { storage: None })
        .await
        .unwrap();
    assert!(storages[0].is_default && !storages[1].is_default);

    // A place stays while collections are on it.
    let err = n
        .api
        .remove_storage(
            &OWNER,
            StorageRef {
                storage: "documents".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err}");
    assert!(err.message.contains("1 collection;"), "{err}");

    // An empty one goes, files and all left where they are, and Local is
    // the default again when it was the default.
    let spare = tempfile::tempdir().unwrap();
    n.api
        .add_storage(
            &OWNER,
            NewStorage {
                label: "Spare".into(),
                kind: StorageType::Folder,
                address: spare.path().to_str().unwrap().into(),
                username: None,
                password: None,
                encrypted: None,
                make_default: Some(true),
            },
        )
        .await
        .unwrap();
    // A collection made there and removed leaves its files, not the place.
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "scratch".into(),
                kind: None,
                storage: Some("spare".into()),
            },
        )
        .await
        .unwrap();
    n.api
        .drop_space(
            &OWNER,
            SpaceRef {
                space: "scratch".into(),
            },
        )
        .await
        .unwrap();
    let storages = n
        .api
        .remove_storage(
            &OWNER,
            StorageRef {
                storage: "spare".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(storages.len(), 2);
    assert!(storages[0].is_default);
    assert!(spare.path().join("Oikome").is_dir());
    let err = n
        .api
        .remove_storage(
            &OWNER,
            StorageRef {
                storage: "spare".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

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

#[tokio::test]
async fn lending_this_computer_to_contacts_then_pausing_and_stopping() {
    let rosh = node();
    let rosh_api = rosh.api.clone().with_flight_url("https://127.0.0.1:9");
    rosh_api
        .rename_node(
            &OWNER,
            NodeName {
                name: "Rosh's data center".into(),
            },
        )
        .await
        .unwrap();
    let sam = node();
    let sam_did = sam.api.node_info(&OWNER, Empty {}).await.unwrap().did;
    let lend = |on: bool, people: Vec<String>, concurrent: u32, per_hour: u32| SetLending {
        on,
        people,
        concurrent,
        per_hour,
    };

    // Not lent yet; lending is to contacts, within limits.
    let lending = rosh_api.lending(&OWNER, Empty {}).await.unwrap();
    assert!(!lending.on && lending.people.is_empty());
    assert_eq!((lending.concurrent, lending.per_hour), (1, 30));
    let e = rosh_api
        .set_lending(&OWNER, lend(true, vec![sam_did.clone()], 2, 20))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    rosh_api
        .add_contact(
            &OWNER,
            ContactInput {
                did: sam_did.clone(),
                name: "Sam".into(),
            },
        )
        .await
        .unwrap();
    let e = rosh_api
        .set_lending(&OWNER, lend(true, vec![sam_did.clone()], 0, 20))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // Lent to Sam: a code for Sam, with the limits in it.
    let saved = rosh_api
        .set_lending(&OWNER, lend(true, vec![sam_did.clone()], 2, 20))
        .await
        .unwrap();
    assert!(saved.lending.on);
    assert_eq!((saved.lending.concurrent, saved.lending.per_hour), (2, 20));
    assert_eq!(saved.lending.people.len(), 1);
    assert_eq!(saved.lending.people[0].name.as_deref(), Some("Sam"));
    assert_eq!(saved.invites.len(), 1);
    let code = saved.invites[0].invite.clone();

    // Sam adds it: Rosh's computer, 2 tasks at once, 20 an hour.
    let invite = add(&sam, &code).await.unwrap();
    assert_eq!(invite.kind, InviteKind::Compute);
    assert_eq!(invite.from_name, "Rosh's data center");
    assert_eq!((invite.concurrent, invite.per_hour), (Some(2), Some(20)));
    let accepted = sam
        .api
        .accept_invite(
            &OWNER,
            AcceptInvite {
                id: invite.id,
                name: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(accepted.kind, InviteKind::Compute);
    assert_eq!(accepted.name, "Rosh's data center");
    let e = add(&sam, &code).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    // Rosh's data center isn't running here, so it can't be reached.
    let borrowed = sam.api.list_borrowed(&OWNER, Empty {}).await.unwrap();
    assert_eq!(borrowed.len(), 1);
    assert_eq!(borrowed[0].from_name, "Rosh's data center");
    assert_eq!(borrowed[0].task_ms, Some(30_000));
    assert_eq!(borrowed[0].status, BorrowStatus::Unreachable);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = sam
            .catalog
            .oikome_dir()
            .join("identity")
            .join("borrowed.json");
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Paused and resumed without new codes; new limits for everyone.
    let paused = rosh_api
        .set_lending(&OWNER, lend(false, vec![sam_did.clone()], 4, 100))
        .await
        .unwrap();
    assert!(!paused.lending.on);
    assert_eq!(paused.lending.people.len(), 1);
    assert_eq!(
        (paused.lending.concurrent, paused.lending.per_hour),
        (4, 100)
    );
    assert!(paused.invites.is_empty());
    let resumed = rosh_api
        .set_lending(&OWNER, lend(true, vec![sam_did.clone()], 4, 100))
        .await
        .unwrap();
    assert!(resumed.lending.on && resumed.invites.is_empty());

    // Removing Sam as a contact stops lending to Sam.
    rosh_api
        .remove_contact(
            &OWNER,
            DidRef {
                did: sam_did.clone(),
            },
        )
        .await
        .unwrap();
    let lending = rosh_api.lending(&OWNER, Empty {}).await.unwrap();
    assert!(!lending.on && lending.people.is_empty());

    // Sam stops using it.
    let left = sam
        .api
        .stop_borrowing(
            &OWNER,
            BorrowedRef {
                id: borrowed[0].id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(left.is_empty());
}

#[tokio::test]
async fn a_plugin_reads_only_what_its_owner_let_it() {
    let n = node();
    notes(&n).await;
    sql(
        &n,
        "CREATE SPACE diary; CREATE TABLE diary.days (id BIGINT NOT NULL, entry VARCHAR); \
         INSERT INTO diary.days VALUES (1, 'private')",
    )
    .await;
    let read = |plugin: &str, space: &str| PluginRead {
        plugin: plugin.into(),
        space: space.into(),
        limit: None,
    };

    // Oikome's own are offered; one is added, to read notes only.
    let offered = n.api.list_plugins(&OWNER, Empty {}).await.unwrap();
    assert!(offered
        .iter()
        .any(|p| p.id == "spending-by-category" && p.built_in && !p.installed));
    let spending = n
        .api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("spending-by-category".into()),
                path: None,
                collections: vec!["notes".into()],
            },
        )
        .await
        .unwrap();
    assert!(spending.installed);
    assert_eq!(spending.collections, vec!["notes".to_string()]);
    let rows = n
        .api
        .plugin_read(&OWNER, read(&spending.id, "notes"))
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 3);
    let e = n
        .api
        .plugin_read(&OWNER, read(&spending.id, "diary"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);

    // Its record claiming more changes nothing: the node's grants decide.
    let record = n.catalog.oikome_dir().join("plugins.json");
    let mut kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    kept[0]["collections"] = serde_json::json!(["notes", "diary"]);
    std::fs::write(&record, serde_json::to_vec(&kept).unwrap()).unwrap();
    let e = n
        .api
        .plugin_read(&OWNER, read(&spending.id, "diary"))
        .await
        .unwrap_err();
    // Without a grant there, the collection doesn't exist for the plugin.
    assert_eq!(e.code, ErrorCode::NotFound, "{e}");
    kept[0]["collections"] = serde_json::json!(["notes"]);
    std::fs::write(&record, serde_json::to_vec(&kept).unwrap()).unwrap();

    // Its page loads the bridge first; nothing outside its folder is served.
    let page = String::from_utf8(n.api.plugin_file(&spending.id, "index.html").unwrap()).unwrap();
    assert!(page.find("/plugins/sdk.js").unwrap() < page.find("main.js").unwrap());
    assert!(n.api.plugin_file(&spending.id, "../plugins.json").is_none());
    assert!(n.api.plugin_file(&spending.id, "main.js").is_some());
    assert!(n.api.plugin_file("photo-memories", "index.html").is_none());

    // One from a folder: looked at, then added.
    let folder = tempfile::tempdir().unwrap();
    let manifest = r#"{"id": "my-card", "name": "My card", "by": "Sam", "version": "0.1.0",
        "reads": {"what": "Anything", "shape": "any"}, "card": {"width": 6, "height": 200}}"#;
    std::fs::write(folder.path().join("oikome-plugin.json"), manifest).unwrap();
    std::fs::write(
        folder.path().join("index.html"),
        "<html><head></head><body></body></html>",
    )
    .unwrap();
    std::fs::create_dir(folder.path().join("img")).unwrap();
    std::fs::write(folder.path().join("img").join("a.png"), [0u8; 4]).unwrap();
    let path = folder.path().to_string_lossy().into_owned();
    let seen = n
        .api
        .inspect_plugin(&OWNER, PluginFolder { path: path.clone() })
        .await
        .unwrap();
    assert_eq!(
        (seen.id.as_str(), seen.width, seen.installed),
        ("my-card", 6, false)
    );
    let mine = n
        .api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: None,
                path: Some(path.clone()),
                collections: vec!["diary".into()],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        row_count(
            &n.api
                .plugin_read(&OWNER, read(&mine.id, "diary"))
                .await
                .unwrap()
        ),
        1
    );
    assert!(n.api.plugin_file("my-card", "img/a.png").is_some());

    // Folders that aren't plugins, or hold what a plugin may not, are refused.
    let bad = tempfile::tempdir().unwrap();
    let bad_path = bad.path().to_string_lossy().into_owned();
    let inspect = |path: String| n.api.inspect_plugin(&OWNER, PluginFolder { path });
    assert_eq!(
        inspect(bad_path.clone()).await.unwrap_err().code,
        ErrorCode::Invalid
    );
    std::fs::write(
        bad.path().join("oikome-plugin.json"),
        manifest.replace("my-card", "My Card"),
    )
    .unwrap();
    std::fs::write(bad.path().join("index.html"), "").unwrap();
    assert_eq!(
        inspect(bad_path.clone()).await.unwrap_err().code,
        ErrorCode::Invalid
    );
    std::fs::write(bad.path().join("oikome-plugin.json"), manifest).unwrap();
    std::fs::write(bad.path().join("run.exe"), "").unwrap();
    assert_eq!(
        inspect(bad_path.clone()).await.unwrap_err().code,
        ErrorCode::Invalid
    );
    std::fs::remove_file(bad.path().join("run.exe")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", bad.path().join("hosts.txt")).unwrap();
        assert_eq!(
            inspect(bad_path).await.unwrap_err().code,
            ErrorCode::Invalid
        );
    }

    // What it may read changes, and its reads follow.
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: spending.id.clone(),
                collections: vec!["diary".into()],
            },
        )
        .await
        .unwrap();
    assert!(n
        .api
        .plugin_read(&OWNER, read(&spending.id, "notes"))
        .await
        .is_err());
    assert!(n
        .api
        .plugin_read(&OWNER, read(&spending.id, "diary"))
        .await
        .is_ok());

    // Removed: it reads nothing, its files go.
    let left = n
        .api
        .remove_plugin(
            &OWNER,
            PluginRef {
                id: spending.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(left
        .iter()
        .any(|p| p.id == "spending-by-category" && !p.installed));
    assert!(n
        .api
        .plugin_read(&OWNER, read(&spending.id, "diary"))
        .await
        .is_err());
    assert!(n.api.plugin_file(&spending.id, "index.html").is_none());
    assert!(!n
        .catalog
        .oikome_dir()
        .join("plugins")
        .join(&spending.id)
        .exists());

    // A principal can't add plugins.
    let e = n
        .api
        .list_plugins(&Caller::Principal { did: did(9) }, Empty {})
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
}

/// Activity names who each line was: you, a person, a plugin (by the name
/// it had, once removed), or a computer it has no name for.
#[tokio::test]
async fn activity_names_who_it_was() {
    use oikome_node::{AccessEntry, AccessLog, Outcome};
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), NodeConfig::default()).unwrap());
    let log = AccessLog::open(&catalog.oikome_dir()).unwrap();
    let api = Api::new(catalog.clone(), engines.clone()).with_access_log(log.clone());
    let n = Node {
        _dir: dir,
        catalog,
        engines,
        api,
    };
    notes(&n).await;
    let own = n.api.node_info(&OWNER, Empty {}).await.unwrap().did;
    let (bob, stranger) = (did(2), did(3));
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
    // Bob, a stranger and someone unauthenticated ask; so does the owner,
    // from this computer.
    for caller in [Some(bob.clone()), Some(stranger.clone()), None, Some(own)] {
        log.record(AccessEntry {
            at_ms: 1,
            caller,
            method: "do_get".into(),
            spaces: vec!["notes".into()],
            token_id: None,
            outcome: Outcome::Denied,
            reason: None,
            detail: None,
            merge: false,
        });
    }
    // A plugin reads, then it and Bob are removed.
    let plugin = n
        .api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("spending-by-category".into()),
                path: None,
                collections: vec!["notes".into()],
            },
        )
        .await
        .unwrap();
    n.api
        .plugin_read(
            &OWNER,
            PluginRead {
                plugin: plugin.id.clone(),
                space: "notes".into(),
                limit: None,
            },
        )
        .await
        .unwrap();
    n.api
        .remove_plugin(&OWNER, PluginRef { id: plugin.id })
        .await
        .unwrap();
    n.api
        .remove_contact(&OWNER, DidRef { did: bob.clone() })
        .await
        .unwrap();

    let mut items = Vec::new();
    for _ in 0..100 {
        items = n
            .api
            .list_access(&OWNER, ListAccess::default())
            .await
            .unwrap();
        if items.len() >= 5 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let of = |did: Option<&str>, method: &str| {
        let item = items
            .iter()
            .find(|i| i.did.as_deref() == did && i.method == method)
            .unwrap_or_else(|| panic!("no {method} line by {did:?}"));
        (item.who, item.name.clone())
    };
    assert_eq!(
        of(Some(&bob), "do_get"),
        (AccessWho::Person, Some("Bob".into()))
    );
    assert_eq!(of(Some(&stranger), "do_get"), (AccessWho::Unknown, None));
    assert_eq!(of(None, "do_get"), (AccessWho::Unknown, None));
    let read = items.iter().find(|i| i.method == "plugin_read").unwrap();
    assert_eq!(
        (read.who, read.name.as_deref()),
        (AccessWho::Plugin, Some("Spending by category"))
    );
    assert_eq!(items.iter().filter(|i| i.who == AccessWho::You).count(), 1);
}

/// A source can be paused and resumed; removed, its collection stays, and
/// adding the same folder again puts it back into that collection.
#[tokio::test]
async fn a_source_pauses_and_comes_back_into_its_collection() {
    let n = node();
    n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    let guide =
        |sources: &[SourceSummary]| sources.iter().find(|s| s.name == "oikome-guide").cloned();
    let pause = |paused| PauseSource {
        source: "oikome-guide".into(),
        paused,
    };
    let paused = n.api.pause_source(&OWNER, pause(true)).await.unwrap();
    assert!(paused.paused);
    let listed = n.api.list_sources(&OWNER, Empty {}).await.unwrap();
    assert!(guide(&listed).unwrap().paused);
    assert!(
        !n.api
            .pause_source(&OWNER, pause(false))
            .await
            .unwrap()
            .paused
    );

    let place = guide(&listed).unwrap().place;
    let left = n
        .api
        .remove_source(
            &OWNER,
            SourceRef {
                source: "oikome-guide".into(),
            },
        )
        .await
        .unwrap();
    assert!(guide(&left).is_none());
    assert!(n.catalog.get_space("oikome-guide").is_ok());
    let back = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::Notes,
                path: place,
            },
        )
        .await
        .unwrap();
    assert_eq!(back.name, "oikome-guide");
    assert!(n.catalog.get_space("oikome-guide-2").is_err());

    // How often it is read: one of the choices, or only when asked.
    let every = |every_secs| SetSourceEvery {
        source: "oikome-guide".into(),
        every_secs,
    };
    let set = n
        .api
        .set_source_every(&OWNER, every(Some(300)))
        .await
        .unwrap();
    assert_eq!(set.every_secs, Some(300));
    assert!(n
        .api
        .set_source_every(&OWNER, every(Some(42)))
        .await
        .is_err());
    let set = n.api.set_source_every(&OWNER, every(None)).await.unwrap();
    assert_eq!(set.every_secs, None);

    // Its recent syncs: what came in, then nothing new again and again as
    // one line, then a failure.
    let guide = || SourceRef {
        source: "oikome-guide".into(),
    };
    n.api.pull_source(&OWNER, guide()).await.unwrap();
    n.api.pull_source(&OWNER, guide()).await.unwrap();
    let runs = n.api.list_source_runs(&OWNER, guide()).await.unwrap();
    assert_eq!(runs[0].upserted + runs[0].deleted, 0, "{runs:?}");
    // Added back, then read twice: three times nothing new.
    assert_eq!(runs[0].count, 3, "{runs:?}");
    assert!(runs[0].first_ms < runs[0].at_ms, "{runs:?}");
    assert!(runs.iter().any(|r| r.upserted == 7), "{runs:?}");
    let root = std::path::PathBuf::from(n.catalog.root_path());
    std::fs::write(
        root.join("Examples/spending/spending.csv"),
        "what,day\nonly,two\n",
    )
    .unwrap();
    let spending = || SourceRef {
        source: "spending".into(),
    };
    assert!(n.api.pull_source(&OWNER, spending()).await.is_err());
    let runs = n.api.list_source_runs(&OWNER, spending()).await.unwrap();
    assert!(runs[0].failure.is_some(), "{runs:?}");
}

/// Search finds what sources brought in (photo captions, notes, table rows),
/// follows their files as they change, reads collections with no index
/// too, and takes what is typed as words, never as syntax.
#[tokio::test]
async fn search_finds_what_sources_bring_in() {
    let n = node();
    n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    let search = |query: &str, collection: Option<&str>| Search {
        query: query.into(),
        collection: collection.map(Into::into),
        limit: None,
    };
    let marked = |hit: &SearchHit| -> Vec<String> {
        let units: Vec<u16> = hit.excerpt.encode_utf16().collect();
        hit.marks
            .iter()
            .map(|m| String::from_utf16_lossy(&units[m.start as usize..m.end as usize]))
            .collect()
    };

    // A photo by its caption, case and accents aside; opened by its key.
    let found = n
        .api
        .search(&OWNER, search("pasteis BELEM", None))
        .await
        .unwrap();
    let photo = found
        .hits
        .iter()
        .find(|h| h.collection == "lisbon-photos")
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(photo.table, "photos");
    assert!(marked(photo).contains(&"Pastéis".to_string()), "{photo:?}");
    let row = n
        .api
        .read_row(
            &OWNER,
            ReadRow {
                space: photo.collection.clone(),
                table: photo.table.clone(),
                key: photo.key.clone().unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&row), 1);
    // The page knows the key, so each photo has a link of its own.
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
    assert_eq!(tables[0].key, vec!["file".to_string()]);

    // Rows of a table, in the one collection asked for.
    let found = n
        .api
        .search(&OWNER, search("groceries", Some("spending")))
        .await
        .unwrap();
    assert!(found.hits.len() >= 3, "{found:?}");
    assert!(found.hits.iter().all(|h| h.collection == "spending"));

    // A note written and read again: words of three characters or more
    // through the index, shorter ones too (Chinese has no spaces).
    let guide = std::path::PathBuf::from(n.catalog.root_path()).join("Examples/oikome-guide");
    let lisbon = guide.join("9 Lisbon.md");
    std::fs::write(&lisbon, "今天在里斯本吃了蛋挞, and a bica at Brasileira.").unwrap();
    let pull = || SourceRef {
        source: "oikome-guide".into(),
    };
    n.api.pull_source(&OWNER, pull()).await.unwrap();
    for query in ["里斯本", "蛋挞", "brasileira bica"] {
        let found = n.api.search(&OWNER, search(query, None)).await.unwrap();
        let note = found
            .hits
            .iter()
            .find(|h| h.collection == "oikome-guide")
            .unwrap_or_else(|| panic!("{query}: {found:?}"));
        assert_eq!(note.column, "note");
        assert!(!note.marks.is_empty(), "{query}: {note:?}");
    }

    // Changed: the new words are found, the old ones no longer.
    std::fs::write(&lisbon, "Ginjinha in Rossio.").unwrap();
    n.api.pull_source(&OWNER, pull()).await.unwrap();
    let hits = |query: &str| {
        let api = &n.api;
        let query = query.to_string();
        async move {
            api.search(&OWNER, search(&query, Some("oikome-guide")))
                .await
                .unwrap()
                .hits
                .len()
        }
    };
    assert_eq!(hits("ginjinha").await, 1);
    assert_eq!(hits("brasileira").await, 0);
    // Removed: gone from the index with its row.
    std::fs::remove_file(&lisbon).unwrap();
    n.api.pull_source(&OWNER, pull()).await.unwrap();
    assert_eq!(hits("ginjinha").await, 0);

    // A collection with no index (made with SQL) is read and matched.
    notes(&n).await;
    sql(
        &n,
        "INSERT INTO notes.memos VALUES (4, 'Remember the tram 28')",
    )
    .await;
    let found = n
        .api
        .search(&OWNER, search("tram 28", Some("notes")))
        .await
        .unwrap();
    assert_eq!(found.hits.len(), 1, "{found:?}");
    assert_eq!(found.hits[0].excerpt, "Remember the tram 28");

    // Syntax of SQL, FTS5 and LIKE is only words to look for.
    for query in [
        "\"; DROP TABLE notes; --",
        "NEAR(",
        "a\"b OR *",
        "%",
        "_",
        "\\",
        "'",
    ] {
        let found = n.api.search(&OWNER, search(query, None)).await;
        assert!(found.is_ok(), "{query}: {found:?}");
        assert!(found.unwrap().skipped.is_empty(), "{query}");
    }
    assert!(n
        .api
        .search(&OWNER, search(&"x".repeat(201), None))
        .await
        .is_err());
    assert_eq!(hits("ginjinha").await, 0);
    assert!(n
        .api
        .search(&OWNER, search("welcome", None))
        .await
        .unwrap()
        .hits
        .iter()
        .any(|h| h.collection == "oikome-guide"));

    // Only the owner searches.
    let stranger = Caller::Principal { did: did(9) };
    assert!(n.api.search(&stranger, search("tram", None)).await.is_err());
}

/// A node whose requests go to an access log, as `serve` keeps one.
fn logged_node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), NodeConfig::default()).unwrap());
    let log = oikome_node::AccessLog::open(&catalog.oikome_dir()).unwrap();
    let api = Api::new(catalog.clone(), engines.clone()).with_access_log(log);
    Node {
        _dir: dir,
        catalog,
        engines,
        api,
    }
}

/// Activity's lines once `n` of them are written (the log writes behind).
async fn activity(node: &Node, n: usize) -> Vec<AccessItem> {
    for _ in 0..300 {
        let items = node
            .api
            .list_access(&OWNER, ListAccess::default())
            .await
            .unwrap();
        if items.len() >= n {
            return items;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("Activity never had {n} lines");
}

#[tokio::test]
async fn a_principal_searches_only_what_it_may_read() {
    use oikome_node::{OikomeSession, QueryResult, SessionConfig};
    let n = node();
    // A notes folder of the owner's, filled by a source: indexed.
    let folder = std::path::PathBuf::from(n.catalog.root_path()).join("journal");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("plans.md"),
        "Secret plans for Lisbon in spring.",
    )
    .unwrap();
    std::fs::write(
        folder.join("trams.md"),
        "The trams of Lisbon climb all day.",
    )
    .unwrap();
    let source = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::Notes,
                path: folder.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(source.name, "journal");
    n.api
        .pull_source(
            &OWNER,
            SourceRef {
                source: "journal".into(),
            },
        )
        .await
        .unwrap();
    // Alice may see the file names only.
    sql(&n, "GRANT SELECT (file) ON journal.notes TO alice").await;
    let session = |who: &str| {
        OikomeSession::new(
            n.catalog.clone(),
            n.engines.clone(),
            SessionConfig {
                default_space: None,
                principal: Some(who.to_string()),
            },
        )
    };
    let terms = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    // The owner finds both.
    let found = n
        .api
        .search(
            &OWNER,
            Search {
                query: "lisbon".into(),
                collection: Some("journal".into()),
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(found.hits.len(), 2, "{found:?}");

    // Words only in a column it may not see find nothing; its own columns
    // are all it gets back.
    let alice = session("alice");
    assert!(alice
        .search("journal", &terms(&["lisbon"]), 10)
        .await
        .unwrap()
        .is_empty());
    let by_name = alice
        .search("journal", &terms(&["plans"]), 10)
        .await
        .unwrap();
    assert_eq!(by_name.len(), 1);
    assert!(by_name[0].texts.iter().all(|(column, _)| column == "file"));
    let row = alice
        .row_by_key("journal", "notes", &["plans.md".to_string()])
        .await
        .unwrap();
    let QueryResult::Batches { schema, .. } = row else {
        panic!("a row")
    };
    assert_eq!(schema.fields().len(), 1);

    // Bob may see one note: a row his policy leaves out is not found, nor
    // read. (With a policy on the table, anyone it doesn't name sees no row.)
    sql(&n, "GRANT SELECT ON journal.notes TO bob").await;
    sql(
        &n,
        "CREATE POLICY trams_only ON journal.notes FOR SELECT TO bob USING (file = 'trams.md')",
    )
    .await;
    let bob = session("bob");
    let seen = bob
        .search("journal", &terms(&["lisbon"]), 10)
        .await
        .unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].key, Some(vec!["trams.md".to_string()]));
    let QueryResult::Batches { batches, .. } = bob
        .row_by_key("journal", "notes", &["plans.md".to_string()])
        .await
        .unwrap()
    else {
        panic!("rows")
    };
    assert!(batches.iter().all(|b| b.num_rows() == 0));

    // Someone with no grant finds nothing there at all.
    assert!(session("carol")
        .search("journal", &terms(&["lisbon"]), 10)
        .await
        .is_err());
}

#[tokio::test]
async fn an_assistant_reads_only_the_collections_it_is_given() {
    let n = logged_node();
    n.api.add_examples(&OWNER, Empty {}).await.unwrap();
    let q = |query: &str, collection: Option<&str>| Search {
        query: query.into(),
        collection: collection.map(Into::into),
        limit: None,
    };

    // Connected to read the guide; its token is shown once, not kept.
    let connected = n
        .api
        .connect_assistant(
            &OWNER,
            ConnectAssistant {
                name: "Claude Code".into(),
                app: AssistantApp::ClaudeCode,
                collections: vec!["oikome-guide".into()],
            },
        )
        .await
        .unwrap();
    let id = connected.assistant.id.clone();
    assert!(connected.token.starts_with("oik_"));
    assert_eq!(connected.assistant.collections, vec!["oikome-guide"]);
    assert_eq!(connected.assistant.last_used_ms, None);
    let kept = std::fs::read_to_string(n.catalog.oikome_dir().join("assistants.json")).unwrap();
    assert!(!kept.contains(&connected.token));
    let me = n
        .api
        .assistant_by_token(&connected.token)
        .expect("its token");
    assert!(n.api.assistant_by_token("oik_not-its-token").is_none());
    let listed = n.api.list_assistants(&OWNER, Empty {}).await.unwrap();
    assert!(listed[0].last_used_ms.is_some());

    // It is told of its collection only, and does none of the owner's things.
    let readable = n.api.assistant_collections(&me, Empty {}).await.unwrap();
    assert_eq!(readable.len(), 1);
    assert_eq!(readable[0].name, "oikome-guide");
    let table = &readable[0].tables[0];
    assert!(table.columns.contains(&"note".to_string()));
    assert_eq!(table.key, vec!["file".to_string()]);
    assert_eq!(table.rows, Some(7));
    assert!(n.api.list_spaces(&me, Empty {}).await.is_err());
    assert!(n.api.search(&me, q("welcome", None)).await.is_err());
    assert!(n.api.assistant_collections(&OWNER, Empty {}).await.is_err());

    // Its search: its collection only; one it may not read doesn't exist.
    let found = n
        .api
        .assistant_search(&me, q("welcome", None))
        .await
        .unwrap();
    assert!(!found.hits.is_empty());
    assert!(found.hits.iter().all(|h| h.collection == "oikome-guide"));
    assert!(n
        .api
        .assistant_search(&me, q("belem", None))
        .await
        .unwrap()
        .hits
        .is_empty());
    let e = n
        .api
        .assistant_search(&me, q("belem", Some("lisbon-photos")))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let hit = &found.hits[0];
    let row = n
        .api
        .assistant_read_row(
            &me,
            ReadRow {
                space: hit.collection.clone(),
                table: hit.table.clone(),
                key: hit.key.clone().unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&row), 1);
    let e = n
        .api
        .assistant_read_row(
            &me,
            ReadRow {
                space: "lisbon-photos".into(),
                table: "photos".into(),
                key: vec!["Tram 28E.jpg".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    // Given the photos too, it finds them; taken back, it doesn't.
    let give = |collections: &[&str]| AssistantCollections {
        id: id.clone(),
        collections: collections.iter().map(|c| c.to_string()).collect(),
    };
    n.api
        .set_assistant_collections(&OWNER, give(&["oikome-guide", "lisbon-photos"]))
        .await
        .unwrap();
    let photos = n.api.assistant_search(&me, q("belem", None)).await.unwrap();
    assert!(photos.hits.iter().any(|h| h.collection == "lisbon-photos"));
    n.api
        .set_assistant_collections(&OWNER, give(&["oikome-guide"]))
        .await
        .unwrap();
    assert!(n
        .api
        .assistant_search(&me, q("belem", None))
        .await
        .unwrap()
        .hits
        .is_empty());

    // Activity: what it searched for, what it read, what it was refused.
    let lines = activity(&n, 7).await;
    let mine: Vec<&AccessItem> = lines
        .iter()
        .filter(|l| l.who == AccessWho::Assistant)
        .collect();
    assert!(mine
        .iter()
        .all(|l| l.name.as_deref() == Some("Claude Code")));
    assert!(mine.iter().any(|l| l.method == "assistant_search"
        && l.detail.as_deref() == Some("welcome")
        && l.outcome == AccessOutcome::Allowed));
    assert!(mine
        .iter()
        .any(|l| l.method == "assistant_read" && l.outcome == AccessOutcome::Allowed));
    assert!(mine.iter().any(|l| l.method == "assistant_search"
        && l.spaces == ["lisbon-photos"]
        && l.outcome == AccessOutcome::Denied));

    // Its access ended: the token stops working and the grants go at once.
    let Caller::Principal { did: its_did } = &me else {
        panic!("an assistant is a principal")
    };
    let ended = n
        .api
        .end_assistant_access(&OWNER, AssistantRef { id: id.clone() })
        .await
        .unwrap();
    assert!(ended.ended_at_ms.is_some());
    assert!(ended.collections.is_empty());
    assert!(n.api.assistant_by_token(&connected.token).is_none());
    assert!(n
        .api
        .assistant_search(&me, q("welcome", None))
        .await
        .is_err());
    assert!(!n.catalog.has_space_grant(its_did, "oikome-guide"));
    assert!(n
        .api
        .set_assistant_collections(&OWNER, give(&["oikome-guide"]))
        .await
        .is_err());

    // Removed, it is no longer listed; Activity keeps its name.
    let left = n
        .api
        .remove_assistant(&OWNER, AssistantRef { id })
        .await
        .unwrap();
    assert!(left.is_empty());
    let lines = activity(&n, 7).await;
    assert!(lines
        .iter()
        .filter(|l| l.who == AccessWho::Assistant)
        .all(|l| l.name.as_deref() == Some("Claude Code")));

    // Only the owner's own collections, and a name, are accepted.
    let connect = |name: &str, collections: Vec<String>| ConnectAssistant {
        name: name.into(),
        app: AssistantApp::Other,
        collections,
    };
    assert!(n
        .api
        .connect_assistant(&OWNER, connect("", vec![]))
        .await
        .is_err());
    assert!(n
        .api
        .connect_assistant(&OWNER, connect("Another", vec!["no-such".into()]))
        .await
        .is_err());
    assert!(n
        .api
        .connect_assistant(&me, connect("Another", vec![]))
        .await
        .is_err());
}
