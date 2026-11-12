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
use oikome_node::{EngineRegistry, MemorySecrets, NodeConfig};

/// A node's settings for a test: credentials in memory, never in the
/// machine's keychain.
fn test_config() -> NodeConfig {
    NodeConfig {
        secret_backend: Some(Arc::new(MemorySecrets::default())),
        ..NodeConfig::default()
    }
}

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
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), test_config()).unwrap());
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
                reach: None,
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
                reach: None,
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
                    reach: None,
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
                reach: None,
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
            reach: None,
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
    // Each says which failure it is, so the apps word it in their language.
    let e = add(&rosh, &made.invite).await.unwrap_err();
    assert!(e.message.contains("one you made"), "{e}");
    assert_eq!(e.reason.as_deref(), Some("invite_own"));
    let other = node();
    let e = add(&other, &made.invite).await.unwrap_err();
    assert!(e.message.contains("another data center"), "{e}");
    assert_eq!(e.reason.as_deref(), Some("invite_other"));
    let e = add(&sam, &made.invite[..60]).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(e.reason.as_deref(), Some("invite_incomplete"));

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
async fn object_storage_needs_its_bucket_and_keys() {
    let n = node();
    let s3 = |bucket: &str, password: Option<&str>| NewStorage {
        label: "Family".into(),
        kind: StorageType::S3,
        // Nothing listens here: these are refused before any request.
        address: "http://127.0.0.1:9".into(),
        username: Some("oikome".into()),
        password: password.map(Into::into),
        bucket: Some(bucket.into()),
        encrypted: None,
        make_default: None,
    };
    let e = n
        .api
        .add_storage(&OWNER, s3("family", None))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(e.reason.as_deref(), Some("storage_s3_incomplete"));
    let e = n
        .api
        .add_storage(&OWNER, s3("Family Photos", Some("secret")))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert!(e.message.contains("bucket"), "{e}");
    // Nothing was kept: only this computer.
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(storages.len(), 1);
}

