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