#[tokio::test]
async fn the_examples_are_written_in_the_apps_language() {
    let n = node();
    let added = n
        .api
        .add_examples(
            &OWNER,
            AddExamples {
                language: Some("zh-Hans".into()),
            },
        )
        .await
        .unwrap();
    // The collections keep their names: they name folders and are typed in SQL.
    let names: Vec<&str> = added.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["home-climate", "lisbon-photos", "oikome-guide", "spending"]
    );
    let root = std::path::PathBuf::from(n.catalog.root_path()).join("Examples");
    let mut notes: Vec<String> = std::fs::read_dir(root.join("oikome-guide"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    notes.sort();
    assert_eq!(notes.len(), 7);
    assert!(
        notes.iter().all(|f| f.ends_with(".md") && !f.is_ascii()),
        "{notes:?}"
    );
    // The tables' columns are named in the language too, and still read.
    let spending = std::fs::read_to_string(root.join("spending/spending.csv")).unwrap();
    assert!(!spending.lines().next().unwrap().is_ascii(), "{spending}");
    let readings = std::fs::read_to_string(root.join("home-climate/readings.csv")).unwrap();
    let header = readings.lines().next().unwrap();
    assert!(
        header.contains("_celsius") && header.ends_with("_pct"),
        "{header}"
    );
    for (space, table, count) in [
        ("spending", "spending", 29),
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

    // A language they aren't written in gets English.
    let m = node();
    m.api
        .add_examples(
            &OWNER,
            AddExamples {
                language: Some("pt-BR".into()),
            },
        )
        .await
        .unwrap();
    let root = std::path::PathBuf::from(m.catalog.root_path()).join("Examples");
    assert!(root.join("oikome-guide/1 Welcome.md").is_file());
}

#[tokio::test]
async fn a_new_data_center_gets_examples_once() {
    let n = node();
    let added = n
        .api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
    let names: Vec<&str> = added.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["home-climate", "lisbon-photos", "oikome-guide", "spending"]
    );
    assert!(added.iter().all(|s| s.example && s.storage.is_none()));

    // Photos and notes are viewed where they are: nothing is copied, and
    // the index keeps a thumbnail of each photo.
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let lisbon = added.iter().find(|s| s.name == "lisbon-photos").unwrap();
    let viewed = lisbon.in_place.as_ref().expect("viewed in place");
    assert!(viewed.place.is_none());
    assert!(
        viewed.path.ends_with("Examples/lisbon-photos"),
        "{}",
        viewed.path
    );
    assert!(!root.join(".oikome/sqlite/lisbon-photos.sqlite").exists());
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "lisbon-photos".into(),
                folder: None,
                offset: None,
                limit: None,
                deep: None,
                kind: None,
                sort: None,
                descending: None,
            },
        )
        .await
        .unwrap();
    let photos: Vec<&FileItem> = listed
        .files
        .iter()
        .filter(|f| f.kind == FileKind::Image)
        .collect();
    assert_eq!(photos.len(), 6);
    assert!(photos.iter().all(|p| p.thumbnail && p.title.is_some()));
    let photo = FileRef {
        space: "lisbon-photos".into(),
        path: photos[0].path.clone(),
    };
    let thumb = n.api.file_thumbnail(&OWNER, photo.clone()).await.unwrap();
    assert!(thumb.starts_with(&[0xff, 0xd8]), "a JPEG");
    let original = n
        .api
        .file_original(&OWNER, photo.clone(), None)
        .await
        .unwrap();
    assert!(
        original.bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "the photo itself"
    );
    assert_eq!(original.bytes.len() as u64, original.total);
    let part = n
        .api
        .file_original(&OWNER, photo, Some((2, Some(11))))
        .await
        .unwrap();
    assert_eq!((part.start, part.bytes.len()), (2, 10));
    assert_eq!(part.bytes, original.bytes[2..12]);
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
    // The captions name the photos; they are no table of their own.
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["files"]);
    assert_eq!(tables[0].key, ["path"]);
    for (space, table, count) in [
        ("spending", "spending", 29),
        ("oikome-guide", "files", 7),
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

    // The tables are files read by sources: edit one and read it again.
    let spending = root.join("Examples/spending/spending.csv");
    assert!(root.join("Examples/lisbon-photos/captions.csv").is_file());
    assert!(root.join("Examples/oikome-guide/1 Welcome.md").is_file());
    let sources = n.api.list_sources(&OWNER, Empty {}).await.unwrap();
    let names: Vec<&str> = sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["home-climate", "spending"]);
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
    let again = n
        .api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
    assert_eq!(again.len(), 3);
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(storages.len(), 1);
    assert_eq!(storages[0].name, None);
    assert_eq!(storages[0].kind, StorageType::Disk);
    assert_eq!(storages[0].spaces, 3);
    if cfg!(unix) {
        assert!(storages[0].total_bytes.is_some_and(|t| t > 0));
    }

    // Its files stay where they are; it kept nothing of its own, so its
    // name is free again.
    assert!(root.join("Examples/oikome-guide/1 Welcome.md").is_file());
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "oikome-guide".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();

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
                bucket: None,
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
    let examples = n
        .api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
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
                bucket: None,
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
                bucket: None,
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
        reach: None,
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
    // Oikome's own come with their words in the app's languages.
    let spending = offered
        .iter()
        .find(|p| p.id == "spending-by-category")
        .unwrap();
    for language in ["de", "fr", "ja", "ko", "ru", "zh-Hans", "zh-Hant"] {
        let text = &spending.translations[language];
        assert!(
            text.name.is_some() && text.description.is_some() && text.reads.is_some(),
            "{language}"
        );
    }
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
    let e = inspect(bad_path.clone()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(e.reason.as_deref(), Some("plugin_no_manifest"));
    assert_eq!(
        e.params.get("path").map(String::as_str),
        Some(bad.path().canonicalize().unwrap().to_str().unwrap())
    );
    std::fs::write(
        bad.path().join("oikome-plugin.json"),
        manifest.replace("my-card", "My Card"),
    )
    .unwrap();
    std::fs::write(bad.path().join("index.html"), "").unwrap();
    // A rule for plugin makers: the detail stays in English, for them.
    let e = inspect(bad_path.clone()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(e.reason.as_deref(), Some("plugin_invalid"));
    assert_eq!(e.params.get("detail"), Some(&e.message));
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
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), test_config()).unwrap());
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
/// adding the same file again puts it back into that collection.
#[tokio::test]
async fn a_source_pauses_and_comes_back_into_its_collection() {
    let n = node();
    n.api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
    let guide = |sources: &[SourceSummary]| sources.iter().find(|s| s.name == "spending").cloned();
    let pause = |paused| PauseSource {
        source: "spending".into(),
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
                source: "spending".into(),
            },
        )
        .await
        .unwrap();
    assert!(guide(&left).is_none());
    assert!(n.catalog.get_space("spending").is_ok());
    let back = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::Csv,
                path: place,
            },
        )
        .await
        .unwrap();
    assert_eq!(back.name, "spending");
    assert!(n.catalog.get_space("spending-2").is_err());

    // How often it is read: one of the choices, or only when asked.
    let every = |every_secs| SetSourceEvery {
        source: "spending".into(),
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
        source: "spending".into(),
    };
    n.api.pull_source(&OWNER, guide()).await.unwrap();
    n.api.pull_source(&OWNER, guide()).await.unwrap();
    let runs = n.api.list_source_runs(&OWNER, guide()).await.unwrap();
    assert_eq!(runs[0].upserted + runs[0].deleted, 0, "{runs:?}");
    // Added back, then read twice: three times nothing new.
    assert_eq!(runs[0].count, 3, "{runs:?}");
    assert!(runs[0].first_ms < runs[0].at_ms, "{runs:?}");
    assert!(runs.iter().any(|r| r.upserted == 29), "{runs:?}");
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
    n.api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
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
    assert_eq!(photo.table, "files");
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
    assert_eq!(tables[0].key, vec!["path".to_string()]);

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
    // The folder is read again on its schedule, or now when asked.
    let pull = || SpaceRef {
        space: "oikome-guide".into(),
    };
    n.api.refresh_space(&OWNER, pull()).await.unwrap();
    for query in ["里斯本", "蛋挞", "brasileira bica"] {
        let found = n.api.search(&OWNER, search(query, None)).await.unwrap();
        let note = found
            .hits
            .iter()
            .find(|h| h.collection == "oikome-guide")
            .unwrap_or_else(|| panic!("{query}: {found:?}"));
        assert_eq!(note.column, "text");
        assert!(!note.marks.is_empty(), "{query}: {note:?}");
    }

    // Changed: the new words are found, the old ones no longer.
    std::fs::write(&lisbon, "Ginjinha in Rossio.").unwrap();
    n.api.refresh_space(&OWNER, pull()).await.unwrap();
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
    n.api.refresh_space(&OWNER, pull()).await.unwrap();
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
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), test_config()).unwrap());
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
    // A file of the owner's, filled into a collection by a source: indexed.
    let folder = std::path::PathBuf::from(n.catalog.root_path()).join("journal");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("journal.csv");
    std::fs::write(
        &file,
        "file,note\nplans.md,Secret plans for Lisbon in spring.\n\
         trams.md,The trams of Lisbon climb all day.\n",
    )
    .unwrap();
    let source = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::Csv,
                path: file.to_string_lossy().into_owned(),
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
    sql(&n, "GRANT SELECT (file) ON journal.journal TO alice").await;
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
        .row_by_key("journal", "journal", &["plans.md".to_string()])
        .await
        .unwrap();
    let QueryResult::Batches { schema, .. } = row else {
        panic!("a row")
    };
    assert_eq!(schema.fields().len(), 1);

    // Bob may see one note: a row his policy leaves out is not found, nor
    // read. (With a policy on the table, anyone it doesn't name sees no row.)
    sql(&n, "GRANT SELECT ON journal.journal TO bob").await;
    sql(
        &n,
        "CREATE POLICY trams_only ON journal.journal FOR SELECT TO bob USING (file = 'trams.md')",
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
        .row_by_key("journal", "journal", &["plans.md".to_string()])
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
    n.api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
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
    // The guide is a folder viewed in place: its files, keyed by path.
    let table = &readable[0].tables[0];
    assert_eq!(table.name, "files");
    assert!(table.columns.contains(&"text".to_string()));
    assert_eq!(table.key, vec!["path".to_string()]);
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
                table: "files".into(),
                key: vec!["tram.jpg".into()],
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

/// Big summaries go to a computer lent here only when its owner may read
/// all of what they are made from; otherwise this computer works them out,
/// and says why.
#[tokio::test]
async fn daily_totals_stay_here_unless_the_lender_may_read_all_of_a_big_collection() {
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
    let rosh_did = rosh_api.node_info(&OWNER, Empty {}).await.unwrap().did;
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
    let saved = rosh_api
        .set_lending(
            &OWNER,
            SetLending {
                on: true,
                people: vec![sam_did.clone()],
                concurrent: 1,
                per_hour: 30,
                reach: None,
            },
        )
        .await
        .unwrap();
    let invite = add(&sam, &saved.invites[0].invite).await.unwrap();
    sam.api
        .accept_invite(
            &OWNER,
            AcceptInvite {
                id: invite.id,
                name: None,
            },
        )
        .await
        .unwrap();
    let borrowed = sam.api.list_borrowed(&OWNER, Empty {}).await.unwrap();
    assert_eq!(borrowed.len(), 1);
    assert!(!borrowed[0].in_use);
    let id = borrowed[0].id.clone();

    // A small table of days written as text, with a gap and rows without
    // an amount or a day.
    for name in ["spending", "big"] {
        sam.api
            .create_space(
                &OWNER,
                NewSpace {
                    name: name.into(),
                    kind: None,
                    storage: None,
                },
            )
            .await
            .unwrap();
    }
    sql(
        &sam,
        "CREATE TABLE spending.days (what VARCHAR, day VARCHAR, euros DOUBLE); \
         INSERT INTO spending.days VALUES \
         ('bread', '2025-10-20', 2.5), ('tram', '2025-10-20 09:40:00', 3.0), \
         ('book', '2025-10-23', 12.0), ('nothing', '2025-10-24', NULL), \
         ('lost', NULL, 4.0), ('old', '2025-09-01', 100.0)",
    )
    .await;
    let totals_of = |space: &str, table: &str| DailyTotalsOf {
        space: space.into(),
        table: table.into(),
        when: "day".into(),
        amount: "euros".into(),
    };
    let totals = sam
        .api
        .daily_totals(&OWNER, totals_of("spending", "days"))
        .await
        .unwrap();
    assert_eq!(totals.rows, 6);
    assert_eq!(totals.days.len(), 14);
    // The latest day with an amount ends the fortnight (a day without one
    // doesn't); empty days are 0; a day too old is left out.
    assert_eq!(totals.days[13].day, "2025-10-23");
    assert_eq!(totals.days[0].day, "2025-10-10");
    let on = |day: &str| totals.days.iter().find(|d| d.day == day).unwrap().total;
    assert_eq!(on("2025-10-20"), 5.5);
    assert_eq!(on("2025-10-23"), 12.0);
    assert_eq!(on("2025-10-21"), 0.0);
    assert_eq!(totals.days.iter().map(|d| d.total).sum::<f64>(), 17.5);
    // Nothing lent is in use: no word about where.
    assert_eq!(totals.borrowed, None);
    assert_eq!(totals.stayed_here, None);

    // Only the table's own columns, and only its tables.
    let mut wrong = totals_of("spending", "days");
    wrong.amount = "euros\" FROM x --".into();
    let e = sam.api.daily_totals(&OWNER, wrong).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let e = sam
        .api
        .daily_totals(&OWNER, totals_of("spending", "nope"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    // In use: a small table still stays here, and says so.
    let listed = sam
        .api
        .use_borrowed(
            &OWNER,
            UseBorrowed {
                id: id.clone(),
                in_use: true,
            },
        )
        .await
        .unwrap();
    assert!(listed[0].in_use);
    let totals = sam
        .api
        .daily_totals(&OWNER, totals_of("spending", "days"))
        .await
        .unwrap();
    assert_eq!(totals.borrowed, None);
    assert_eq!(totals.stayed_here, Some(StayedHere::Small));
    assert_eq!(totals.lender.as_deref(), Some("Rosh's data center"));

    // A big one: 100,000 rows over ten days, a euro each.
    sql(
        &sam,
        "CREATE TABLE big.t (day VARCHAR, euros DOUBLE); \
         INSERT INTO big.t SELECT a.d, 1.0 FROM \
         (VALUES ('2025-01-01'), ('2025-01-02'), ('2025-01-03'), ('2025-01-04'), ('2025-01-05'), \
                 ('2025-01-06'), ('2025-01-07'), ('2025-01-08'), ('2025-01-09'), ('2025-01-10')) a(d), \
         (VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9)) b(x), \
         (VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9)) c(x), \
         (VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9)) d(x), \
         (VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9)) e(x)",
    )
    .await;
    // Not shared with Rosh: it stays here.
    let totals = sam
        .api
        .daily_totals(&OWNER, totals_of("big", "t"))
        .await
        .unwrap();
    assert_eq!(totals.rows, 100_000);
    assert_eq!(totals.stayed_here, Some(StayedHere::NotShared));
    assert_eq!(totals.days[13].day, "2025-01-10");
    assert_eq!(totals.days[13].total, 10_000.0);

    // Shared, but a row policy for someone else leaves Rosh no row: still
    // here.
    sam.api
        .share_space(
            &OWNER,
            ShareSpace {
                space: "big".into(),
                did: rosh_did.clone(),
                allow_copy: None,
                expires_in_days: None,
                reach: None,
            },
        )
        .await
        .unwrap();
    sql(
        &sam,
        "CREATE POLICY someone ON big.t FOR SELECT TO alice USING (true)",
    )
    .await;
    let totals = sam
        .api
        .daily_totals(&OWNER, totals_of("big", "t"))
        .await
        .unwrap();
    assert_eq!(totals.stayed_here, Some(StayedHere::PartlyShared));

    // Rosh may read all of it: it is sent, and Rosh's computer, not
    // running here, can't be reached. The answer is the same.
    sql(&sam, "DROP POLICY someone ON big.t").await;
    let totals = sam
        .api
        .daily_totals(&OWNER, totals_of("big", "t"))
        .await
        .unwrap();
    assert_eq!(totals.borrowed, None);
    assert_eq!(totals.stayed_here, Some(StayedHere::Unreachable));
    assert_eq!(totals.lender.as_deref(), Some("Rosh's data center"));
    assert_eq!(totals.days[13].total, 10_000.0);

    // Nothing was worked out there, so nothing to count; Rosh may be given
    // summaries of `big` only.
    let jobs = sam
        .api
        .borrowed_jobs(
            &OWNER,
            BorrowedJobsOf {
                id: id.clone(),
                since_ms: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(jobs.since, 0);
    assert!(jobs.recent.is_empty());
    assert_eq!(jobs.collections, vec!["big".to_string()]);

    // Turned off: kept so in the file.
    sam.api
        .use_borrowed(
            &OWNER,
            UseBorrowed {
                id: id.clone(),
                in_use: false,
            },
        )
        .await
        .unwrap();
    let again = Api::new(sam.catalog.clone(), sam.engines.clone());
    let listed = again.list_borrowed(&OWNER, Empty {}).await.unwrap();
    assert!(!listed[0].in_use);
    let e = sam
        .api
        .use_borrowed(
            &OWNER,
            UseBorrowed {
                id: "nope".into(),
                in_use: true,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

/// A folder viewed in place: chosen by its path, browsed from its index,
/// shared to view and never to copy; a folder source of old becomes one,
/// keeping its name and whom it was shared with.
#[tokio::test]
async fn folders_are_viewed_in_place_and_folder_sources_become_so() {
    use oikome_node::{OikomeSession, SessionConfig};
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());

    // Where a folder may be viewed from.
    for (path, code) in [
        ("", ErrorCode::Invalid),
        ("relative/folder", ErrorCode::Invalid),
        ("/no/such/folder", ErrorCode::Invalid),
    ] {
        let e = n
            .api
            .view_in_place(
                &OWNER,
                ViewInPlace {
                    place: None,
                    path: path.into(),
                    name: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code, "{path}: {e}");
    }
    let e = n
        .api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: Some("nas".into()),
                path: "photos".into(),
                name: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let inside = root.join(".oikome").join("index");
    std::fs::create_dir_all(&inside).unwrap();
    assert!(n
        .api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: inside.to_string_lossy().into_owned(),
                name: None,
            },
        )
        .await
        .is_err());

    // A folder of the owner's, with a photo, notes and a subfolder.
    let trip = root.join("Trip 2025");
    std::fs::create_dir_all(trip.join("Notes")).unwrap();
    let mut png = Vec::new();
    image_png(&mut png);
    std::fs::write(trip.join("tram.png"), &png).unwrap();
    for i in 0..3 {
        std::fs::write(trip.join(format!("{i}.txt")), format!("Day {i} in Lisbon")).unwrap();
    }
    std::fs::write(trip.join("Notes").join("ferry.md"), "Ferry to Cacilhas").unwrap();
    let viewed = n
        .api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: trip.to_string_lossy().into_owned(),
                name: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(viewed.name, "trip-2025");
    assert!(viewed.readonly);
    let in_place = viewed.in_place.unwrap();
    assert_eq!(in_place.files, 5);
    assert_eq!(in_place.reachable, Some(true));

    // Browsed a page at a time, folders first known by what they hold.
    let page = |folder: Option<&str>, offset, limit| ListFiles {
        space: "trip-2025".into(),
        folder: folder.map(Into::into),
        offset: Some(offset),
        limit: Some(limit),
        deep: None,
        kind: None,
        sort: None,
        descending: None,
    };
    let first = n.api.list_files(&OWNER, page(None, 0, 2)).await.unwrap();
    assert_eq!(first.total, 4);
    assert_eq!(first.files.len(), 2);
    assert_eq!(first.files[0].kind, FileKind::Image);
    assert_eq!(first.folders[0].name, "Notes");
    assert_eq!(first.folders[0].files, 1);
    let rest = n.api.list_files(&OWNER, page(None, 2, 2)).await.unwrap();
    assert_eq!(rest.files.len(), 2);
    let notes = n
        .api
        .list_files(&OWNER, page(Some("Notes"), 0, 10))
        .await
        .unwrap();
    assert_eq!(notes.files[0].path, "Notes/ferry.md");
    // In the order asked, either way; a folder says its bytes and newest file.
    assert!(first.folders[0].size > 0 && first.folders[0].modified_ms.is_some());
    // And what its cover shows, as a collection's does.
    assert!(first.folders.iter().all(|f| !f.cover.is_empty()));
    let by_size = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                sort: Some(FileSort::Size),
                descending: Some(true),
                ..page(None, 0, 10)
            },
        )
        .await
        .unwrap();
    let sizes: Vec<u64> = by_size.files.iter().map(|f| f.size).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    // A file says when it changed as a moment too, not only as the clock read.
    assert!(by_size.files.iter().all(|f| f.modified_ms > 0));
    // The list of collections says how big it is and when it last changed.
    let listed = n.api.list_spaces(&OWNER, Empty {}).await.unwrap();
    let trip = listed.iter().find(|s| s.name == "trip-2025").unwrap();
    assert!(trip.size_bytes.unwrap_or(0) > 0);
    assert!(trip.changed_at_ms.is_some());
    // Every folder at once, of one kind: what a cover shows.
    let deep = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "trip-2025".into(),
                folder: None,
                offset: None,
                limit: None,
                deep: Some(true),
                kind: Some(FileKind::Note),
                sort: None,
                descending: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(deep.total, 4);
    assert!(deep.folders.is_empty());
    assert!(deep.files.iter().all(|f| f.kind == FileKind::Note));
    let note = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "trip-2025".into(),
                path: "Notes/ferry.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(note.text.as_deref(), Some("Ferry to Cacilhas"));
    assert!(n
        .api
        .list_files(&OWNER, page(Some("../.."), 0, 10))
        .await
        .is_err());

    // Only a photo the index lists is sent, and only to the owner.
    let file = |path: &str| FileRef {
        space: "trip-2025".into(),
        path: path.into(),
    };
    for path in ["0.txt", "../../etc/passwd", "nope.png"] {
        assert_eq!(
            n.api
                .file_original(&OWNER, file(path), None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound,
            "{path}"
        );
    }
    let stranger = Caller::Principal { did: did(9) };
    assert!(n
        .api
        .file_thumbnail(&stranger, file("tram.png"))
        .await
        .is_err());

    // Shared to view where it is, never to keep a copy.
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
                space: "trip-2025".into(),
                did: bob.clone(),
                allow_copy: Some(true),
                expires_in_days: None,
                reach: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // A photos folder that a source copied (M2): viewed in place once the
    // node starts, under its name, still shared with Bob, with no copy.
    let photos = root.join("Old photos");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::write(photos.join("tram.png"), &png).unwrap();
    let owner = OikomeSession::new(
        n.catalog.clone(),
        n.engines.clone(),
        SessionConfig::default(),
    );
    owner
        .add_source(
            "old-photos",
            oikome_connect::SourceConfig::PhotosFolder {
                path: photos.to_string_lossy().into_owned(),
            },
            None,
            None,
        )
        .await
        .unwrap();
    let copy = root.join(".oikome/sqlite/old-photos.sqlite");
    assert!(copy.is_file());
    n.api
        .share_space(
            &OWNER,
            ShareSpace {
                space: "old-photos".into(),
                did: bob.clone(),
                allow_copy: None,
                expires_in_days: None,
                reach: None,
            },
        )
        .await
        .unwrap();
    n.api.view_folders_in_place().await.unwrap();
    assert!(!copy.exists());
    assert!(n.catalog.get_source("old-photos").is_err());
    assert!(n.catalog.has_space_grant(&bob, "old-photos"));
    let spaces = n.api.list_spaces(&OWNER, Empty {}).await.unwrap();
    let old = spaces.iter().find(|s| s.name == "old-photos").unwrap();
    assert!(old.in_place.is_some());
    assert_eq!(old.shared_with, 1);
    assert!(photos.join("tram.png").is_file());
    // Once only.
    n.api.view_folders_in_place().await.unwrap();

    // A card plugin reads its photos as the photos it read before.
    n.api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("photo-memories".into()),
                path: None,
                collections: vec!["old-photos".into()],
            },
        )
        .await
        .unwrap();
    let rows = n
        .api
        .plugin_read(
            &OWNER,
            PluginRead {
                plugin: "photo-memories".into(),
                space: "old-photos".into(),
                limit: None,
            },
        )
        .await
        .unwrap();
    let batch = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batch.num_rows(), 1);
    let photo = batch
        .column_by_name("photo")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .unwrap();
    assert!(photo.value(0).starts_with(&[0xff, 0xd8]), "its thumbnail");
}

#[tokio::test]
async fn open_with_accepts_only_an_owner_and_an_indexed_local_path() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let trip = root.join("Trip originals");
    std::fs::create_dir_all(&trip).unwrap();
    std::fs::write(trip.join("Portfolio.psd"), [0u8, 1, 2, 3]).unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: trip.to_string_lossy().into_owned(),
                name: None,
            },
        )
        .await
        .unwrap();

    for (path, code) in [
        ("../outside.psd", ErrorCode::Invalid),
        ("missing.psd", ErrorCode::NotFound),
    ] {
        let e = n
            .api
            .open_with(
                &OWNER,
                FileRef {
                    space: "trip-originals".into(),
                    path: path.into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code, "{path}: {e}");
    }
    let e = n
        .api
        .open_with(
            &Caller::Principal { did: did(99) },
            FileRef {
                space: "trip-originals".into(),
                path: "Portfolio.psd".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_folders_files_open_as_tables_read_where_they_are() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());

    // The owner's folder: data files at its top and deeper, a photo with
    // captions beside it, and a file that is not a table.
    let trip = root.join("Trip tables");
    std::fs::create_dir_all(trip.join("2025")).unwrap();
    std::fs::write(
        trip.join("budget.csv"),
        "item,amount\nflights,240\nhotel,410\ntrams,18\n",
    )
    .unwrap();
    std::fs::write(
        trip.join("2025").join("q4.tsv"),
        "day\tspent\n2025-11-01\t33.5\n2025-11-02\t21\n",
    )
    .unwrap();
    std::fs::write(
        trip.join("2025").join("rides.jsonl"),
        "{\"from\": \"Alfama\", \"to\": \"Belém\"}\n{\"from\": \"Porto\", \"to\": \"Ribeira\"}\n",
    )
    .unwrap();
    let mut png = Vec::new();
    image_png(&mut png);
    std::fs::write(trip.join("tram.png"), &png).unwrap();
    std::fs::write(
        trip.join("captions.csv"),
        "file,caption\ntram.png,A tram at dawn\n",
    )
    .unwrap();
    std::fs::write(trip.join("scan.dat"), [0u8, 1, 2, 3]).unwrap();
    let viewed = n
        .api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: trip.to_string_lossy().into_owned(),
                name: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(viewed.name, "trip-tables");

    let rows = |path: &str, offset: u64, limit: u32| ReadFileRows {
        space: "trip-tables".into(),
        path: path.into(),
        limit: Some(limit),
        offset: Some(offset),
    };
    let page = |rows: &Rows| {
        StreamReader::try_new(rows.ipc.as_slice(), None)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let strings = |batch: &arrow_array::RecordBatch, column: usize| -> Vec<String> {
        batch
            .column(column)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap()
            .iter()
            .map(|v| v.unwrap_or_default().to_string())
            .collect()
    };

    // A CSV at the top: every cell the text it is, the header naming the
    // columns.
    let budget = n
        .api
        .read_file_rows(&OWNER, rows("budget.csv", 0, 100))
        .await
        .unwrap();
    assert!(!budget.truncated);
    let read = page(&budget);
    assert_eq!(read[0].num_rows(), 3);
    assert_eq!(read[0].schema().fields().len(), 2);
    assert_eq!(strings(&read[0], 0), ["flights", "hotel", "trams"]);
    assert_eq!(
        strings(&read[0], 1),
        ["240", "410", "18"],
        "money is as written"
    );
    // Paged: a page past the first rows is the rest.
    let tail = n
        .api
        .read_file_rows(&OWNER, rows("budget.csv", 2, 2))
        .await
        .unwrap();
    let read = page(&tail);
    assert_eq!(read[0].num_rows(), 1);
    assert_eq!(strings(&read[0], 0), ["trams"]);

    // A TSV and JSON lines deeper in the folder read the same way.
    let tsv = n
        .api
        .read_file_rows(&OWNER, rows("2025/q4.tsv", 0, 100))
        .await
        .unwrap();
    let read = page(&tsv);
    assert_eq!(strings(&read[0], 1), ["33.5", "21"]);
    let json = n
        .api
        .read_file_rows(&OWNER, rows("2025/rides.jsonl", 0, 100))
        .await
        .unwrap();
    let read = page(&json);
    let from = read[0]
        .schema()
        .fields()
        .iter()
        .position(|f| f.name() == "from")
        .unwrap();
    assert_eq!(strings(&read[0], from), ["Alfama", "Porto"]);

    // The captions beside a photo are a table too when asked.
    let captions = n
        .api
        .read_file_rows(&OWNER, rows("captions.csv", 0, 100))
        .await
        .unwrap();
    assert_eq!(row_count(&captions), 1);

    // The examples' captions, with their titles, places, times and credits.
    n.api
        .add_examples(&OWNER, AddExamples::default())
        .await
        .unwrap();
    let captions = n
        .api
        .read_file_rows(
            &OWNER,
            ReadFileRows {
                space: "lisbon-photos".into(),
                path: "captions.csv".into(),
                limit: Some(100),
                offset: Some(0),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&captions), 6);

    // What is not a table is said so; a path outside the folder, and a
    // file the index doesn't know, are not found.
    for (path, code) in [
        ("scan.dat", ErrorCode::Invalid),
        ("tram.png", ErrorCode::Invalid),
        ("../../etc/passwd", ErrorCode::NotFound),
        ("nope.csv", ErrorCode::NotFound),
    ] {
        let e = n
            .api
            .read_file_rows(&OWNER, rows(path, 0, 100))
            .await
            .unwrap_err();
        assert_eq!(e.code, code, "{path}: {e}");
    }

    // Only the owner reads a folder's files.
    let stranger = Caller::Principal { did: did(9) };
    assert!(n
        .api
        .read_file_rows(&stranger, rows("budget.csv", 0, 100))
        .await
        .is_err());
}

fn image_png(out: &mut Vec<u8>) {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(32, 16, |x, y| {
        image::Rgb([x as u8 * 8, y as u8 * 16, 120])
    }))
    .write_to(&mut std::io::Cursor::new(out), image::ImageFormat::Png)
    .unwrap();
}

/// 3.12: being reached from outside is allowed per share or lending, and
/// ends with it; "Don't ask again" makes the answer the rule.
#[tokio::test]
async fn reaching_from_outside_is_allowed_one_share_at_a_time() {
    let n = node();
    notes(&n).await;
    let bob = did(41);
    let sam = did(42);
    for (who, name) in [(&bob, "Bob"), (&sam, "Sam")] {
        n.api
            .add_contact(
                &OWNER,
                ContactInput {
                    did: who.clone(),
                    name: name.into(),
                },
            )
            .await
            .unwrap();
    }
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert_eq!(network.policy, ReachPolicy::Ask);
    assert!(network.help_contacts);
    assert!(network.open_for.is_empty());
    // This process runs no peer-to-peer layer: only on this network.
    assert!(!network.available);
    assert_eq!(network.reachable, Reachable::LocalOnly);

    let share_with = |who: String, reach: Option<ReachChoice>| ShareSpace {
        space: "notes".into(),
        did: who,
        allow_copy: None,
        expires_in_days: None,
        reach,
    };
    // Allowed for Bob's share alone; Sam's denied.
    let bobs = n
        .api
        .share_space(
            &OWNER,
            share_with(
                bob.clone(),
                Some(ReachChoice {
                    allow: true,
                    remember: None,
                }),
            ),
        )
        .await
        .unwrap();
    let sams = n
        .api
        .share_space(
            &OWNER,
            share_with(
                sam.clone(),
                Some(ReachChoice {
                    allow: false,
                    remember: Some(false),
                }),
            ),
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert_eq!(network.policy, ReachPolicy::Ask);
    assert_eq!(
        network.open_for,
        vec![OpenFor {
            kind: OpenForKind::Share,
            id: bobs.id.clone(),
            name: "Bob".into(),
            space: Some("notes".into()),
        }]
    );
    let shares = n
        .api
        .list_shares(&OWNER, ListShares::default())
        .await
        .unwrap();
    let outside = |id: &str| shares.iter().find(|s| s.id == id).unwrap().reach_outside;
    assert!(outside(&bobs.id));
    assert!(!outside(&sams.id));

    // Stopping Bob's share ends what was allowed for it.
    n.api
        .revoke_share(
            &OWNER,
            ShareRef {
                id: bobs.id.clone(),
            },
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert!(network.open_for.is_empty());
    // A new share with Bob is asked about again.
    let again = share(&n, &bob, None).await;
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert!(network.open_for.is_empty());

    // "Don't ask again" with Allow: every share, now and later.
    n.api
        .share_space(
            &OWNER,
            share_with(
                bob.clone(),
                Some(ReachChoice {
                    allow: true,
                    remember: Some(true),
                }),
            ),
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert_eq!(network.policy, ReachPolicy::Always);
    assert!(network.open_for.iter().any(|o| o.id == again.id));
    assert!(network.open_for.iter().any(|o| o.id == sams.id));

    // Settings › Network changes it back, or to never.
    let network = n
        .api
        .set_network(
            &OWNER,
            SetNetwork {
                policy: Some(ReachPolicy::Never),
                help_contacts: Some(false),
            },
        )
        .await
        .unwrap();
    assert_eq!(network.policy, ReachPolicy::Never);
    assert!(!network.help_contacts);
    assert!(network.open_for.is_empty());

    // Lending: allowed for the people lent to, while lent.
    n.api
        .set_network(
            &OWNER,
            SetNetwork {
                policy: Some(ReachPolicy::Ask),
                help_contacts: None,
            },
        )
        .await
        .unwrap();
    n.api
        .set_lending(
            &OWNER,
            SetLending {
                on: true,
                people: vec![sam.clone()],
                concurrent: 1,
                per_hour: 10,
                reach: Some(ReachChoice {
                    allow: true,
                    remember: None,
                }),
            },
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert_eq!(
        network.open_for,
        vec![OpenFor {
            kind: OpenForKind::Lend,
            id: sam.clone(),
            name: "Sam".into(),
            space: None,
        }]
    );
    n.api
        .set_lending(
            &OWNER,
            SetLending {
                on: false,
                people: vec![sam.clone()],
                concurrent: 1,
                per_hour: 10,
                reach: None,
            },
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert!(network.open_for.is_empty());
    // Lent again, it is asked about again: the answer ended with the pause.
    n.api
        .set_lending(
            &OWNER,
            SetLending {
                on: true,
                people: vec![sam.clone()],
                concurrent: 1,
                per_hour: 10,
                reach: None,
            },
        )
        .await
        .unwrap();
    let network = n.api.network(&OWNER, Empty {}).await.unwrap();
    assert!(network.open_for.is_empty());
}
