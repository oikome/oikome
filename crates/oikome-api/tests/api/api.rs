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

use std::collections::HashMap;
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

/// Wait for the backup or check under way to end.
async fn backup_done(n: &Node) -> BackupInfo {
    for _ in 0..600 {
        let info = n.api.backup_info(&OWNER, Empty {}).await.unwrap();
        if info.running.is_none() {
            return info;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the backup did not end");
}

fn daily(hour: u8) -> BackupSchedule {
    BackupSchedule {
        every: BackupEvery::Daily,
        weekday: None,
        hour,
        minute: 0,
    }
}

fn to_folder(dir: &std::path::Path) -> BackupDestination {
    BackupDestination {
        place: None,
        path: dir.to_string_lossy().into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn backups_are_turned_on_with_places_and_a_time_and_made_then_or_when_asked() {
    let n = node();
    notes(&n).await;
    let info = n.api.backup_info(&OWNER, Empty {}).await.unwrap();
    assert!(!info.enabled && info.places.is_empty() && info.next_at_ms.is_none());
    assert_eq!(
        (info.keep_daily, info.keep_weekly, info.keep_monthly),
        (7, 4, 12)
    );
    let not_yet = n.api.back_up_now(&OWNER, Empty {}).await.unwrap_err();
    assert_eq!(not_yet.reason.as_deref(), Some("backup_no_place"));

    // Not inside the data folder.
    let inside = n
        .api
        .set_backup(
            &OWNER,
            BackupSettings {
                destinations: vec![to_folder(std::path::Path::new(n.catalog.root_path()))],
                schedule: daily(3),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(inside.code, ErrorCode::Invalid);

    // Two places, every day at three; nothing is backed up yet.
    let (disk, nas) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let on = n
        .api
        .set_backup(
            &OWNER,
            BackupSettings {
                destinations: vec![to_folder(disk.path()), to_folder(nas.path())],
                schedule: daily(3),
            },
        )
        .await
        .unwrap();
    assert!(on.enabled);
    assert_eq!(on.places.len(), 2);
    assert_eq!(on.places[0].label, "This computer");
    assert!(on.next_at_ms.is_some());
    n.api.back_up_when_due().await;
    let waiting = n.api.backup_info(&OWNER, Empty {}).await.unwrap();
    assert!(
        waiting.running.is_none(),
        "turning backups on backs up nothing"
    );
    assert!(waiting.places.iter().all(|p| p.last_backup.is_none()));

    // When asked: to both places.
    n.api.back_up_now(&OWNER, Empty {}).await.unwrap();
    let done = backup_done(&n).await;
    for place in &done.places {
        let last = place.last_backup.as_ref().unwrap();
        assert_eq!(last.error, None, "{}", place.path);
        assert!(last.backup.as_ref().unwrap().bytes > 0);
    }
    assert!(disk.path().join("oikome-backup/config").is_file());
    assert!(nas.path().join("oikome-backup/config").is_file());
    let listed = n
        .api
        .list_backups(&OWNER, BackupPlaceRef { place: 1 })
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].this_folder);
    assert!(
        !n.catalog
            .oikome_dir()
            .join("backup")
            .read_dir()
            .unwrap()
            .any(|_| true),
        "the staging copy is gone"
    );

    // A time missed while the node was off is made up when it runs.
    let settings = n.catalog.oikome_dir().join("node.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    json["backup"]["enabled_at_ms"] = 0.into();
    json["backup"]["last_started_ms"] = 0.into();
    std::fs::write(&settings, serde_json::to_vec(&json).unwrap()).unwrap();
    let before = done.places[0].last_backup.as_ref().unwrap().at_ms;
    n.api.back_up_when_due().await;
    let made_up = backup_done(&n).await;
    assert!(made_up.places[0].last_backup.as_ref().unwrap().at_ms > before);

    n.api.check_backup(&OWNER, Empty {}).await.unwrap();
    let checked = backup_done(&n).await;
    assert!(checked
        .places
        .iter()
        .all(|p| p.last_check.as_ref().unwrap().error.is_none()));

    // Off: the places and time stay, for turning them on again.
    let off = n.api.turn_off_backup(&OWNER, Empty {}).await.unwrap();
    assert!(!off.enabled && off.places.len() == 2 && off.next_at_ms.is_none());

    // Another data center's backups are not mixed with these.
    let other = node();
    let refused = other
        .api
        .set_backup(
            &OWNER,
            BackupSettings {
                destinations: vec![to_folder(disk.path())],
                schedule: daily(3),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.reason.as_deref(),
        Some("backup_other_key"),
        "{refused}"
    );
}

async fn rows_in(api: &Api, query: &str) -> usize {
    let read = api
        .run_sql(
            &OWNER,
            RunSql {
                sql: query.into(),
                space: None,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{query}: {e}"));
    let SqlResult::Rows(read) = read else {
        panic!("{query}: no rows");
    };
    row_count(&read)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_data_center_is_turned_back_to_a_backup_of_its_own_folder() {
    let n = node();
    notes(&n).await;
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "ledger".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    sql(
        &n,
        "CREATE TABLE ledger.entries (id BIGINT PRIMARY KEY, amount DOUBLE); \
         INSERT INTO ledger.entries VALUES (1, 12.5), (2, 30)",
    )
    .await;
    let disk = tempfile::tempdir().unwrap();
    n.api
        .set_backup(
            &OWNER,
            BackupSettings {
                destinations: vec![to_folder(disk.path())],
                schedule: daily(3),
            },
        )
        .await
        .unwrap();
    n.api.back_up_now(&OWNER, Empty {}).await.unwrap();
    assert_eq!(
        backup_done(&n).await.places[0]
            .last_backup
            .as_ref()
            .unwrap()
            .error,
        None
    );

    // Afterwards: a row more, and a collection made.
    sql(&n, "INSERT INTO ledger.entries VALUES (3, 7.25)").await;
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "later".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    sql(
        &n,
        "CREATE TABLE later.things (id BIGINT); INSERT INTO later.things VALUES (1)",
    )
    .await;

    // Another data center's backup cannot be restored here, even with its key.
    let other = node();
    let elsewhere = tempfile::tempdir().unwrap();
    other
        .api
        .set_backup(
            &OWNER,
            BackupSettings {
                destinations: vec![to_folder(elsewhere.path())],
                schedule: daily(3),
            },
        )
        .await
        .unwrap();
    other.api.back_up_now(&OWNER, Empty {}).await.unwrap();
    backup_done(&other).await;
    let from_there = |key: Option<String>| RestoreBackup {
        place: None,
        folder: Some(elsewhere.path().to_string_lossy().into_owned()),
        backup: None,
        recovery_key: key,
    };
    let needs_key = n
        .api
        .restore_from_backup(&OWNER, from_there(None))
        .await
        .unwrap_err();
    assert_eq!(needs_key.reason.as_deref(), Some("backup_key_needed"));
    let their_key = other.api.recovery_key(&OWNER, Empty {}).await.unwrap().key;
    let theirs = n
        .api
        .restore_from_backup(&OWNER, from_there(Some(their_key)))
        .await
        .unwrap_err();
    assert_eq!(
        theirs.reason.as_deref(),
        Some("backup_none_here"),
        "{theirs}"
    );

    // This data center's own: made ready, put in place at the next start.
    let restored = n
        .api
        .restore_from_backup(
            &OWNER,
            RestoreBackup {
                place: Some(0),
                folder: None,
                backup: None,
                recovery_key: None,
            },
        )
        .await
        .unwrap();
    assert!(restored.files > 0);
    let root = n._dir.path().to_path_buf();
    let did = n.api.node_info(&OWNER, Empty {}).await.unwrap().did;
    assert!(apply_pending_restore(&root).unwrap());
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), root.to_str().unwrap()).unwrap());
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), test_config()).unwrap());
    let again = Api::new(catalog.clone(), engines);
    assert_eq!(again.node_info(&OWNER, Empty {}).await.unwrap().did, did);
    assert_eq!(rows_in(&again, "SELECT * FROM ledger.entries").await, 2);
    assert_eq!(rows_in(&again, "SELECT * FROM notes.memos").await, 3);
    assert!(catalog.get_space("later").is_err(), "made after the backup");
    assert!(!root.join(".oikome/backup/pending").exists());
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
                place: None,
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

    // A new node installs official packages by default, with no grants.
    let offered = n.api.list_plugins(&OWNER, Empty {}).await.unwrap();
    assert!(offered
        .iter()
        .any(|p| p.id == "spending-by-category" && p.built_in && p.installed));
    for id in [
        "file-import",
        "rustfs",
        "kimi-code",
        "codex",
        "claude-code",
        "flomo",
    ] {
        assert!(offered.iter().any(|p| p.id == id), "{id}");
    }
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
            },
        )
        .await
        .unwrap();
    assert!(spending.installed);
    assert!(spending.collections.is_empty());
    assert!(!spending.home_by_default);
    assert!(n
        .api
        .plugin_read(&OWNER, read(&spending.id, "notes"))
        .await
        .is_err());
    // Adding a card configures its read grant separately from installation.
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: spending.id.clone(),
                collections: vec!["notes".into()],
            },
        )
        .await
        .unwrap();
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
    // The grant does not exist and no rows come back, whether the semantic
    // layer hides the relation or refuses the read.
    let plugin_did = kept[0]["did"].as_str().unwrap().to_string();
    assert!(!n.catalog.has_space_grant(&plugin_did, "diary"));
    assert!(
        matches!(e.code, ErrorCode::NotFound | ErrorCode::PermissionDenied),
        "{e}"
    );
    kept[0]["collections"] = serde_json::json!(["notes"]);
    std::fs::write(&record, serde_json::to_vec(&kept).unwrap()).unwrap();

    // Its page loads the bridge first; nothing outside its folder is served.
    let page = String::from_utf8(n.api.plugin_file(&spending.id, "index.html").unwrap()).unwrap();
    assert!(page.find("/plugins/sdk.js").unwrap() < page.find("main.js").unwrap());
    assert!(n.api.plugin_file(&spending.id, "../plugins.json").is_none());
    assert!(n.api.plugin_file(&spending.id, "main.js").is_some());
    assert!(n.api.plugin_file("photo-memories", "index.html").is_some());
    assert!(n.api.plugin_file("not-installed", "index.html").is_none());

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
            },
        )
        .await
        .unwrap();
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: mine.id.clone(),
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

    // An update that removes the last card also revokes its old grants.
    std::fs::write(
        folder.path().join("oikome-plugin.json"),
        r#"{"id":"my-card","name":"My card","by":"Sam","version":"0.2.0",
            "capabilities":{"mcpClients":[{"id":"example","name":"Example","command":"example {url} {token}"}]}}"#,
    )
    .unwrap();
    let updated = n
        .api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: None,
                path: Some(path.clone()),
            },
        )
        .await
        .unwrap();
    assert!(updated.collections.is_empty());
    assert!(n
        .api
        .plugin_read(&OWNER, read(&mine.id, "diary"))
        .await
        .is_err());
    assert_eq!(
        n.api
            .set_plugin_collections(
                &OWNER,
                PluginCollections {
                    id: mine.id.clone(),
                    collections: vec!["diary".into()],
                },
            )
            .await
            .unwrap_err()
            .reason
            .as_deref(),
        Some("plugin_has_no_card")
    );

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

#[tokio::test]
async fn a_saved_plugin_card_recovers_its_collection_grant() {
    let n = node();
    notes(&n).await;
    n.api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("spending-by-category".into()),
                path: None,
            },
        )
        .await
        .unwrap();
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: "spending-by-category".into(),
                collections: vec!["notes".into()],
            },
        )
        .await
        .unwrap();
    n.api
        .drop_space(
            &OWNER,
            SpaceRef {
                space: "notes".into(),
            },
        )
        .await
        .unwrap();
    // The owner also removes the files the removed collection left behind.
    let location = n.catalog.get_location("notes").unwrap();
    if let oikome_catalog::LocationFormat::IcebergParquet { namespace } = &location.format {
        std::fs::remove_dir_all(std::path::Path::new(n.catalog.root_path()).join(namespace))
            .unwrap();
    }
    n.catalog.forget_location(&location.id).unwrap();
    notes(&n).await;
    n.api.list_plugins(&OWNER, Empty {}).await.unwrap();
    let rows = n
        .api
        .plugin_read(
            &OWNER,
            PluginRead {
                plugin: "spending-by-category".into(),
                space: "notes".into(),
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 3);
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
            },
        )
        .await
        .unwrap();
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: plugin.id.clone(),
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

/// A workbook as Excel writes one: shared strings, a date style, formulas
/// with the values last calculated, and `sheets` as (name, rows of cells
/// already in Excel's XML).
fn workbook(path: &std::path::Path, sheets: &[(&str, &[&str])], strings: &[&str]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut put = |name: &str, body: String| {
        zip.start_file(name, stored).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    };
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>"#,
    );
    let mut listed = String::new();
    let mut rels = String::new();
    for (i, (name, _)) in sheets.iter().enumerate() {
        let n = i + 1;
        types.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
        listed.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="{rel}/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }
    let n = sheets.len();
    rels.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="{rel}/styles" Target="styles.xml"/><Relationship Id="rId{}" Type="{rel}/sharedStrings" Target="sharedStrings.xml"/>"#,
        n + 1,
        n + 2
    ));
    put("[Content_Types].xml", format!("{types}</Types>"));
    put(
        "_rels/.rels",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        ),
    );
    put(
        "xl/workbook.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="{main}" xmlns:r="{rel}"><sheets>{listed}</sheets></workbook>"#
        ),
    );
    put(
        "xl/_rels/workbook.xml.rels",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
        ),
    );
    // Style 1 is Excel's short date (number format 14).
    put(
        "xl/styles.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="{main}"><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>"#
        ),
    );
    let shared: String = strings
        .iter()
        .map(|t| format!("<si><t>{t}</t></si>"))
        .collect();
    put(
        "xl/sharedStrings.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><sst xmlns="{main}" count="{0}" uniqueCount="{0}">{shared}</sst>"#,
            strings.len()
        ),
    );
    for (i, (_, rows)) in sheets.iter().enumerate() {
        let rows: String = rows
            .iter()
            .enumerate()
            .map(|(r, cells)| format!(r#"<row r="{}">{cells}</row>"#, r + 1))
            .collect();
        put(
            &format!("xl/worksheets/sheet{}.xml", i + 1),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="{main}"><sheetData>{rows}</sheetData></worksheet>"#
            ),
        );
    }
    zip.finish().unwrap();
}

#[tokio::test]
async fn a_spreadsheet_s_sheets_become_analysis_tables() {
    let n = node();
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("ledger");
    std::fs::create_dir_all(&root).unwrap();
    let book = root.join("Budget.xlsx");
    // Shared strings: 0 Date, 1 Category, 2 Amount, 3 With tip, 4 Groceries,
    // 5 Books.
    let strings = [
        "Date",
        "Category",
        "Amount",
        "With tip",
        "Groceries",
        "Books",
    ];
    let spending_cells: &[&str] = &[
        r#"<c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c><c r="D1" t="s"><v>3</v></c>"#,
        // 2026-01-01, 2026-01-02 and 2026-02-03 as Excel's day numbers.
        r#"<c r="A2" s="1"><v>46023</v></c><c r="B2" t="s"><v>4</v></c><c r="C2"><v>12.5</v></c><c r="D2"><f>C2*1.1</f><v>13.75</v></c>"#,
        r#"<c r="A3" s="1"><v>46024</v></c><c r="B3" t="s"><v>5</v></c><c r="C3"><v>30</v></c><c r="D3"><f>C3*1.1</f><v>33</v></c>"#,
        r#"<c r="A4" s="1"><v>46056</v></c><c r="B4" t="s"><v>4</v></c><c r="C4"><v>7.25</v></c><c r="D4"><f>C4*1.1</f><v>7.975</v></c>"#,
    ];
    // Text written in the cell itself, as other programs write it.
    let rates: &[&str] = &[
        r#"<c r="A1" t="inlineStr"><is><t>Currency</t></is></c><c r="B1" t="inlineStr"><is><t>Rate</t></is></c>"#,
        r#"<c r="A2" t="inlineStr"><is><t>EUR</t></is></c><c r="B2"><v>1</v></c>"#,
        r#"<c r="A3" t="inlineStr"><is><t>USD</t></is></c><c r="B3"><v>1.08</v></c>"#,
    ];
    workbook(
        &book,
        &[
            ("Spending", spending_cells),
            ("Notes", &[]),
            ("Rates", rates),
        ],
        &strings,
    );
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("ledger".into()),
            },
        )
        .await
        .unwrap();
    let ledger = || SpaceRef {
        space: "ledger".into(),
    };

    // Each sheet with something in it can become a table; the empty one not.
    let found = n.api.file_analysis(&OWNER, ledger()).await.unwrap();
    let sources: Vec<_> = found
        .sources
        .iter()
        .map(|s| (s.name.as_str(), s.sheet.as_deref()))
        .collect();
    assert_eq!(
        sources,
        [
            ("budget_spending", Some("Spending")),
            ("budget_rates", Some("Rates"))
        ]
    );
    let made = n
        .api
        .build_analysis(
            &OWNER,
            BuildAnalysis {
                space: "ledger".into(),
                tables: vec!["budget_spending".into(), "budget_rates".into()],
            },
        )
        .await
        .unwrap();
    let spending = made
        .tables
        .iter()
        .find(|t| t.name == "budget_spending")
        .unwrap();
    assert_eq!(spending.rows, 3);
    assert_eq!(spending.files, ["Budget.xlsx"]);
    assert_eq!(spending.problem, None);
    let date = spending.fields.iter().find(|f| f.name == "Date").unwrap();
    assert_eq!(date.role, FieldRole::Time, "a date column is a time");

    // Dates are dates and formulas give the values Excel saved.
    let read = sql(
        &n,
        "SELECT CAST(min(\"Date\") AS VARCHAR), CAST(round(sum(\"With tip\"), 3) AS VARCHAR), \
         CAST(count(DISTINCT \"_source_file\") AS VARCHAR) FROM ledger.budget_spending",
    )
    .await;
    let SqlResult::Rows(rows) = read else {
        panic!("expected rows, got {read:?}");
    };
    let batch = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let text = |c: usize| {
        batch
            .column(c)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap()
            .value(0)
            .to_string()
    };
    assert_eq!(text(0), "2026-01-01");
    assert_eq!(text(1), "54.725");
    assert_eq!(text(2), "1");
    let read = sql(
        &n,
        "SELECT CAST(sum(\"Rate\") AS VARCHAR) FROM ledger.budget_rates WHERE \"Currency\" = 'USD'",
    )
    .await;
    let SqlResult::Rows(rows) = read else {
        panic!("expected rows, got {read:?}");
    };
    let usd = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let usd = usd
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap()
        .value(0)
        .to_string();
    assert_eq!(usd, "1.08");

    // Saved with one sheet only, it is one table named after the file, and
    // the sheets' tables go.
    workbook(&book, &[("Spending", spending_cells)], &strings);
    n.api.refresh_space(&OWNER, ledger()).await.unwrap();
    let again = n
        .api
        .build_analysis(
            &OWNER,
            BuildAnalysis {
                space: "ledger".into(),
                tables: vec!["budget".into()],
            },
        )
        .await
        .unwrap();
    let names: Vec<_> = again.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["budget"]);
    assert_eq!(again.tables[0].rows, 3);
    assert_eq!(again.tables[0].sheet.as_deref(), Some("Spending"));
}

#[tokio::test]
async fn a_csv_source_becomes_its_file_in_a_file_collection() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    std::fs::create_dir_all(root.join("bank")).unwrap();
    std::fs::create_dir_all(root.join("household")).unwrap();
    let statement = root.join("bank/statement.csv");
    std::fs::write(
        &statement,
        "day,amount,category\n2026-01-01,12.50,Groceries\n2026-01-02,18.25,Books\n",
    )
    .unwrap();
    std::fs::write(
        root.join("household/statement.csv"),
        "day,amount\n2025-12-01,40\n",
    )
    .unwrap();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "finance".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    let source = |path: &std::path::Path, name: &str| NewSource {
        kind: SourceKind::Csv,
        path: path.to_string_lossy().into_owned(),
        space: Some("finance".into()),
        table: Some(name.replace('-', "_")),
        name: Some(name.into()),
    };
    n.api
        .add_source(&OWNER, source(&statement, "bank-import"))
        .await
        .unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.join("household").to_string_lossy().into_owned(),
                name: Some("household".into()),
            },
        )
        .await
        .unwrap();
    let convert = |space: &str, source: &str| ConvertSource {
        source: source.into(),
        space: space.into(),
    };

    // Not into the collection it writes.
    let own = n
        .api
        .convert_source_to_file(&OWNER, convert("finance", "bank-import"))
        .await;
    assert_eq!(own.unwrap_err().code, ErrorCode::Invalid);

    // Into a file collection: at its top under its own name, numbered past
    // the collection's own statement.csv, with an analysis table.
    let converted = n
        .api
        .convert_source_to_file(&OWNER, convert("household", "bank-import"))
        .await
        .unwrap();
    assert_eq!(converted.space.name, "household");
    assert_eq!(converted.path, "statement 2.csv");
    assert_eq!(converted.table.as_deref(), Some("statement_2"));
    assert_eq!(converted.previous, "finance");
    let rows = n
        .api
        .read_file_rows(
            &OWNER,
            ReadFileRows {
                space: "household".into(),
                path: "statement 2.csv".into(),
                limit: Some(10),
                offset: Some(0),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 2);
    let analysis = n
        .api
        .file_analysis(
            &OWNER,
            SpaceRef {
                space: "household".into(),
            },
        )
        .await
        .unwrap();
    let made = analysis
        .tables
        .iter()
        .find(|table| table.name == "statement_2")
        .unwrap();
    assert_eq!(made.rows, 2);
    // Only its own: the collection's other file waits for the owner.
    assert!(analysis
        .tables
        .iter()
        .all(|table| table.name != "statement"));
    assert!(analysis
        .sources
        .iter()
        .any(|source| source.name == "statement"));

    // The source stopped; the collection it wrote stays with its table.
    let sources = n.api.list_sources(&OWNER, Empty {}).await.unwrap();
    assert!(sources.iter().all(|s| s.name != "bank-import"));
    let tables = n
        .api
        .list_tables(
            &OWNER,
            SpaceRef {
                space: "finance".into(),
            },
        )
        .await
        .unwrap();
    assert!(tables.iter().any(|table| table.name == "bank_import"));
    let again = n
        .api
        .convert_source_to_file(&OWNER, convert("household", "bank-import"))
        .await;
    assert!(again.is_err(), "the source is gone");

    // A file does not become a collection: one must be chosen.
    let card = root.join("bank/card.csv");
    std::fs::write(&card, "day,amount\n2026-02-01,9.90\n").unwrap();
    n.api
        .add_source(&OWNER, source(&card, "card-import"))
        .await
        .unwrap();
    let none = n
        .api
        .convert_source_to_file(&OWNER, convert(" ", "card-import"))
        .await
        .unwrap_err();
    assert_eq!(none.code, ErrorCode::Invalid);
    assert!(n.catalog.get_source("card-import").is_ok());
}

#[tokio::test]
async fn a_csv_source_targets_an_existing_collection_and_keeps_legacy_destinations() {
    let n = node();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "finance".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let csv = root.join("bank.csv");
    std::fs::write(
        &csv,
        "id,day,amount\n1,2026-01-01,12.50\n2,2026-01-02,18.25\n",
    )
    .unwrap();
    let request = |space: Option<&str>, table: Option<&str>, name: Option<&str>| NewSource {
        kind: SourceKind::Csv,
        path: csv.to_string_lossy().into_owned(),
        space: space.map(Into::into),
        table: table.map(Into::into),
        name: name.map(Into::into),
    };
    assert!(n
        .api
        .add_source(&OWNER, request(None, Some("entries"), Some("bank-import")))
        .await
        .is_err());
    assert!(n.catalog.get_space("finance").is_ok());
    assert!(n.catalog.get_space("bank-import").is_err());

    let source = n
        .api
        .add_source(
            &OWNER,
            request(Some("finance"), Some("entries"), Some("bank-import")),
        )
        .await
        .unwrap();
    assert_eq!(source.name, "bank-import");
    assert_eq!(source.space, "finance");
    assert_eq!(source.table.as_deref(), Some("entries"));
    assert!(n.catalog.get_space("bank-import").is_err());
    let tables = n
        .api
        .list_tables(
            &OWNER,
            SpaceRef {
                space: "finance".into(),
            },
        )
        .await
        .unwrap();
    let entries = tables.iter().find(|table| table.name == "entries").unwrap();
    assert_eq!(entries.rows, Some(2));
    let second = n
        .api
        .add_source(
            &OWNER,
            request(Some("finance"), Some("other"), Some("second-import")),
        )
        .await
        .unwrap();
    assert_eq!(second.space, "finance");
    assert_eq!(second.table.as_deref(), Some("other"));
    assert_eq!(second.plugin, None);
    assert_eq!(second.capability, None);
    let registered = n.catalog.get_source("bank-import").unwrap();
    assert_eq!(registered.target.table.as_deref(), Some("entries"));
    assert!(n
        .api
        .add_source(
            &OWNER,
            request(Some("finance"), Some("entries"), Some("another-import")),
        )
        .await
        .is_err());

    let settings =
        serde_json::from_value(n.catalog.get_source("bank-import").unwrap().config.clone())
            .unwrap();
    oikome_node::OikomeSession::new(
        n.catalog.clone(),
        n.engines.clone(),
        oikome_node::SessionConfig::default(),
    )
    .add_source("legacy", settings, None, None)
    .await
    .unwrap();
    let legacy = n
        .api
        .list_sources(&OWNER, Empty {})
        .await
        .unwrap()
        .into_iter()
        .find(|source| source.name == "legacy")
        .unwrap();
    assert_eq!(legacy.space, "legacy");
    assert!(n.catalog.get_source("legacy").unwrap().space.is_none());
}

#[tokio::test]
async fn a_plugin_source_records_its_collection_table_and_capability() {
    let n = node();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "library".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    n.api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("flomo".into()),
                path: None,
            },
        )
        .await
        .unwrap();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let html = root.join("memos.html");
    std::fs::write(
        &html,
        "<html><body><div class=\"memo\"><div class=\"time\">2026-01-01 10:00:00</div><div class=\"content\"><p>hello</p></div></div></body></html>",
    )
    .unwrap();
    let source = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::FlomoExport,
                path: html.to_string_lossy().into_owned(),
                space: Some("library".into()),
                table: None,
                name: Some("flomo-export".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(source.space, "library");
    assert_eq!(source.table.as_deref(), Some("memos"));
    assert_eq!(source.plugin.as_deref(), Some("flomo"));
    assert_eq!(source.capability.as_deref(), Some("flomo-export"));
    assert!(n.catalog.get_space("flomo-export").is_err());
    let registered = n.catalog.get_source("flomo-export").unwrap();
    assert_eq!(registered.target.table.as_deref(), Some("memos"));
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
                space: Some("spending".into()),
                table: None,
                name: None,
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
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "journal".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    let source = n
        .api
        .add_source(
            &OWNER,
            NewSource {
                kind: SourceKind::Csv,
                path: file.to_string_lossy().into_owned(),
                space: Some("journal".into()),
                table: None,
                name: None,
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
                plugin: None,
                client: None,
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
        plugin: None,
        client: None,
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

#[tokio::test]
async fn imported_files_become_a_new_local_folder_without_overwriting() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let request = |name: &str, file: &str, data: &str| ImportFiles {
        name: Some(name.into()),
        space: None,
        folder: None,
        files: vec![ImportFile {
            name: file.into(),
            data_base64: data.into(),
        }],
    };

    let e = n
        .api
        .import_files(
            &Caller::Principal { did: did(8) },
            request("imported", "note.md", "SGVsbG8="),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    for file in ["../escape", "a/b", ".", ""] {
        let e = n
            .api
            .import_files(&OWNER, request("imported", file, "SGVsbG8="))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid, "{file}: {e}");
    }
    let e = n
        .api
        .import_files(&OWNER, request("imported", "note.md", "not base64"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert!(!root.join("Imports").join("imported").exists());

    let imported = n
        .api
        .import_files(
            &OWNER,
            ImportFiles {
                name: Some("imported".into()),
                space: None,
                folder: None,
                files: vec![
                    ImportFile {
                        name: "note.md".into(),
                        data_base64: "SGVsbG8sIGltcG9ydGVkLg==".into(),
                    },
                    ImportFile {
                        name: "rows.csv".into(),
                        data_base64: "aSxiCjEsMgo=".into(),
                    },
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(imported.name, "imported");
    assert!(imported.readonly);
    let place = imported.in_place.unwrap();
    assert_eq!(place.place, None);
    assert_eq!(place.files, 2);
    assert_eq!(
        std::path::PathBuf::from(place.path),
        std::fs::canonicalize(root.join("Imports").join("imported")).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("Imports/imported/note.md")).unwrap(),
        "Hello, imported."
    );

    let e = n
        .api
        .import_files(&OWNER, request("imported", "note.md", "UkVQTEFDRUQ="))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(
        std::fs::read_to_string(root.join("Imports/imported/note.md")).unwrap(),
        "Hello, imported."
    );
}

#[tokio::test]
async fn imports_stay_in_the_current_folder_and_convert_only_an_empty_collection() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let upload = |space: &str, folder: Option<&str>, file: &str| ImportFiles {
        name: None,
        space: Some(space.into()),
        folder: folder.map(Into::into),
        files: vec![ImportFile {
            name: file.into(),
            data_base64: "aW1wb3J0ZWQ=".into(),
        }],
    };
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "T".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    let converted = n
        .api
        .import_files(&OWNER, upload("T", None, "one.md"))
        .await
        .unwrap();
    assert_eq!(converted.name, "T");
    assert_eq!(converted.in_place.as_ref().unwrap().files, 1);
    assert_eq!(
        std::fs::read_to_string(root.join("Imports/T/one.md")).unwrap(),
        "imported"
    );

    n.api
        .import_files(&OWNER, upload("T", None, "two.md"))
        .await
        .unwrap();
    std::fs::create_dir(root.join("Imports/T/Nested")).unwrap();
    n.api
        .import_files(&OWNER, upload("T", Some("Nested"), "three.md"))
        .await
        .unwrap();
    let nested = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "T".into(),
                folder: Some("Nested".into()),
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
    assert_eq!(nested.files[0].name, "three.md");
    assert!(n
        .api
        .import_files(&OWNER, upload("T", Some("../outside"), "escape.md"))
        .await
        .is_err());
    assert!(!root.join("Imports/outside").exists());

    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "occupied".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    sql(&n, "CREATE TABLE occupied.t (id BIGINT)").await;
    let e = n
        .api
        .import_files(&OWNER, upload("occupied", None, "no.md"))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(n.catalog.get_space("occupied").unwrap().location == "occupied");
}

#[tokio::test]
async fn selected_originals_form_a_multi_source_collection_without_exposing_siblings() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let one = root.join("one");
    let two = root.join("two");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    std::fs::write(one.join("keep.md"), "first original").unwrap();
    std::fs::write(one.join("sibling.md"), "not selected").unwrap();
    std::fs::write(one.join(".hidden.md"), "hidden").unwrap();
    std::fs::write(two.join("other.txt"), "second original").unwrap();
    std::fs::write(two.join("sibling.txt"), "not selected either").unwrap();
    let private = root.join(".oikome/index");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("private.md"), "private").unwrap();
    let file = |path: &std::path::Path| vec![path.to_string_lossy().into_owned()];

    // A collection is made first, as a folder is; files then join it.
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "references".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    let add = |files: Vec<String>| AddInPlaceFiles {
        space: "references".into(),
        files,
    };
    for files in [
        vec![],
        vec!["relative.md".into()],
        file(&one),
        file(&one.join(".hidden.md")),
        file(&private.join("private.md")),
    ] {
        let e = n
            .api
            .add_in_place_files(&OWNER, add(files))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid);
    }
    #[cfg(unix)]
    {
        let link = root.join("link.md");
        std::os::unix::fs::symlink(one.join("keep.md"), &link).unwrap();
        assert!(n
            .api
            .add_in_place_files(&OWNER, add(file(&link)))
            .await
            .is_err());
    }
    let e = n
        .api
        .add_in_place_files(
            &Caller::Principal { did: did(8) },
            add(file(&one.join("keep.md"))),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);

    let viewed = n
        .api
        .add_in_place_files(&OWNER, add(file(&one.join("keep.md"))))
        .await
        .unwrap();
    assert_eq!(viewed.in_place.as_ref().unwrap().files, 1);
    let read = |path: &str| FileRef {
        space: "references".into(),
        path: path.into(),
    };
    assert_eq!(
        n.api
            .read_file(&OWNER, read("keep.md"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("first original")
    );
    assert!(n.api.read_file(&OWNER, read("sibling.md")).await.is_err());

    let expanded = n
        .api
        .add_in_place_files(
            &OWNER,
            AddInPlaceFiles {
                space: "references".into(),
                files: file(&two.join("other.txt")),
            },
        )
        .await
        .unwrap();
    assert_eq!(expanded.in_place.as_ref().unwrap().files, 2);
    assert_eq!(
        n.api
            .read_file(&OWNER, read("one/keep.md"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("first original")
    );
    assert_eq!(
        n.api
            .read_file(&OWNER, read("two/other.txt"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("second original")
    );
    for absent in ["one/sibling.md", "two/sibling.txt", "sibling.md"] {
        assert!(n.api.read_file(&OWNER, read(absent)).await.is_err());
    }
    let count = |result: SqlResult| match result {
        SqlResult::Rows(rows) => row_count(&rows),
        other => panic!("expected rows, got {other:?}"),
    };
    assert_eq!(
        count(sql(&n, "SELECT path FROM \"references\".files").await),
        2
    );

    n.api
        .import_files(
            &OWNER,
            ImportFiles {
                name: None,
                space: Some("references".into()),
                folder: None,
                files: vec![ImportFile {
                    name: "copied.md".into(),
                    data_base64: "Y29waWVk".into(),
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        n.api
            .read_file(&OWNER, read("copied.md"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("copied")
    );
    assert_eq!(
        count(sql(&n, "SELECT path FROM \"references\".files").await),
        3
    );
    assert!(!root.join("Imports/references").exists());

    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "empty-reference".into(),
                kind: None,
                storage: None,
            },
        )
        .await
        .unwrap();
    let converted = n
        .api
        .add_in_place_files(
            &OWNER,
            AddInPlaceFiles {
                space: "empty-reference".into(),
                files: file(&two.join("other.txt")),
            },
        )
        .await
        .unwrap();
    assert_eq!(converted.name, "empty-reference");
    assert_eq!(converted.in_place.as_ref().unwrap().files, 1);
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
    let storages = n.api.list_storages(&OWNER, Empty {}).await.unwrap();
    assert_eq!(
        storages[0].spaces, 1,
        "a folder viewed here counts on this computer"
    );
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
    assert_eq!(note.content.format, "markdown");
    assert_eq!(note.content.media_type.as_deref(), Some("text/markdown"));
    assert_eq!(
        note.content.views,
        vec![ContentView::Read, ContentView::Source]
    );
    assert_eq!(note.content.source, "in_place");
    assert!(note.content.version.starts_with("blake3:"));
    assert!(note.content.capabilities.open_default && note.content.capabilities.reveal);
    assert!(note.content.capabilities.edit && note.content.capabilities.save_copy);
    assert_eq!(first.files[0].content.views, vec![ContentView::Image]);
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
            },
        )
        .await
        .unwrap();
    n.api
        .set_plugin_collections(
            &OWNER,
            PluginCollections {
                id: "photo-memories".into(),
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
async fn a_whole_folder_joins_an_existing_file_collection() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("one.md"), "first note").unwrap();
    std::fs::write(second.join("two.md"), "second note").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: first.to_string_lossy().into_owned(),
                name: Some("notes".into()),
            },
        )
        .await
        .unwrap();
    n.api
        .add_in_place_folder(
            &OWNER,
            AddInPlaceFolder {
                space: "notes".into(),
                path: second.to_string_lossy().into_owned(),
                place: None,
            },
        )
        .await
        .unwrap();
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "notes".into(),
                folder: None,
                offset: None,
                limit: None,
                deep: Some(true),
                kind: None,
                sort: None,
                descending: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(listed.total, 2);
    let second_note = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "notes".into(),
                path: "second/two.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(second_note.text.as_deref(), Some("second note"));
    let duplicate = n
        .api
        .add_in_place_folder(
            &OWNER,
            AddInPlaceFolder {
                space: "notes".into(),
                path: second.to_string_lossy().into_owned(),
                place: None,
            },
        )
        .await;
    assert_eq!(duplicate.unwrap_err().code, ErrorCode::Conflict);
}

#[tokio::test]
async fn a_folder_on_a_storage_joins_a_collection_by_reference() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let notes = root.join("notes");
    std::fs::create_dir_all(notes.join("trips")).unwrap();
    std::fs::write(notes.join("one.md"), "own note").unwrap();
    std::fs::write(notes.join("trips/plan.md"), "own plan").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: notes.to_string_lossy().into_owned(),
                name: Some("notes".into()),
            },
        )
        .await
        .unwrap();

    // A folder on another place, with a table in it.
    let picked = tempfile::tempdir().unwrap();
    let archive = n
        .api
        .add_storage(
            &OWNER,
            NewStorage {
                label: "Archive".into(),
                kind: StorageType::Folder,
                address: picked.path().to_str().unwrap().into(),
                username: None,
                password: None,
                bucket: None,
                encrypted: None,
                make_default: Some(false),
            },
        )
        .await
        .unwrap();
    let on_archive = std::path::PathBuf::from(&archive.place);
    std::fs::create_dir_all(on_archive.join("2025/trips")).unwrap();
    std::fs::write(on_archive.join("2025/trips/lisbon.md"), "Lisbon, slowly").unwrap();
    std::fs::write(
        on_archive.join("2025/trips/costs.csv"),
        "day,amount\n2025-05-01,12.5\n2025-05-02,30\n",
    )
    .unwrap();
    let add = |path: &str| AddInPlaceFolder {
        space: "notes".into(),
        path: path.into(),
        place: archive.name.clone(),
    };
    n.api
        .add_in_place_folder(&OWNER, add("2025/trips"))
        .await
        .unwrap();

    // It sits at the top under its own name, numbered past the
    // collection's own "trips"; the collection's files keep their paths.
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "notes".into(),
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
    let folders: Vec<_> = listed.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(folders, ["trips", "trips 2"], "{folders:?}");
    let own = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "notes".into(),
                path: "trips/plan.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(own.text.as_deref(), Some("own plan"));

    // Its files are read where they are, and not edited from Oikome.
    let added = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "notes".into(),
                path: "trips 2/lisbon.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(added.text.as_deref(), Some("Lisbon, slowly"));
    assert!(!added.content.capabilities.edit);
    assert!(added
        .content
        .edit_unavailable_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("added from another place")));

    // Its tables read like the collection's own.
    let costs = n
        .api
        .read_file_rows(
            &OWNER,
            ReadFileRows {
                space: "notes".into(),
                path: "trips 2/costs.csv".into(),
                limit: Some(10),
                offset: Some(0),
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&costs), 2);

    // The same folder twice is refused; the place's root is named after it.
    let twice = n.api.add_in_place_folder(&OWNER, add("2025/trips")).await;
    assert_eq!(twice.unwrap_err().code, ErrorCode::Conflict);
    std::fs::write(on_archive.join("readme.md"), "the archive").unwrap();
    n.api.add_in_place_folder(&OWNER, add("")).await.unwrap();
    let whole = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "notes".into(),
                path: "archive/readme.md".into(),
            },
        )
        .await;
    assert!(whole.is_ok(), "{whole:?}");

    // A folder of a storage needs a file collection.
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "ledger".into(),
                kind: Some(SpaceKind::Analytical),
                storage: None,
            },
        )
        .await
        .unwrap();
    let wrong = n
        .api
        .add_in_place_folder(
            &OWNER,
            AddInPlaceFolder {
                space: "ledger".into(),
                path: "2025/trips".into(),
                place: archive.name.clone(),
            },
        )
        .await;
    assert_eq!(wrong.unwrap_err().code, ErrorCode::Invalid);
}

#[tokio::test]
async fn an_empty_analytical_collection_accepts_a_folder() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let vault = root.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("trip.md"), "Lisbon, slowly").unwrap();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "file-library".into(),
                kind: Some(SpaceKind::Analytical),
                storage: None,
            },
        )
        .await
        .unwrap();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "source-destination".into(),
                kind: Some(SpaceKind::Analytical),
                storage: None,
            },
        )
        .await
        .unwrap();
    n.catalog
        .add_source_to(
            "separate-source-id",
            "source-destination",
            serde_json::json!({}),
            None,
        )
        .unwrap();
    assert!(n
        .api
        .add_in_place_folder(
            &OWNER,
            AddInPlaceFolder {
                space: "source-destination".into(),
                path: vault.to_string_lossy().into_owned(),
                place: None,
            },
        )
        .await
        .is_err());

    let added = n
        .api
        .add_in_place_folder(
            &OWNER,
            AddInPlaceFolder {
                space: "file-library".into(),
                path: vault.to_string_lossy().into_owned(),
                place: None,
            },
        )
        .await
        .unwrap();
    assert!(added.in_place.is_some());
    let file = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "file-library".into(),
                path: "trip.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(file.text.as_deref(), Some("Lisbon, slowly"));
}

#[tokio::test]
async fn safe_text_editing_preserves_sources_copies_and_confirmation_boundaries() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let vault = root.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let original = vault.join("note.md");
    std::fs::write(&original, b"\xef\xbb\xbf# First\r\nline\r\n").unwrap();
    std::fs::write(vault.join("rows.csv"), "id\n1\n").unwrap();
    std::fs::write(vault.join("settings"), "theme: dark\n").unwrap();
    std::fs::write(vault.join("binary.dat"), [0, 159, 146, 150]).unwrap();
    std::fs::write(
        vault.join("ascii.pdf"),
        "%PDF-1.4\n% plain ASCII is still not text\n",
    )
    .unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: vault.to_string_lossy().into_owned(),
                name: Some("notes".into()),
            },
        )
        .await
        .unwrap();
    let reference = |path: &str| FileRef {
        space: "notes".into(),
        path: path.into(),
    };
    let opened = n.api.read_file(&OWNER, reference("note.md")).await.unwrap();
    assert!(opened.content.capabilities.edit);
    assert!(opened.content.capabilities.save_copy);
    assert!(opened.content.version.starts_with("blake3:"));
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "notes".into(),
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
    let listed_csv = listed
        .files
        .iter()
        .find(|file| file.path == "rows.csv")
        .unwrap();
    assert!(!listed_csv.content.views.contains(&ContentView::Source));
    let csv = n
        .api
        .read_file(&OWNER, reference("rows.csv"))
        .await
        .unwrap();
    assert!(csv.content.capabilities.edit && csv.content.capabilities.save_copy);
    assert!(csv.content.views.contains(&ContentView::Table));
    assert!(csv.content.views.contains(&ContentView::Source));
    assert_eq!(csv.text.as_deref(), Some("id\n1\n"));
    let extensionless = n
        .api
        .read_file(&OWNER, reference("settings"))
        .await
        .unwrap();
    assert!(extensionless.content.capabilities.edit);
    assert!(extensionless.content.views.contains(&ContentView::Source));
    let binary = n
        .api
        .read_file(&OWNER, reference("binary.dat"))
        .await
        .unwrap();
    assert!(!binary.content.capabilities.edit && !binary.content.capabilities.save_copy);
    assert_eq!(binary.text, None);
    let pdf = n
        .api
        .read_file(&OWNER, reference("ascii.pdf"))
        .await
        .unwrap();
    assert!(!pdf.content.capabilities.edit && !pdf.content.capabilities.save_copy);
    assert_eq!(pdf.text, None);
    let csv_copy = n
        .api
        .save_text_file(
            &OWNER,
            SaveTextFile {
                space: "notes".into(),
                path: "rows.csv".into(),
                text: "id,name\n1,Lisbon\n".into(),
                version: csv.content.version,
                target: TextSaveTarget::Copy,
                copy_name: Some("rows copy.csv".into()),
                confirm: false,
                target_version: None,
                remember: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(csv_copy.path, "rows copy.csv");
    assert_eq!(
        std::fs::read_to_string(vault.join("rows copy.csv")).unwrap(),
        "id,name\n1,Lisbon\n"
    );

    let request = |version: String, target, copy_name: &str, confirm, target_version, remember| {
        SaveTextFile {
            space: "notes".into(),
            path: "note.md".into(),
            text: "# Changed\nnext\n".into(),
            version,
            target,
            copy_name: Some(copy_name.into()),
            confirm,
            target_version,
            remember,
        }
    };
    let denied = n
        .api
        .save_text_file(
            &OWNER,
            request(
                opened.content.version.clone(),
                TextSaveTarget::Original,
                "unused.md",
                false,
                None,
                None,
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(
        denied.reason.as_deref(),
        Some("source_confirmation_required")
    );
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"\xef\xbb\xbf# First\r\nline\r\n"
    );

    let saved = n
        .api
        .save_text_file(
            &OWNER,
            request(
                opened.content.version.clone(),
                TextSaveTarget::Original,
                "unused.md",
                true,
                None,
                Some(ConfirmationScope::Directory),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"\xef\xbb\xbf# Changed\r\nnext\r\n"
    );
    assert!(saved.content.version.starts_with("blake3:"));
    let remembered = n
        .api
        .list_save_confirmations(&OWNER, Empty {})
        .await
        .unwrap();
    assert_eq!(remembered.len(), 1);
    assert_eq!(remembered[0].scope, ConfirmationScope::Directory);

    // The node's own save advances the remembered version, so a repeated save
    // needs no second prompt. An outside change still conflicts.
    let saved_again = n
        .api
        .save_text_file(
            &OWNER,
            request(
                saved.content.version.clone(),
                TextSaveTarget::Original,
                "unused.md",
                false,
                None,
                None,
            ),
        )
        .await
        .unwrap();
    std::fs::write(&original, "changed elsewhere\n").unwrap();
    let stale = n
        .api
        .save_text_file(
            &OWNER,
            request(
                saved_again.content.version.clone(),
                TextSaveTarget::Original,
                "unused.md",
                true,
                None,
                None,
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.reason.as_deref(), Some("version_conflict"));
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "changed elsewhere\n"
    );

    let current = n.api.read_file(&OWNER, reference("note.md")).await.unwrap();
    let copied = n
        .api
        .save_text_file(
            &OWNER,
            request(
                current.content.version.clone(),
                TextSaveTarget::Copy,
                "note copy.md",
                false,
                None,
                None,
            ),
        )
        .await
        .unwrap();
    assert_eq!(copied.path, "note copy.md");
    assert_eq!(
        std::fs::read_to_string(vault.join("note copy.md")).unwrap(),
        "# Changed\nnext\n"
    );
    let overwrite = n
        .api
        .save_text_file(
            &OWNER,
            request(
                current.content.version.clone(),
                TextSaveTarget::Copy,
                "note copy.md",
                false,
                None,
                None,
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(
        overwrite.reason.as_deref(),
        Some("overwrite_confirmation_required")
    );
    let target_version = overwrite.params.get("targetVersion").unwrap().clone();
    assert!(!target_version.is_empty());
    let wrong_target = n
        .api
        .save_text_file(
            &OWNER,
            request(
                current.content.version.clone(),
                TextSaveTarget::Copy,
                "note copy.md",
                true,
                Some("stale".into()),
                None,
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(wrong_target.reason.as_deref(), Some("version_conflict"));
    assert_eq!(
        std::fs::read_to_string(vault.join("note copy.md")).unwrap(),
        "# Changed\nnext\n"
    );
    n.api
        .save_text_file(
            &OWNER,
            request(
                current.content.version.clone(),
                TextSaveTarget::Copy,
                "note copy.md",
                true,
                Some(target_version),
                Some(ConfirmationScope::Identity),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        n.api
            .list_save_confirmations(&OWNER, Empty {})
            .await
            .unwrap()
            .len(),
        2
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o444)).unwrap();
        let read_only = n.api.read_file(&OWNER, reference("note.md")).await.unwrap();
        assert!(!read_only.content.capabilities.edit);
        assert!(read_only.content.capabilities.save_copy);
        let refused = n
            .api
            .save_text_file(
                &OWNER,
                request(
                    read_only.content.version.clone(),
                    TextSaveTarget::Original,
                    "unused.md",
                    true,
                    None,
                    None,
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, ErrorCode::PermissionDenied);
        n.api
            .save_text_file(
                &OWNER,
                request(
                    read_only.content.version.clone(),
                    TextSaveTarget::Copy,
                    "read-only copy.md",
                    false,
                    None,
                    None,
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&original).unwrap(),
            "changed elsewhere\n"
        );
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let source_preference = n
        .api
        .list_save_confirmations(&OWNER, Empty {})
        .await
        .unwrap()[0]
        .id
        .clone();
    let left = n
        .api
        .revoke_save_confirmation(
            &OWNER,
            RevokeSaveConfirmation {
                id: source_preference,
            },
        )
        .await
        .unwrap();
    assert_eq!(left.len(), 1);
    let principal = Caller::Principal { did: did(9) };
    assert!(n
        .api
        .save_text_file(
            &principal,
            request(
                current.content.version.clone(),
                TextSaveTarget::Copy,
                "principal.md",
                false,
                None,
                None,
            ),
        )
        .await
        .is_err());
}

#[tokio::test]
async fn media_files_have_native_views_and_ranged_originals() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let media = root.join("Media originals");
    std::fs::create_dir_all(&media).unwrap();
    let bytes = (0..=255).collect::<Vec<u8>>();
    for name in ["voice.wav", "clip.webm", "vector.svg"] {
        std::fs::write(media.join(name), &bytes).unwrap();
    }
    std::fs::write(media.join("notes.txt"), "not media").unwrap();
    std::fs::write(media.join("document.pdf"), &bytes).unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: media.to_string_lossy().into_owned(),
                name: None,
            },
        )
        .await
        .unwrap();

    for (path, media_type, view) in [
        ("voice.wav", "audio/wav", ContentView::Audio),
        ("clip.webm", "video/webm", ContentView::Video),
        ("vector.svg", "image/svg+xml", ContentView::Image),
        ("document.pdf", "application/pdf", ContentView::Pdf),
    ] {
        let reference = FileRef {
            space: "media-originals".into(),
            path: path.into(),
        };
        let file = n.api.read_file(&OWNER, reference.clone()).await.unwrap();
        assert_eq!(file.kind, FileKind::Other);
        assert_eq!(file.content.media_type.as_deref(), Some(media_type));
        assert_eq!(file.content.views, vec![view]);
        let part = n
            .api
            .file_original(&OWNER, reference, Some((4, Some(9))))
            .await
            .unwrap();
        assert_eq!(part.content_type, media_type);
        assert_eq!((part.start, part.total), (4, 256));
        assert_eq!(part.bytes, bytes[4..10]);
    }
    for path in ["notes.txt"] {
        assert!(n
            .api
            .file_original(
                &OWNER,
                FileRef {
                    space: "media-originals".into(),
                    path: path.into(),
                },
                None,
            )
            .await
            .is_err());
    }
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

    std::fs::write(trip.join("damaged.json"), "{\"place\": broken json}\n").unwrap();
    n.api
        .refresh_space(
            &OWNER,
            SpaceRef {
                space: "trip-tables".into(),
            },
        )
        .await
        .unwrap();
    let damaged = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "trip-tables".into(),
                path: "damaged.json".into(),
            },
        )
        .await
        .unwrap();
    assert!(damaged.text.is_some());
    assert_eq!(damaged.content.views, vec![ContentView::Source]);
    assert!(damaged.content.unavailable_reason.is_some());
    sql(&n, "SELECT path FROM \"trip-tables\".files").await;
    assert!(n
        .api
        .read_file_rows(&OWNER, rows("damaged.json", 0, 100))
        .await
        .is_err());
}

#[tokio::test]
async fn save_to_files_keeps_originals_and_never_overwrites() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "first").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("source".into()),
            },
        )
        .await
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let destination = SaveDestination::Files {
        place: None,
        path: target.path().to_string_lossy().into_owned(),
        folder_rule: String::new(),
        name_rule: "{name}".into(),
    };
    for expected in ["a.txt", "a-2.txt"] {
        let saved = n
            .api
            .save_to(
                &OWNER,
                SaveTo {
                    source: "source".into(),
                    paths: None,
                    destination: destination.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.files[0].path, expected);
        assert_eq!(saved.files[0].size, 5);
        assert_eq!(saved.files[0].hash, blake3::hash(b"first").to_string());
        assert_eq!(
            std::fs::read(target.path().join(expected)).unwrap(),
            b"first"
        );
    }
    std::fs::write(root.join("a.txt"), "changed").unwrap();
    assert_eq!(
        std::fs::read(target.path().join("a.txt")).unwrap(),
        b"first"
    );
}

#[tokio::test]
async fn save_to_space_keeps_original_bytes_and_portfolio() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "first").unwrap();
    let mut jpeg = Vec::new();
    image_png(&mut jpeg);
    std::fs::write(root.join("b.jpg"), jpeg.clone()).unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("source".into()),
            },
        )
        .await
        .unwrap();
    let saved = n
        .api
        .save_to(
            &OWNER,
            SaveTo {
                source: "source".into(),
                paths: Some(vec!["b.jpg".into(), "a.txt".into()]),
                destination: SaveDestination::Space {
                    name: "trip".into(),
                    storage: None,
                    title: "A weekend".into(),
                    cover: Some("b.jpg".into()),
                    captions: HashMap::from([("a.txt".into(), "First caption".into())]),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.space.unwrap().name, "trip");
    assert_eq!(saved.files.len(), 2);
    std::fs::write(root.join("a.txt"), "changed").unwrap();
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "trip".into(),
                folder: None,
                offset: None,
                limit: None,
                deep: Some(true),
                kind: None,
                sort: None,
                descending: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(listed.total, 2);
    assert_eq!(listed.files[0].path, "b.jpg");
    let note = n
        .api
        .read_file(
            &OWNER,
            FileRef {
                space: "trip".into(),
                path: "a.txt".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(note.text.as_deref(), Some("first"));
    assert_eq!(note.title.as_deref(), Some("First caption"));
    let original = n
        .api
        .file_original(
            &OWNER,
            FileRef {
                space: "trip".into(),
                path: "b.jpg".into(),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(original.bytes, jpeg);
    assert!(!n
        .api
        .file_thumbnail(
            &OWNER,
            FileRef {
                space: "trip".into(),
                path: "b.jpg".into(),
            },
        )
        .await
        .unwrap()
        .is_empty());
    let rows = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "trip".into(),
                table: "_oikome_files".into(),
                limit: Some(10),
                offset: None,
            },
        )
        .await
        .unwrap();
    let batches = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .map(|b| b.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
    let batch = &batches[0];
    let paths = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    let captions = batch
        .column(3)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    let bytes = batch
        .column(5)
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .unwrap();
    assert_eq!(paths.value(0), "b.jpg");
    assert_eq!(bytes.value(0), jpeg);
    assert_eq!(paths.value(1), "a.txt");
    assert_eq!(captions.value(1), "First caption");
    assert_eq!(bytes.value(1), b"first");
    let portfolio = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "trip".into(),
                table: "_oikome_portfolio".into(),
                limit: Some(1),
                offset: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&portfolio), 1);
}

#[tokio::test]
async fn save_to_space_detaches_connector_tables() {
    let n = node();
    notes(&n).await;
    n.api
        .save_to(
            &OWNER,
            SaveTo {
                source: "notes".into(),
                paths: None,
                destination: SaveDestination::Space {
                    name: "saved-notes".into(),
                    storage: None,
                    title: "Saved notes".into(),
                    cover: None,
                    captions: HashMap::new(),
                },
            },
        )
        .await
        .unwrap();
    sql(&n, "INSERT INTO notes.memos VALUES (4, 'later')").await;
    for (space, expected) in [("notes", 4), ("saved-notes", 3)] {
        let rows = n
            .api
            .read_rows(
                &OWNER,
                ReadRows {
                    space: space.into(),
                    table: "memos".into(),
                    limit: Some(10),
                    offset: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(row_count(&rows), expected);
    }
}

#[tokio::test]
async fn analysis_profiles_drive_explicit_derived_results_and_source_review() {
    let n = node();
    n.api
        .create_space(
            &OWNER,
            NewSpace {
                name: "spending".into(),
                kind: Some(SpaceKind::Transactional),
                storage: None,
            },
        )
        .await
        .unwrap();
    sql(&n, "CREATE TABLE spending.entries (id BIGINT, day DATE, amount DOUBLE, category VARCHAR, title VARCHAR)").await;
    sql(&n, "INSERT INTO spending.entries VALUES (1, '2026-01-01', 12.5, 'food', 'lunch'), (2, '2026-01-01', 17.5, 'food', 'dinner'), (3, '2026-01-02', 8, 'travel', 'metro')").await;
    let profile = n
        .api
        .dataset_profile(
            &OWNER,
            SpaceRef {
                space: "spending.entries".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(profile.fields[0].role, FieldRole::Key);
    assert_eq!(profile.fields[1].role, FieldRole::Time);
    assert_eq!(profile.fields[2].role, FieldRole::Amount);
    assert_eq!(profile.fields[3].role, FieldRole::Category);
    assert_eq!(profile.fields[4].role, FieldRole::Title);
    let analysis = n
        .api
        .analyze_dataset(
            &OWNER,
            AnalyzeDataset {
                space: "spending".into(),
                table: "entries".into(),
                name: "spending-analysis".into(),
                time_field: "day".into(),
                amount_field: "amount".into(),
                category_field: "category".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(analysis.space.name, "spending-analysis");
    assert!(!analysis.profile.source_changed);
    let rows = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "spending-analysis".into(),
                table: "_analysis_daily".into(),
                limit: Some(10),
                offset: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 2);
    let batches = StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect::<Vec<_>>();
    let totals = batches[0]
        .column(2)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap();
    assert_eq!(totals.value(0), 30.0);
    assert_eq!(totals.value(1), 8.0);
    assert_eq!(
        row_count(
            &n.api
                .read_rows(
                    &OWNER,
                    ReadRows {
                        space: "spending".into(),
                        table: "entries".into(),
                        limit: Some(10),
                        offset: None,
                    }
                )
                .await
                .unwrap()
        ),
        3
    );
    sql(&n, "ALTER TABLE spending.entries ADD COLUMN note VARCHAR").await;
    let changed = n
        .api
        .analysis_profile(
            &OWNER,
            SpaceRef {
                space: "spending-analysis".into(),
            },
        )
        .await
        .unwrap();
    assert!(changed.source_changed);
}

async fn saved_files(n: &Node, name: &str) -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "first").unwrap();
    std::fs::write(root.join("b.txt"), "second").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("source".into()),
            },
        )
        .await
        .unwrap();
    n.api
        .save_to(
            &OWNER,
            SaveTo {
                source: "source".into(),
                paths: None,
                destination: SaveDestination::Space {
                    name: name.into(),
                    storage: None,
                    title: "Published notes".into(),
                    cover: None,
                    captions: HashMap::new(),
                },
            },
        )
        .await
        .unwrap();
    source
}

#[tokio::test]
async fn public_snapshot_is_signed_private_and_revocable() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("source");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "first").unwrap();
    let tiff = [
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x25, 0x88, 4, 0, 0, 0, 0, 0,
    ];
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([120, 80, 40])))
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let mut with_gps = b"Exif\0\0".to_vec();
    with_gps.extend_from_slice(&tiff);
    let mut app = vec![0xff, 0xe1];
    app.extend_from_slice(&((with_gps.len() as u16 + 2).to_be_bytes()));
    app.extend_from_slice(&with_gps);
    jpeg.splice(2..2, app);
    std::fs::write(root.join("b.jpg"), jpeg).unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("source".into()),
            },
        )
        .await
        .unwrap();
    n.api
        .save_to(
            &OWNER,
            SaveTo {
                source: "source".into(),
                paths: None,
                destination: SaveDestination::Space {
                    name: "trip".into(),
                    storage: None,
                    title: "A weekend".into(),
                    cover: Some("b.jpg".into()),
                    captions: HashMap::from([("a.txt".into(), "First caption".into())]),
                },
            },
        )
        .await
        .unwrap();
    let share = n
        .api
        .create_public_share(
            &OWNER,
            NewPublicShare {
                space: "trip".into(),
                allow_copy: Some(true),
                requests_per_minute: Some(10),
                max_rows: Some(100),
                max_ms: Some(1_000),
                remove_location: None,
                reach: None,
                expires_at_ms: None,
            },
        )
        .await
        .unwrap();
    assert!(share.active);
    // Without copying, a visitor sees a photo's preview, never the original,
    // and a share given an end stops at it.
    let viewing = n
        .api
        .create_public_share(
            &OWNER,
            NewPublicShare {
                space: "trip".into(),
                allow_copy: Some(false),
                requests_per_minute: None,
                max_rows: None,
                max_ms: None,
                remove_location: None,
                reach: None,
                expires_at_ms: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64
                        + 1_500,
                ),
            },
        )
        .await
        .unwrap();
    let file = |path: &str| PublicFileRef {
        id: viewing.id.clone(),
        path: path.into(),
    };
    assert!(n.api.public_file(file("b.jpg")).await.is_err());
    let preview = n.api.public_preview(file("b.jpg")).await.unwrap();
    assert_eq!(&preview[..2], &[0xff, 0xd8]);
    assert!(!preview.windows(4).any(|w| w == b"Exif"));
    assert!(n.api.public_preview(file("a.txt")).await.is_err());
    assert!(n
        .api
        .create_public_share(
            &OWNER,
            NewPublicShare {
                space: "trip".into(),
                allow_copy: None,
                requests_per_minute: None,
                max_rows: None,
                max_ms: None,
                remove_location: None,
                reach: None,
                expires_at_ms: Some(1),
            },
        )
        .await
        .is_err());
    tokio::time::sleep(std::time::Duration::from_millis(1_600)).await;
    assert!(n
        .api
        .public_manifest(PublicShareRef {
            id: viewing.id.clone()
        })
        .await
        .is_err());
    assert!(n.api.public_preview(file("b.jpg")).await.is_err());
    let manifest = n
        .api
        .public_manifest(PublicShareRef {
            id: share.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(manifest.title, "A weekend");
    assert_eq!(manifest.cover.as_deref(), Some("b.jpg"));
    assert_eq!(manifest.files.len(), 2);
    assert!(manifest.remove_location);
    let payload = serde_json::to_vec(&serde_json::json!({
        "version": manifest.version,
        "owner": manifest.owner,
        "space": manifest.space,
        "title": manifest.title,
        "cover": manifest.cover,
        "createdAtMs": manifest.created_at_ms,
        "removeLocation": manifest.remove_location,
        "allowCopy": manifest.allow_copy,
        "files": manifest.files,
    }))
    .unwrap();
    n.engines
        .identity()
        .unwrap()
        .did()
        .verify(&payload, &hex::decode(&manifest.signature).unwrap())
        .unwrap();
    let bytes = n
        .api
        .public_snapshot(PublicShareRef {
            id: share.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        blake3::hash(&bytes).to_string(),
        share.snapshot.unwrap().hash
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    use std::io::Read;
    let mut photo = Vec::new();
    zip.by_name("b.jpg")
        .unwrap()
        .read_to_end(&mut photo)
        .unwrap();
    assert!(!photo.windows(2).any(|pair| pair == [0x25, 0x88]));
    let photo_file = manifest
        .files
        .iter()
        .find(|file| file.path == "b.jpg")
        .unwrap();
    assert_eq!(blake3::hash(&photo).to_string(), photo_file.hash);
    let revoked = n
        .api
        .revoke_public_share(
            &OWNER,
            PublicShareRef {
                id: share.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(!revoked.active);
    assert!(n
        .api
        .public_snapshot(PublicShareRef {
            id: share.id.clone()
        })
        .await
        .is_err());
    let shares = n.api.list_public_shares(&OWNER, Empty {}).await.unwrap();
    assert_eq!(shares.len(), 2);
    assert!(!shares.iter().find(|s| s.id == share.id).unwrap().active);
}

#[tokio::test]
async fn public_live_queries_authenticate_limit_log_and_revoke() {
    let mut n = node();
    let _source = saved_files(&n, "published").await;
    let log = oikome_node::AccessLog::open(&n.catalog.oikome_dir()).unwrap();
    n.api = n.api.with_access_log(log);
    let share = n
        .api
        .create_public_share(
            &OWNER,
            NewPublicShare {
                space: "published".into(),
                allow_copy: Some(false),
                requests_per_minute: Some(4),
                max_rows: Some(1),
                max_ms: Some(1_000),
                remove_location: None,
                reach: None,
                expires_at_ms: None,
            },
        )
        .await
        .unwrap();
    assert!(n
        .api
        .public_snapshot(PublicShareRef {
            id: share.id.clone()
        })
        .await
        .is_err());
    let visitor = NodeIdentity::from_seed(&[88; 32]);
    let did = visitor.did().to_string();
    let challenge = n
        .api
        .public_auth_challenge(PublicAuthRequest {
            id: share.id.clone(),
            did: did.clone(),
        })
        .await
        .unwrap();
    let signature = hex::encode(visitor.sign(challenge.message.as_bytes()));
    let verification = PublicAuthVerification {
        id: share.id.clone(),
        did: did.clone(),
        message: challenge.message,
        signature,
    };
    let session = n
        .api
        .public_auth_verify(verification.clone())
        .await
        .unwrap();
    assert!(n.api.public_auth_verify(verification).await.is_err());
    let query = |sql: &str| PublicQuery {
        id: share.id.clone(),
        token: session.token.clone(),
        sql: sql.into(),
    };
    assert!(n
        .api
        .public_query(query("SELECT content FROM _oikome_files LIMIT 1"))
        .await
        .is_err());
    assert!(n
        .api
        .public_query(query("SELECT path FROM _oikome_files"))
        .await
        .is_err());
    let rows = n
        .api
        .public_query(query("SELECT path FROM _oikome_files WHERE path = 'a.txt'"))
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 1);
    assert!(n
        .api
        .public_query(query("DELETE FROM _oikome_files"))
        .await
        .is_err());
    let limited = n
        .api
        .public_query(query("SELECT path FROM _oikome_files LIMIT 1"))
        .await
        .unwrap_err();
    assert!(limited.message.contains("request limit"));
    n.api
        .revoke_public_share(
            &OWNER,
            PublicShareRef {
                id: share.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(n
        .api
        .public_query(query("SELECT path FROM _oikome_files LIMIT 1"))
        .await
        .is_err());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let activity = n
        .api
        .list_access(&OWNER, ListAccess::default())
        .await
        .unwrap();
    assert!(activity
        .iter()
        .any(|entry| entry.did.as_deref() == Some(did.as_str())
            && entry.method == "public_query"
            && entry.outcome == AccessOutcome::Allowed));
    assert!(activity
        .iter()
        .any(|entry| entry.did.as_deref() == Some(did.as_str())
            && entry.outcome == AccessOutcome::Denied));
}

#[tokio::test]
async fn public_live_queries_verify_ethereum_signatures() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};
    use sha3::{Digest, Keccak256};

    let n = node();
    let _source = saved_files(&n, "published").await;
    let share = n
        .api
        .create_public_share(
            &OWNER,
            NewPublicShare {
                space: "published".into(),
                allow_copy: Some(true),
                requests_per_minute: None,
                max_rows: None,
                max_ms: None,
                remove_location: None,
                reach: None,
                expires_at_ms: None,
            },
        )
        .await
        .unwrap();
    let key = SigningKey::from_bytes(&[7; 32].into()).unwrap();
    let encoded = key.verifying_key().to_encoded_point(false);
    let address = hex::encode(&Keccak256::digest(&encoded.as_bytes()[1..])[12..]);
    let did = format!("did:pkh:eip155:1:0x{address}");
    let challenge = n
        .api
        .public_auth_challenge(PublicAuthRequest {
            id: share.id.clone(),
            did: did.clone(),
        })
        .await
        .unwrap();
    assert!(challenge.message.contains("Version: 1\nChain ID: 1"));
    let prefixed = format!(
        "\x19Ethereum Signed Message:\n{}{}",
        challenge.message.len(),
        challenge.message
    );
    let hash = Keccak256::digest(prefixed.as_bytes());
    let (signature, recovery): (Signature, _) = key.sign_prehash(&hash).unwrap();
    let mut signature = signature.to_vec();
    signature.push(recovery.to_byte() + 27);
    let session = n
        .api
        .public_auth_verify(PublicAuthVerification {
            id: share.id.clone(),
            did,
            message: challenge.message,
            signature: hex::encode(signature),
        })
        .await
        .unwrap();
    let rows = n
        .api
        .public_query(PublicQuery {
            id: share.id.clone(),
            token: session.token,
            sql: "SELECT content FROM _oikome_files WHERE path = 'a.txt'".into(),
        })
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 1);
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

#[tokio::test]
async fn an_external_mcp_client_uses_the_same_manifest_driven_setup() {
    let n = node();
    let package = tempfile::tempdir().unwrap();
    let client_home = tempfile::tempdir().unwrap();
    std::fs::write(
        package.path().join("oikome-plugin.json"),
        r#"{
          "id":"external-mcp", "name":"External MCP", "by":"Sam", "version":"1.0.0",
          "capabilities":{"mcpClients":[{
            "id":"external-cli", "name":"External CLI",
            "config":{"format":"json", "homeEnv":"OIKOME_TEST_EXTERNAL_MCP_HOME",
              "directory":".external-mcp", "file":"client.json",
              "root":"customServers", "headers":"authHeaders"}
          }]}
        }"#,
    )
    .unwrap();
    let installed = n
        .api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: None,
                path: Some(package.path().to_string_lossy().into_owned()),
            },
        )
        .await
        .unwrap();
    assert!(installed.capabilities.cards.is_empty());
    assert!(installed.collections.is_empty());
    assert!(!installed.home_by_default);

    let variable = "OIKOME_TEST_EXTERNAL_MCP_HOME";
    let previous = std::env::var_os(variable);
    unsafe { std::env::set_var(variable, client_home.path()) };
    let connected = n
        .api
        .connect_assistant(
            &OWNER,
            ConnectAssistant {
                name: String::new(),
                app: AssistantApp::Other,
                plugin: Some("external-mcp".into()),
                client: Some("external-cli".into()),
                collections: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(connected.assistant.name, "External CLI");
    assert_eq!(connected.assistant.plugin.as_deref(), Some("external-mcp"));
    n.api
        .install_assistant(
            &OWNER,
            InstallAssistant {
                id: connected.assistant.id.clone(),
                token: connected.token.clone(),
                url: "http://127.0.0.1:7342/mcp".into(),
                replace: false,
            },
        )
        .await
        .unwrap();
    match previous {
        Some(value) => unsafe { std::env::set_var(variable, value) },
        None => unsafe { std::env::remove_var(variable) },
    }
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(client_home.path().join("client.json")).unwrap())
            .unwrap();
    assert_eq!(
        saved["customServers"]["oikome"]["authHeaders"]["Authorization"],
        format!("Bearer {}", connected.token)
    );
    let records = std::fs::read_to_string(n.catalog.oikome_dir().join("assistants.json")).unwrap();
    assert!(!records.contains(&connected.token));
}

#[tokio::test]
async fn official_registry_packages_are_verified_before_installation() {
    use sha2::Digest as _;
    use std::io::Write as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../plugins/spending-by-category/oikome-plugin.json"
    ))
    .unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("oikome-plugin.json", options).unwrap();
    zip.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    for (name, bytes) in [
        (
            "index.html",
            &include_bytes!("../../../../plugins/spending-by-category/index.html")[..],
        ),
        (
            "main.js",
            &include_bytes!("../../../../plugins/spending-by-category/main.js")[..],
        ),
        (
            "style.css",
            &include_bytes!("../../../../plugins/spending-by-category/style.css")[..],
        ),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    let package = zip.finish().unwrap().into_inner();
    let bad_manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../plugins/photo-memories/oikome-plugin.json"
    ))
    .unwrap();
    let index = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "plugins": [
            {
                "manifest": manifest,
                "download": format!("http://{address}/spending-by-category.zip"),
                "sha256": format!("{:x}", sha2::Sha256::digest(&package)),
                "bytes": package.len()
            },
            {
                "manifest": bad_manifest,
                "download": format!("http://{address}/spending-by-category.zip"),
                "sha256": "0".repeat(64),
                "bytes": package.len()
            }
        ]
    }))
    .unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let index = index.clone();
            let package = package.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(n) = socket.read(&mut buffer).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                let body = if request.starts_with(b"GET /index.json ") {
                    index
                } else if request.starts_with(b"GET /spending-by-category.zip ") {
                    package
                } else {
                    let response =
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(response).await;
                    return;
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });

    let mut n = node();
    n.api = n.api.clone().with_registry_endpoints(
        format!("http://{address}/index.json"),
        format!("http://{address}/"),
    );
    let offered = n.api.list_registry_plugins(&OWNER, Empty {}).await.unwrap();
    assert_eq!(offered.len(), 2);
    assert!(offered[0].plugin.installed);
    let installed = n
        .api
        .install_registry_plugin(
            &OWNER,
            RegistryPluginRef {
                id: "spending-by-category".into(),
                version: "1.0.0".into(),
            },
        )
        .await
        .unwrap();
    assert!(installed.installed && installed.collections.is_empty());
    let page = String::from_utf8(
        n.api
            .plugin_file("spending-by-category", "index.html")
            .unwrap(),
    )
    .unwrap();
    assert!(page.contains("/plugins/sdk.js"));
    let photo_before = n.api.plugin_file("photo-memories", "index.html").unwrap();
    let error = n
        .api
        .install_registry_plugin(
            &OWNER,
            RegistryPluginRef {
                id: "photo-memories".into(),
                version: "1.0.0".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("plugin_registry_checksum"));
    assert_eq!(
        n.api.plugin_file("photo-memories", "index.html").unwrap(),
        photo_before
    );
    server.abort();
}

#[tokio::test]
async fn removed_official_plugins_stay_removed_and_hide_mcp_clients() {
    let n = node();
    let plugins = n.api.list_plugins(&OWNER, Empty {}).await.unwrap();
    assert!(plugins
        .iter()
        .all(|plugin| !plugin.built_in || plugin.installed));
    n.api
        .remove_plugin(
            &OWNER,
            PluginRef {
                id: "kimi-code".into(),
            },
        )
        .await
        .unwrap();
    let error = n
        .api
        .connect_assistant(
            &OWNER,
            ConnectAssistant {
                name: String::new(),
                app: AssistantApp::Other,
                plugin: Some("kimi-code".into()),
                client: Some("kimi-code-cli".into()),
                collections: Vec::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    let reopened = Api::new(n.catalog.clone(), n.engines.clone());
    let plugins = reopened.list_plugins(&OWNER, Empty {}).await.unwrap();
    assert!(plugins
        .iter()
        .any(|plugin| plugin.id == "kimi-code" && !plugin.installed));
    n.api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: Some("kimi-code".into()),
                path: None,
            },
        )
        .await
        .unwrap();
    n.api
        .connect_assistant(
            &OWNER,
            ConnectAssistant {
                name: String::new(),
                app: AssistantApp::Other,
                plugin: Some("kimi-code".into()),
                client: Some("kimi-code-cli".into()),
                collections: Vec::new(),
            },
        )
        .await
        .unwrap();
}

/// Knowledge (docs/knowledge-schema.md, K1) is the owner's: a contact may
/// neither look at nor prepare it, a request is checked before the node
/// runs it, and a preview reads the documents without writing anything.
#[tokio::test]
async fn knowledge_is_the_owners_and_its_requests_are_checked() {
    let n = node();
    let root = std::path::PathBuf::from(n.catalog.root_path());
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(
        docs.join("architecture.md"),
        "# Architecture\n\n## Federation\n\nDataFusion coordinates and never reads files.\n",
    )
    .unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: docs.to_string_lossy().into_owned(),
                name: Some("docs".into()),
            },
        )
        .await
        .unwrap();
    let docs_ref = || SpaceRef {
        space: "docs".into(),
    };

    let contact = Caller::Principal { did: did(9) };
    let e = n
        .api
        .knowledge_status(&contact, docs_ref())
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    let e = n
        .api
        .prepare_knowledge(&contact, docs_ref())
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);
    let e = n.api.install_lance(&contact, Empty {}).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::PermissionDenied);

    // Not prepared: nothing to show, and nothing is found.
    let status = n.api.knowledge_status(&OWNER, docs_ref()).await.unwrap();
    assert!(status.prepared_ms.is_none());
    assert!(status.files.is_empty());
    let none = n
        .api
        .retrieval_test(
            &OWNER,
            RetrievalTest {
                space: "docs".into(),
                query: "DataFusion".into(),
                limit: None,
                mode: None,
            },
        )
        .await
        .unwrap();
    assert!(none.hits.is_empty());

    // The preview: the first passages with their places, nothing written.
    let preview = n.api.knowledge_preview(&OWNER, docs_ref()).await.unwrap();
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].path, "architecture.md");
    assert_eq!(preview[0].heading, vec!["Architecture", "Federation"]);
    assert_eq!((preview[0].line_start, preview[0].line_end), (3, 5));
    let status = n.api.knowledge_status(&OWNER, docs_ref()).await.unwrap();
    assert!(status.prepared_ms.is_none());

    let e = n
        .api
        .set_knowledge_chunk(
            &OWNER,
            SetKnowledgeChunk {
                space: "docs".into(),
                chunk_id: "'; DROP TABLE x; --".into(),
                enabled: false,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    let e = n
        .api
        .retrieval_test(
            &OWNER,
            RetrievalTest {
                space: "docs".into(),
                query: "x".repeat(201),
                limit: None,
                mode: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);

    // A collection of tables holds no documents to prepare yet.
    notes(&n).await;
    assert!(n
        .api
        .knowledge_preview(
            &OWNER,
            SpaceRef {
                space: "notes".into()
            }
        )
        .await
        .is_err());
}

#[tokio::test]
async fn a_folder_s_data_files_become_analysis_tables_that_follow_them() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("household");
    std::fs::create_dir_all(root.join("spending")).unwrap();
    std::fs::write(
        root.join("spending/2026-09.csv"),
        "date,category,amount\n2026-09-02,Groceries,30.50\n2026-09-14,Books,12.00\n",
    )
    .unwrap();
    std::fs::write(
        root.join("spending/2026-10.csv"),
        "date,category,amount\n2026-10-03,Groceries,20.00\n",
    )
    .unwrap();
    std::fs::write(
        root.join("readings.csv"),
        "recorded_at,room,celsius\n2026-11-01 00:00:00,Bedroom,19.5\n",
    )
    .unwrap();
    std::fs::write(root.join("trip.md"), "# Not a table\n").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("household".into()),
            },
        )
        .await
        .unwrap();
    let household = || SpaceRef {
        space: "household".into(),
    };

    // What can be made, before anything is: a top-level file, and a folder
    // of files read as one table; a note is not a table.
    let offered = n.api.file_analysis(&OWNER, household()).await.unwrap();
    assert!(offered.tables.is_empty());
    let names: Vec<(&str, usize)> = offered
        .sources
        .iter()
        .map(|s| (s.name.as_str(), s.files.len()))
        .collect();
    assert_eq!(names, vec![("readings", 1), ("spending", 2)]);

    // Only the tables the owner chooses are made; a name no files make is
    // refused, and so is choosing none.
    let build = |tables: &[&str]| BuildAnalysis {
        space: "household".into(),
        tables: tables.iter().map(|t| t.to_string()).collect(),
    };
    let err = n
        .api
        .build_analysis(&OWNER, build(&["spending", "trip"]))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Invalid, "{err:?}");
    assert!(n.api.build_analysis(&OWNER, build(&[])).await.is_err());
    let made = n
        .api
        .build_analysis(&OWNER, build(&["spending"]))
        .await
        .unwrap();
    let names: Vec<_> = made.sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["readings"]);
    let names: Vec<_> = made.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["spending"]);
    let made = n
        .api
        .build_analysis(&OWNER, build(&["readings"]))
        .await
        .unwrap();
    assert!(made.sources.is_empty());
    let spending = made.tables.iter().find(|t| t.name == "spending").unwrap();
    assert_eq!(spending.rows, 3);
    assert_eq!(spending.problem, None);
    let role = |table: &AnalysisTable, field: &str| {
        table.fields.iter().find(|f| f.name == field).unwrap().role
    };
    assert_eq!(role(spending, "date"), FieldRole::Time);
    assert_eq!(role(spending, "amount"), FieldRole::Amount);
    assert_eq!(role(spending, "category"), FieldRole::Category);
    assert_eq!(role(spending, "_source_file"), FieldRole::Other);

    // The tables are the collection's: queried by name, each row with its file.
    let rows = n
        .api
        .read_rows(
            &OWNER,
            ReadRows {
                space: "household".into(),
                table: "spending".into(),
                limit: None,
                offset: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 3);

    let summary = n
        .api
        .analysis_summary(
            &OWNER,
            AnalysisTableRef {
                space: "household".into(),
                table: "spending".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        summary
            .by_month
            .iter()
            .map(|p| (p.label.as_str(), p.value))
            .collect::<Vec<_>>(),
        vec![("2026-09", 42.5), ("2026-10", 20.0)]
    );
    assert_eq!(summary.by_category[0].label, "Groceries");
    assert_eq!(summary.by_category[0].value, 50.5);

    // A column's role, corrected: the readings are measured in celsius.
    let corrected = n
        .api
        .set_analysis_field(
            &OWNER,
            SetAnalysisField {
                space: "household".into(),
                table: "readings".into(),
                field: "celsius".into(),
                role: FieldRole::Amount,
            },
        )
        .await
        .unwrap();
    let readings = corrected
        .tables
        .iter()
        .find(|t| t.name == "readings")
        .unwrap();
    let celsius = readings
        .fields
        .iter()
        .find(|f| f.name == "celsius")
        .unwrap();
    assert_eq!((celsius.role, celsius.inferred), (FieldRole::Amount, false));
    assert!(n
        .api
        .set_analysis_field(
            &OWNER,
            SetAnalysisField {
                space: "household".into(),
                table: "readings".into(),
                field: "_source_file".into(),
                role: FieldRole::Amount,
            },
        )
        .await
        .is_err());

    // Stopping the analysis of a table drops it with its corrected roles;
    // its file stays and is read in place again, and the table is not made
    // again until the owner chooses it.
    let table_ref = |table: &str| AnalysisTableRef {
        space: "household".into(),
        table: table.into(),
    };
    let stopped = n
        .api
        .remove_analysis_table(&OWNER, table_ref("readings"))
        .await
        .unwrap();
    let names: Vec<_> = stopped.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["spending"]);
    let names: Vec<_> = stopped.sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["readings"]);
    assert!(root.join("readings.csv").is_file());
    assert_eq!(count_of(&n, "household.readings").await, 1);
    let later = n.api.file_analysis(&OWNER, household()).await.unwrap();
    assert!(later.tables.iter().all(|t| t.name != "readings"));
    let err = n
        .api
        .remove_analysis_table(&OWNER, table_ref("readings"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err:?}");
    assert!(n
        .api
        .remove_analysis_table(&Caller::Principal { did: did(9) }, table_ref("spending"))
        .await
        .is_err());
    let again = n
        .api
        .build_analysis(&OWNER, build(&["readings"]))
        .await
        .unwrap();
    let readings = again.tables.iter().find(|t| t.name == "readings").unwrap();
    let celsius = readings
        .fields
        .iter()
        .find(|f| f.name == "celsius")
        .unwrap();
    assert!(celsius.inferred, "its corrected role was forgotten");

    // A file changes: its table follows; a folder that goes takes its table.
    std::fs::write(
        root.join("spending/2026-10.csv"),
        "date,category,amount\n2026-10-03,Groceries,20.00\n2026-10-09,Health,8.25\n",
    )
    .unwrap();
    std::fs::remove_file(root.join("readings.csv")).unwrap();
    n.api.refresh_space(&OWNER, household()).await.unwrap();
    let followed = n.api.file_analysis(&OWNER, household()).await.unwrap();
    let names: Vec<(&str, u64)> = followed
        .tables
        .iter()
        .map(|t| (t.name.as_str(), t.rows))
        .collect();
    assert_eq!(names, vec![("spending", 4)]);

    // Removing the collection removes what was made of its files, not them.
    n.api.drop_space(&OWNER, household()).await.unwrap();
    assert!(root.join("spending/2026-09.csv").is_file());
    assert!(!n
        .catalog
        .oikome_dir()
        .join("analysis/household.json")
        .exists());
}

/// A chart is a description (its time at a grain, dimensions, comparisons,
/// metrics, filters, order, how many): the node writes one plain query from
/// it, grouping by the time and every dimension together, and answers with
/// its groups and series. Charts are kept with their table as views.
#[tokio::test]
async fn a_chart_of_an_analysis_table_is_computed_from_its_description() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("household");
    std::fs::create_dir_all(root.join("spending")).unwrap();
    std::fs::write(
        root.join("spending/2026-07.csv"),
        "date,category,merchant,amount,items\n2026-07-20,Books,Bertrand,30.00,2\n",
    )
    .unwrap();
    std::fs::write(
        root.join("spending/2026-09.csv"),
        "date,category,merchant,amount,items\n2026-09-02,Groceries,Lidl,30.50,3\n2026-09-14,Books,Bertrand,12.00,1\n2026-09-20,Groceries,Pingo Doce,9.50,2\n",
    )
    .unwrap();
    std::fs::write(
        root.join("spending/2026-10.csv"),
        "date,category,merchant,amount,items\n2026-10-03,Groceries,Lidl,20.00,3\n2026-10-09,It's \"odd\"; DROP TABLE x,Somewhere,8.25,10\n",
    )
    .unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("household".into()),
            },
        )
        .await
        .unwrap();
    n.api
        .build_analysis(
            &OWNER,
            BuildAnalysis {
                space: "household".into(),
                tables: vec!["spending".into()],
            },
        )
        .await
        .unwrap();
    let spec = |json: serde_json::Value| -> ChartSpec { serde_json::from_value(json).unwrap() };
    let chart = |json: serde_json::Value| AnalysisChart {
        space: "household".into(),
        table: "spending".into(),
        spec: spec(json),
    };
    let keys = |result: &ChartResult| -> Vec<Vec<String>> {
        result
            .groups
            .iter()
            .map(|g| g.keys.iter().map(|k| k.clone().unwrap_or_default()).collect())
            .collect()
    };
    let firsts = |result: &ChartResult| -> Vec<String> {
        keys(result).into_iter().map(|k| k[0].clone()).collect()
    };
    let compute = |json: serde_json::Value| n.api.analysis_chart(&OWNER, chart(json));

    // By month: one metric, the same column totalled twice more, the
    // number of rows. August has no rows and keeps its place.
    let by_month = compute(serde_json::json!({
        "chart": "column",
        "time": {"column": "date", "grain": "month"},
        "metrics": [
            {"column": "amount", "aggregate": "sum"},
            {"column": "amount", "aggregate": "average"},
            {"column": "amount", "aggregate": "median"},
            {"column": "category", "aggregate": "distinct"},
            {"aggregate": "count"}
        ]
    }))
    .await
    .unwrap();
    assert_eq!(by_month.matched, 6);
    assert_eq!(by_month.series.len(), 5);
    assert_eq!(firsts(&by_month), ["2026-07", "2026-08", "2026-09", "2026-10"]);
    assert_eq!(by_month.groups[1].values, [None, None, None, None, None]);
    assert_eq!(
        by_month.groups[2].values,
        [
            Some(52.0),
            Some(52.0 / 3.0),
            Some(12.0),
            Some(2.0),
            Some(3.0)
        ]
    );
    let columns: Vec<_> = by_month
        .columns
        .iter()
        .map(|c| (c.column.as_str(), c.grain))
        .collect();
    assert_eq!(columns, [("date", Some(DateGrain::Month))]);
    // Time keeps its own order, all of it: an order by a metric and a
    // number of groups are for the dimensions. The query is as plain as
    // the chart.
    let ordered = compute(serde_json::json!({
        "chart": "column",
        "time": {"column": "date", "grain": "month"},
        "metrics": [{"column": "amount", "aggregate": "sum"}],
        "sort": {"metric": 0, "descending": true},
        "limit": 1
    }))
    .await
    .unwrap();
    assert_eq!(firsts(&ordered), ["2026-07", "2026-08", "2026-09", "2026-10"]);
    assert_eq!(
        ordered.query.sql,
        "SELECT strftime(\"date\", '%Y-%m') AS \"date\",\n       SUM(\"amount\") AS \"SUM(amount)\"\n\
         FROM \"household\".\"spending\"\nGROUP BY 1\nORDER BY 1\nLIMIT 1000"
    );
    assert!(ordered.query.params.is_empty());

    // Rolled up to the year, and zoomed into a period: a filter on it and
    // a finer grain, its periods all there. Every grain is worked out from
    // the rows: the year's average is of its rows, not of its months.
    let at = |grain: &str, period: Option<(&str, &str)>| {
        let filters = match period {
            Some((of, key)) => serde_json::json!([{"column": "date", "period": {"grain": of, "key": key}}]),
            None => serde_json::json!([]),
        };
        serde_json::json!({
            "chart": "column",
            "time": {"column": "date", "grain": grain},
            "metrics": [{"column": "amount", "aggregate": "sum"}, {"column": "amount", "aggregate": "average"}],
            "filters": filters
        })
    };
    let year = compute(at("year", None)).await.unwrap();
    assert_eq!(firsts(&year), ["2026"]);
    assert_eq!(year.groups[0].values, [Some(110.25), Some(110.25 / 6.0)]);
    let quarters = compute(at("quarter", Some(("year", "2026")))).await.unwrap();
    assert_eq!(
        firsts(&quarters),
        ["2026-Q1", "2026-Q2", "2026-Q3", "2026-Q4"]
    );
    assert_eq!(quarters.groups[0].values, [None, None]);
    assert_eq!(quarters.groups[2].values[0], Some(82.0));
    let months = compute(at("month", Some(("quarter", "2026-Q3")))).await.unwrap();
    assert_eq!(firsts(&months), ["2026-07", "2026-08", "2026-09"]);
    assert!(months.query.sql.contains("= ?1"));
    assert_eq!(months.query.params, ["'2026-Q3'"]);
    let days = compute(at("day", Some(("month", "2026-09")))).await.unwrap();
    assert_eq!(days.groups.len(), 30);
    assert_eq!(days.matched, 3);
    let weeks = compute(at("week", None)).await.unwrap();
    assert_eq!(firsts(&weeks)[0], "2026-07-20");

    // The time and a dimension, grouped together: a group for each month
    // and category, ordered by the metric within each month.
    let month_category = compute(serde_json::json!({
        "chart": "table",
        "time": {"column": "date", "grain": "month"},
        "dimensions": [{"column": "category"}],
        "metrics": [{"column": "amount", "aggregate": "sum"}],
        "sort": {"metric": 0, "descending": true}
    }))
    .await
    .unwrap();
    let odd = "It's \"odd\"; DROP TABLE x";
    assert_eq!(
        keys(&month_category),
        [
            vec!["2026-07".to_string(), "Books".into()],
            vec!["2026-09".into(), "Groceries".into()],
            vec!["2026-09".into(), "Books".into()],
            vec!["2026-10".into(), "Groceries".into()],
            vec!["2026-10".into(), odd.into()],
        ]
    );
    assert_eq!(month_category.columns.len(), 2);

    // Several dimensions together, ordered by a metric, the first two.
    let top = compute(serde_json::json!({
        "chart": "bar",
        "dimensions": [{"column": "category"}, {"column": "merchant"}],
        "metrics": [{"column": "amount", "aggregate": "sum"}],
        "sort": {"metric": 0, "descending": true},
        "limit": 2
    }))
    .await
    .unwrap();
    assert_eq!(
        keys(&top),
        [
            vec!["Groceries".to_string(), "Lidl".into()],
            vec!["Books".into(), "Bertrand".into()]
        ]
    );
    assert!(top.truncated);
    assert!(top.query.sql.ends_with("LIMIT 2"));
    // Compared values: the top group is picked by its own total, an
    // average over all its rows (Books, 21), not one of what is compared
    // within it (Groceries' Lidl, 25.25).
    let compared = compute(serde_json::json!({
        "chart": "bar",
        "dimensions": [{"column": "category"}],
        "compare": [{"column": "merchant"}],
        "metrics": [{"column": "amount", "aggregate": "average"}],
        "sort": {"metric": 0, "descending": true},
        "limit": 1
    }))
    .await
    .unwrap();
    assert_eq!(firsts(&compared), ["Books"]);
    let series: Vec<_> = compared.series.iter().map(|s| s.compare.clone()).collect();
    assert_eq!(series, [vec![Some("Bertrand".to_string())]]);
    assert!(compared.query.sql.starts_with("WITH top AS ("));
    let compared = compute(serde_json::json!({
        "chart": "bar",
        "dimensions": [{"column": "category"}],
        "compare": [{"column": "merchant"}],
        "metrics": [{"column": "amount", "aggregate": "sum"}],
        "filters": [{"column": "category", "values": ["Groceries", "Books"]}],
        "sort": {"metric": 0, "descending": true},
        "limit": 1
    }))
    .await
    .unwrap();
    assert_eq!(firsts(&compared), ["Groceries"]);
    assert_eq!(compared.groups[0].values, [Some(50.5), Some(9.5)]);
    // Each value is bound once, however often the query reads it.
    assert_eq!(compared.query.params, ["'Groceries'", "'Books'"]);

    // A number by its values, in their order; a date by its days; and a
    // value of either kept as it is shown.
    let by_items = compute(serde_json::json!({"chart": "bar", "dimensions": [{"column": "items"}]}))
        .await
        .unwrap();
    assert_eq!(firsts(&by_items), ["1", "2", "3", "10"]);
    assert_eq!(by_items.groups[1].values, [Some(2.0)]);
    let by_day = compute(serde_json::json!({"chart": "table", "dimensions": [{"column": "date"}]}))
        .await
        .unwrap();
    assert_eq!(firsts(&by_day)[0], "2026-07-20");
    for (column, value) in [("items", "10"), ("date", "2026-09-02")] {
        let kept = compute(serde_json::json!({
            "chart": "table",
            "dimensions": [{"column": "category"}],
            "filters": [{"column": column, "values": [value]}]
        }))
        .await
        .unwrap();
        assert_eq!(kept.matched, 1, "{column}");
    }
    // A group broken down: a month and a category kept, by merchant.
    let broken_down = compute(serde_json::json!({
        "chart": "table",
        "dimensions": [{"column": "merchant"}],
        "metrics": [{"column": "amount", "aggregate": "sum"}],
        "filters": [
            {"column": "date", "period": {"grain": "month", "key": "2026-09"}},
            {"column": "category", "values": ["Groceries"]}
        ]
    }))
    .await
    .unwrap();
    assert_eq!(firsts(&broken_down), ["Lidl", "Pingo Doce"]);
    assert_eq!(broken_down.groups[0].values, [Some(30.5)]);

    // Filters: values ticked (bound, never part of the query's text), and
    // a range of numbers; the time's range.
    let filtered = |filters: serde_json::Value| {
        serde_json::json!({
            "chart": "table",
            "dimensions": [{"column": "category"}],
            "metrics": [{"column": "amount", "aggregate": "sum"}],
            "filters": filters
        })
    };
    let ticked = compute(filtered(serde_json::json!([{"column": "category", "values": [odd, "Books"]}])))
        .await
        .unwrap();
    assert_eq!(ticked.matched, 3);
    assert_eq!(firsts(&ticked), ["Books", odd]);
    assert!(!ticked.query.sql.contains("DROP TABLE x"));
    assert!(ticked
        .query
        .params
        .contains(&"'It''s \"odd\"; DROP TABLE x'".to_string()));
    let ranged = compute(filtered(serde_json::json!([{"column": "amount", "min": "9", "max": "20"}])))
        .await
        .unwrap();
    assert_eq!(ranged.matched, 3);
    let recent = compute(serde_json::json!({
        "chart": "column",
        "time": {"column": "date", "grain": "month", "range": "last30Days"}
    }))
    .await
    .unwrap();
    assert_eq!(recent.matched, 0);

    // A range of its own: whole days, either end open; with the time alone,
    // every period in it keeps its place. The last 7 days hold none here.
    let custom = compute(serde_json::json!({
        "chart": "column",
        "time": {"column": "date", "grain": "month", "range": "custom", "from": "2026-08-01", "to": "2026-09-14"}
    }))
    .await
    .unwrap();
    assert_eq!(custom.matched, 2);
    assert_eq!(firsts(&custom), ["2026-08", "2026-09"]);
    assert!(custom.query.sql.contains("\"date\" >= CAST(?1 AS DATE)"));
    assert_eq!(custom.query.params, ["'2026-08-01'", "'2026-09-14'"]);
    let week = compute(serde_json::json!({
        "chart": "column",
        "time": {"column": "date", "grain": "day", "range": "last7Days"}
    }))
    .await
    .unwrap();
    assert_eq!(week.matched, 0);

    // A column read as another type: charts read it so, the table and its
    // files as they were. A number read as text is grouped as text and not
    // totalled; read as DuckDB read it again, it is a number again.
    let read_as = |column: &str, read_as: Option<ColumnType>| SetAnalysisType {
        space: "household".into(),
        table: "spending".into(),
        column: column.into(),
        read_as,
    };
    let changed = n
        .api
        .set_analysis_type(&OWNER, read_as("items", Some(ColumnType::Text)))
        .await
        .unwrap();
    let items = changed.tables[0].fields.iter().find(|f| f.name == "items").unwrap();
    assert_eq!(items.read_as, Some(ColumnType::Text));
    assert_eq!(items.data_type, "Int64");
    let as_text = compute(serde_json::json!({"chart": "bar", "dimensions": [{"column": "items"}]}))
        .await
        .unwrap();
    assert_eq!(firsts(&as_text), ["1", "10", "2", "3"]);
    assert!(as_text
        .query
        .sql
        .contains("TRY_CAST(\"items\" AS VARCHAR) AS \"items\""));
    assert!(compute(serde_json::json!({"chart": "bar", "metrics": [{"column": "items", "aggregate": "sum"}]}))
        .await
        .is_err());
    n.api
        .set_analysis_type(&OWNER, read_as("items", None))
        .await
        .unwrap();
    let as_number = compute(serde_json::json!({"chart": "bar", "dimensions": [{"column": "items"}]}))
        .await
        .unwrap();
    assert_eq!(firsts(&as_number), ["1", "2", "3", "10"]);
    for refused in ["nothing", "_source_file"] {
        assert!(n
            .api
            .set_analysis_type(&OWNER, read_as(refused, Some(ColumnType::Text)))
            .await
            .is_err());
    }

    // Nothing grouped or totalled: the rows themselves.
    let rows = compute(serde_json::json!({"chart": "table"})).await.unwrap();
    assert!(rows.query.sql.starts_with("SELECT *\nFROM \"household\".\"spending\""));
    let rows = rows.rows.unwrap();
    assert_eq!(
        &rows.columns[..5],
        ["date", "category", "merchant", "amount", "items"]
    );
    assert_eq!(rows.rows.len(), 6);

    // What a chart may not ask: a column the table has not, a time that is
    // not of time, a column grouped twice (the time too), a number's total
    // of text, a metric twice, two kinds of filter, a period of what is not
    // time or that is no period, SQL anywhere.
    for refused in [
        serde_json::json!({"chart": "bar", "dimensions": [{"column": "nothing"}]}),
        serde_json::json!({"chart": "bar", "dimensions": [{"column": "amount\" FROM x --"}]}),
        serde_json::json!({"chart": "bar", "time": {"column": "category", "grain": "month"}}),
        serde_json::json!({"chart": "bar", "time": {"column": "date", "grain": "month"}, "dimensions": [{"column": "date"}]}),
        serde_json::json!({"chart": "bar", "dimensions": [{"column": "category"}], "compare": [{"column": "category"}]}),
        serde_json::json!({"chart": "bar", "metrics": [{"column": "category", "aggregate": "sum"}]}),
        serde_json::json!({"chart": "bar", "metrics": [{"aggregate": "sum"}]}),
        serde_json::json!({"chart": "bar", "metrics": [{"column": "amount", "aggregate": "sum"}, {"column": "amount", "aggregate": "sum"}]}),
        serde_json::json!({"chart": "bar", "filters": [{"column": "amount", "min": "1 OR 1=1"}]}),
        serde_json::json!({"chart": "bar", "filters": [{"column": "category", "min": "1"}]}),
        serde_json::json!({"chart": "bar", "filters": [{"column": "category", "values": ["a"], "min": "1"}]}),
        serde_json::json!({"chart": "bar", "filters": [{"column": "category", "period": {"grain": "month", "key": "2026-09"}}]}),
        serde_json::json!({"chart": "bar", "filters": [{"column": "date", "period": {"grain": "month", "key": "2026-13"}}]}),
        serde_json::json!({"chart": "bar", "dimensions": [{"column": "category"}], "sort": {"metric": 4, "descending": true}}),
        serde_json::json!({"chart": "bar", "time": {"column": "date", "grain": "month", "range": "custom", "from": "2026-09-30", "to": "2026-09-01"}}),
        serde_json::json!({"chart": "bar", "time": {"column": "date", "grain": "month", "range": "custom", "from": "yesterday"}}),
        serde_json::json!({"chart": "bar", "time": {"column": "date", "grain": "month", "from": "2026-09-01"}}),
    ] {
        let err = n.api.analysis_chart(&OWNER, chart(refused.clone())).await;
        assert!(err.is_err(), "{refused} should be refused");
    }
    for unknown in [
        serde_json::json!({"chart": "bar", "sql": "SELECT 1"}),
        serde_json::json!({"chart": "bar", "dimensions": [{"column": "date", "grain": "month"}]}),
        serde_json::json!({"chart": "bar", "time": {"column": "date", "grain": "decade"}}),
        serde_json::json!({"chart": "bar", "drill": ["2026"]}),
    ] {
        assert!(serde_json::from_value::<ChartSpec>(unknown).is_err());
    }
    assert!(n
        .api
        .analysis_chart(
            &Caller::Principal { did: did(9) },
            chart(serde_json::json!({"chart": "table"}))
        )
        .await
        .is_err());
    let values = n
        .api
        .analysis_values(
            &OWNER,
            AnalysisValues {
                space: "household".into(),
                table: "spending".into(),
                column: "category".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(values, ["Books", "Groceries", odd]);

    // A chart is kept with its table as a view, under a name: only a
    // chart's description. A table starts with none.
    let save = |id: Option<&str>, name: &str, config: &str| SaveAnalysisView {
        space: "household".into(),
        table: "spending".into(),
        id: id.map(str::to_string),
        name: name.into(),
        config: config.into(),
    };
    let household = || SpaceRef {
        space: "household".into(),
    };
    let before = n.api.file_analysis(&OWNER, household()).await.unwrap();
    assert!(before.tables[0].views.is_empty());
    let kept = n
        .api
        .save_analysis_view(
            &OWNER,
            save(
                None,
                " Groceries by month ",
                r#"{"chart":"column","time":{"column":"date","grain":"month"},"metrics":[{"column":"amount","aggregate":"sum"}]}"#,
            ),
        )
        .await
        .unwrap();
    let views = &kept.tables[0].views;
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Groceries by month");
    assert_eq!(views[0].by, None);
    let kept_spec: ChartSpec = serde_json::from_str(&views[0].config).unwrap();
    assert_eq!(kept_spec.chart, ChartKind::Column);
    assert_eq!(kept_spec.time.unwrap().grain, DateGrain::Month);
    let id = views[0].id.clone();
    // Another view may not take its name; what is not a chart is not kept.
    let err = n
        .api
        .save_analysis_view(
            &OWNER,
            save(None, "Groceries by month", r#"{"chart":"bar"}"#),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
    for bad in [
        "[]",
        "not json",
        "{}",
        r#"{"chart":"bar","sql":"SELECT 1"}"#,
        r#"{"chart":"bar","drill":["2026"]}"#,
    ] {
        let err = n
            .api
            .save_analysis_view(&OWNER, save(None, "Other", bad))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Invalid, "{bad}: {err:?}");
    }
    assert!(n
        .api
        .save_analysis_view(&OWNER, save(None, "  ", r#"{"chart":"bar"}"#))
        .await
        .is_err());
    let kept = n
        .api
        .save_analysis_view(&OWNER, save(Some(&id), "By month", r#"{"chart":"line"}"#))
        .await
        .unwrap();
    assert_eq!(kept.tables[0].views.len(), 1);
    assert_eq!(kept.tables[0].views[0].name, "By month");
    // An id the table has not is not found: there are no views it starts with.
    for unknown in ["month", "v42"] {
        let err = n
            .api
            .save_analysis_view(&OWNER, save(Some(unknown), "Other", r#"{"chart":"bar"}"#))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound, "{err:?}");
    }
    let view = |id: &str| AnalysisViewRef {
        space: "household".into(),
        table: "spending".into(),
        id: id.into(),
    };

    // The table is made again when its files change; its views stay.
    std::fs::write(
        root.join("spending/2026-11.csv"),
        "date,category,amount\n2026-11-01,Rent,900.00\n",
    )
    .unwrap();
    n.api.refresh_space(&OWNER, household()).await.unwrap();
    let again = n.api.file_analysis(&OWNER, household()).await.unwrap();
    assert_eq!(again.tables[0].rows, 7);
    assert_eq!(again.tables[0].views.len(), 1);
    let err = n
        .api
        .remove_analysis_view(&OWNER, view("v9"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err:?}");
    let gone = n.api.remove_analysis_view(&OWNER, view(&id)).await.unwrap();
    assert!(gone.tables[0].views.is_empty());
}

/// An assistant charts the analysis tables of its collections as itself,
/// and may keep a chart for the owner, marked as its own; nothing else.
#[tokio::test]
async fn an_assistant_charts_its_collections_and_keeps_charts_marked_as_its_own() {
    let n = logged_node();
    let source = tempfile::tempdir().unwrap();
    for name in ["household", "private"] {
        let root = source.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("spending.csv"),
            "date,category,amount\n2026-09-02,Groceries,30.50\n2026-09-14,Books,12.00\n2026-10-03,Groceries,20.00\n",
        )
        .unwrap();
        n.api
            .view_in_place(
                &OWNER,
                ViewInPlace {
                    place: None,
                    path: root.to_string_lossy().into_owned(),
                    name: Some(name.into()),
                },
            )
            .await
            .unwrap();
        n.api
            .build_analysis(
                &OWNER,
                BuildAnalysis {
                    space: name.into(),
                    tables: vec!["spending".into()],
                },
            )
            .await
            .unwrap();
    }
    let connected = n
        .api
        .connect_assistant(
            &OWNER,
            ConnectAssistant {
                name: "Claude Code".into(),
                app: AssistantApp::ClaudeCode,
                plugin: None,
                client: None,
                collections: vec!["household".into()],
            },
        )
        .await
        .unwrap();
    let me = n.api.assistant_by_token(&connected.token).unwrap();
    let spec = |json: serde_json::Value| -> ChartSpec { serde_json::from_value(json).unwrap() };
    let by_month = || {
        spec(serde_json::json!({
            "chart": "column",
            "time": {"column": "date", "grain": "month"},
            "metrics": [{"column": "amount", "aggregate": "sum"}]
        }))
    };

    // What it may chart: its collection's analysis tables, with what each
    // column holds and is for.
    let tables = n
        .api
        .assistant_charts(&me, ListSkills { collection: None })
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(
        (
            tables[0].collection.as_str(),
            tables[0].table.as_str(),
            tables[0].rows
        ),
        ("household", "spending", 3)
    );
    let kinds: Vec<(&str, &str)> = tables[0]
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.kind.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [("date", "time"), ("category", "text"), ("amount", "number")]
    );
    assert!(n
        .api
        .assistant_charts(
            &me,
            ListSkills {
                collection: Some("private".into())
            }
        )
        .await
        .is_err());

    // A chart, computed as itself.
    let chart = |collection: &str, spec: ChartSpec| AssistantChart {
        collection: collection.into(),
        table: "spending".into(),
        spec,
    };
    let result = n
        .api
        .assistant_chart(&me, chart("household", by_month()))
        .await
        .unwrap();
    let totals: Vec<_> = result.groups.iter().map(|g| g.values[0]).collect();
    assert_eq!(totals, [Some(42.5), Some(20.0)]);
    let err = n
        .api
        .assistant_chart(&me, chart("private", by_month()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err:?}");
    // The owner is not an assistant here, nor is an assistant the owner.
    assert!(n
        .api
        .assistant_chart(&OWNER, chart("household", by_month()))
        .await
        .is_err());
    assert!(n
        .api
        .analysis_chart(
            &me,
            AnalysisChart {
                space: "household".into(),
                table: "spending".into(),
                spec: by_month()
            }
        )
        .await
        .is_err());

    // Kept, marked as its own; saved again under its name, replaced; a
    // chart it can't compute, or another's name, is not kept.
    let save = |collection: &str, name: &str, spec: ChartSpec| AssistantSaveChart {
        collection: collection.into(),
        table: "spending".into(),
        name: name.into(),
        spec,
    };
    let saved = n
        .api
        .assistant_save_chart(&me, save("household", "Spending by month", by_month()))
        .await
        .unwrap();
    assert_eq!(saved.by.as_deref(), Some("Claude Code"));
    let again = n
        .api
        .assistant_save_chart(
            &me,
            save(
                "household",
                "Spending by month",
                spec(serde_json::json!({"chart": "line", "time": {"column": "date", "grain": "month"}})),
            ),
        )
        .await
        .unwrap();
    assert_eq!(again.id, saved.id);
    for refused in [
        save(
            "household",
            "Nothing",
            spec(serde_json::json!({"chart": "bar", "dimensions": [{"column": "nothing"}]})),
        ),
        save("private", "Elsewhere", by_month()),
    ] {
        assert!(n.api.assistant_save_chart(&me, refused).await.is_err());
    }
    n.api
        .save_analysis_view(
            &OWNER,
            SaveAnalysisView {
                space: "household".into(),
                table: "spending".into(),
                id: None,
                name: "Mine".into(),
                config: r#"{"chart":"bar"}"#.into(),
            },
        )
        .await
        .unwrap();
    let err = n
        .api
        .assistant_save_chart(&me, save("household", "Mine", by_month()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
    let analysis = n
        .api
        .file_analysis(
            &OWNER,
            SpaceRef {
                space: "household".into(),
            },
        )
        .await
        .unwrap();
    let views: Vec<(&str, Option<&str>)> = analysis.tables[0]
        .views
        .iter()
        .map(|v| (v.name.as_str(), v.by.as_deref()))
        .collect();
    assert_eq!(
        views,
        [("Spending by month", Some("Claude Code")), ("Mine", None)]
    );
    // Changed and saved by the owner, it is the owner's.
    let owned = n
        .api
        .save_analysis_view(
            &OWNER,
            SaveAnalysisView {
                space: "household".into(),
                table: "spending".into(),
                id: Some(saved.id.clone()),
                name: "Spending by month".into(),
                config: r#"{"chart":"area"}"#.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(owned.tables[0].views[0].by, None);

    // All of it is in Activity: the charts, the one kept, the refusals.
    let log = n
        .api
        .list_access(&OWNER, ListAccess::default())
        .await
        .unwrap();
    let methods: Vec<(&str, String)> = log
        .iter()
        .filter(|item| item.method.starts_with("assistant_"))
        .map(|item| (item.method.as_str(), format!("{:?}", item.outcome)))
        .collect();
    assert!(
        methods.contains(&("assistant_chart", "Allowed".into())),
        "{methods:?}"
    );
    assert!(
        methods.contains(&("assistant_chart", "Denied".into())),
        "{methods:?}"
    );
    assert!(
        methods.contains(&("assistant_save_chart", "Allowed".into())),
        "{methods:?}"
    );
    assert!(
        methods.contains(&("assistant_save_chart", "Denied".into())),
        "{methods:?}"
    );
    let kept = log
        .iter()
        .find(|item| {
            item.method == "assistant_save_chart" && item.outcome == AccessOutcome::Allowed
        })
        .unwrap();
    assert_eq!(kept.detail.as_deref(), Some("Spending by month"));
}

/// A node that loads DuckDB's extensions, shared by the tests that need
/// them and downloaded on first use.
fn node_with_extensions() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let config = NodeConfig {
        duckdb: oikome_engine_duckdb::DuckDbConfig {
            extension_dir: Some(
                std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckdb-extensions"),
            ),
            allow_extension_install: true,
            ..Default::default()
        },
        ..test_config()
    };
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), config).unwrap());
    let api = Api::new(catalog.clone(), engines.clone());
    Node {
        _dir: dir,
        catalog,
        engines,
        api,
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

async fn count_of(n: &Node, table: &str) -> i64 {
    let read = sql(n, &format!("SELECT CAST(count(*) AS BIGINT) FROM {table}")).await;
    let SqlResult::Rows(rows) = read else {
        panic!("expected rows, got {read:?}");
    };
    StreamReader::try_new(rows.ipc.as_slice(), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap()
        .value(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn datasets_in_a_folder_are_analysis_tables_read_in_place() {
    let n = node_with_extensions();
    // Another program's Iceberg table and Lance dataset: this node's own,
    // as Iceberg and Lance write them, copied into the owner's folder.
    sql(
        &n,
        "CREATE SPACE shop; CREATE TABLE shop.orders (id BIGINT, item VARCHAR, amount DOUBLE); \
         INSERT INTO shop.orders VALUES (1, 'tea', 3.5), (2, 'rice', 2.0); \
         INSERT INTO shop.orders VALUES (3, 'salt', 1.0)",
    )
    .await;
    sql(
        &n,
        "CREATE SPACE notes WITH FORMAT = 'lance'; \
         CREATE TABLE notes.items (id BIGINT, title VARCHAR); \
         INSERT INTO notes.items VALUES (1, 'a'), (2, 'b')",
    )
    .await;
    let data = std::path::PathBuf::from(n.catalog.root_path());
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("lab");
    copy_tree(&data.join("shop/orders"), &root.join("exports/orders"));
    copy_tree(
        &data.join(".oikome/lance/notes/items.lance"),
        &root.join("vectors/items.lance"),
    );
    std::fs::write(root.join("readings.csv"), "day,kwh\n1,3\n2,4\n").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("lab".into()),
            },
        )
        .await
        .unwrap();
    let lab = || SpaceRef {
        space: "lab".into(),
    };

    // Each dataset is one item of its folder, and opens as a table.
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "lab".into(),
                folder: Some("exports".into()),
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
    assert_eq!(listed.files.len(), 1, "{:?}", listed.files);
    let orders = &listed.files[0];
    assert_eq!(orders.kind, FileKind::Dataset);
    assert_eq!(orders.content.format, "iceberg");
    assert_eq!(orders.content.views, [ContentView::Table]);
    let rows = n
        .api
        .read_file_rows(
            &OWNER,
            ReadFileRows {
                space: "lab".into(),
                path: "exports/orders".into(),
                limit: Some(2),
                offset: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(row_count(&rows), 2);

    // Data Analysis reads the datasets where they are without being asked;
    // the CSV's table waits until the owner makes it.
    let found = n.api.file_analysis(&OWNER, lab()).await.unwrap();
    let sources: Vec<_> = found.sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(sources, ["readings"]);
    let mut tables: Vec<_> = found.tables.iter().map(|t| t.name.as_str()).collect();
    tables.sort();
    assert_eq!(tables, ["items", "orders"]);
    let made = n
        .api
        .build_analysis(
            &OWNER,
            BuildAnalysis {
                space: "lab".into(),
                tables: vec!["readings".into()],
            },
        )
        .await
        .unwrap();
    assert!(made.sources.is_empty(), "{:?}", made.sources);
    let err = n
        .api
        .remove_analysis_table(
            &OWNER,
            AnalysisTableRef {
                space: "lab".into(),
                table: "orders".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Invalid, "a dataset is always analysed");
    assert_eq!(made.tables.len(), 3);
    let table = |name: &str| made.tables.iter().find(|t| t.name == name).unwrap().clone();
    let orders = table("orders");
    assert_eq!(orders.problem, None);
    assert_eq!(orders.rows, 3);
    let facts = orders.dataset.clone().unwrap();
    assert_eq!(facts.kind, DatasetKind::Iceberg);
    assert!(facts.version.is_some() && facts.written_at_ms.is_some());
    assert_eq!(facts.extension, None);
    assert!(orders.fields.iter().any(|f| f.name == "amount"));
    assert_eq!(table("items").rows, 2);
    assert_eq!(
        table("items").dataset.unwrap().version.as_deref(),
        Some("2")
    );
    let summary = n
        .api
        .analysis_summary(
            &OWNER,
            AnalysisTableRef {
                space: "lab".into(),
                table: "orders".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(summary.amount.as_deref(), Some("amount"));
    // Nothing was made in Iceberg for them: they are read where they are,
    // values and all.
    assert_eq!(count_of(&n, "lab.orders").await, 3);
    assert_eq!(
        count_of(&n, "lab.orders WHERE item = 'tea' AND amount = 3.5").await,
        1
    );
}

/// A camera's RAW file as TIFF writes it: one directory that says how the
/// picture is turned and where the stored JPEG is, then the JPEG.
fn raw_file(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let entry = |tag: u16, kind: u16, value: u32| {
        let mut out = tag.to_le_bytes().to_vec();
        out.extend(kind.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(value.to_le_bytes());
        out
    };
    // Header (8), a directory of three entries (2 + 36 + 4), the JPEG.
    let at = 50u32;
    let mut file = b"II\x2A\x00\x08\x00\x00\x00\x03\x00".to_vec();
    file.extend(entry(0x0112, 3, u32::from(orientation)));
    file.extend(entry(0x0201, 4, at));
    file.extend(entry(0x0202, 4, jpeg.len() as u32));
    file.extend(0u32.to_le_bytes());
    assert_eq!(file.len() as u32, at);
    file.extend(jpeg);
    file
}

#[tokio::test(flavor = "multi_thread")]
async fn a_raw_file_is_shown_by_an_installed_preview_plugin_in_the_sandbox() {
    let n = node();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("shoot");
    std::fs::create_dir_all(&root).unwrap();
    // The picture the camera stored: wider than tall, to be turned on its side.
    let mut stored = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        48,
        image::Rgb([200, 80, 40]),
    ))
    .write_to(
        &mut std::io::Cursor::new(&mut stored),
        image::ImageFormat::Jpeg,
    )
    .unwrap();
    let raw = raw_file(&stored, 6);
    std::fs::write(root.join("IMG_0001.DNG"), &raw).unwrap();
    std::fs::write(
        root.join("broken.nef"),
        b"II\x2A\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00",
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "not a picture").unwrap();
    n.api
        .view_in_place(
            &OWNER,
            ViewInPlace {
                place: None,
                path: root.to_string_lossy().into_owned(),
                name: Some("shoot".into()),
            },
        )
        .await
        .unwrap();
    let file = |path: &str| FileRef {
        space: "shoot".into(),
        path: path.into(),
    };

    // A preview plugin is a package with code to run, installed like any
    // other (RAW Preview is one, in the oikome-plugins repository; this is
    // the tests' own). Before it, a RAW file has no preview.
    let before = n
        .api
        .file_preview(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap_err();
    assert_eq!(before.code, ErrorCode::NotFound);
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oikome-sandbox/tests/plugins/stored-picture");
    n.api
        .install_plugin(
            &OWNER,
            InstallPlugin {
                built_in: None,
                path: Some(package.to_string_lossy().into_owned()),
            },
        )
        .await
        .unwrap();
    n.api
        .refresh_space(
            &OWNER,
            SpaceRef {
                space: "shoot".into(),
            },
        )
        .await
        .unwrap();
    let plugins = n.api.list_plugins(&OWNER, Empty {}).await.unwrap();
    assert!(
        plugins.iter().all(|p| p.id != "raw-preview"),
        "RAW Preview is not built in"
    );
    let plugin = plugins.iter().find(|p| p.id == "stored-picture").unwrap();
    assert!(plugin.installed);
    assert_eq!(
        plugin.capabilities.preview[0].entry.as_deref(),
        Some("preview.wasm")
    );

    // The stored picture, as it is, with how to turn it said in it.
    let preview = n
        .api
        .file_preview(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap();
    assert_eq!(preview.content_type, "image/jpeg");
    assert!(preview.bytes.ends_with(&stored[2..]));
    assert!(
        preview.bytes.len() > stored.len(),
        "its orientation was put in"
    );
    // Its thumbnail is turned: taller than wide.
    let thumb = n
        .api
        .file_thumbnail(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap();
    let small = image::load_from_memory(&thumb).unwrap();
    assert_eq!(
        small.width() * 4,
        small.height() * 3,
        "64 by 48, on its side"
    );
    // The listing says which plugin shows it; a file no plugin shows has none.
    let listed = n
        .api
        .list_files(
            &OWNER,
            ListFiles {
                space: "shoot".into(),
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
    let preview_of = |name: &str| {
        listed
            .files
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .preview
            .clone()
    };
    assert_eq!(
        preview_of("IMG_0001.DNG").as_deref(),
        Some("stored-picture")
    );
    assert_eq!(preview_of("notes.txt"), None);
    // The RAW file was only read.
    assert_eq!(std::fs::read(root.join("IMG_0001.DNG")).unwrap(), raw);
    // Kept for the next time, and given again from there.
    let kept = n.catalog.oikome_dir().join("previews");
    assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 2);
    let again = n
        .api
        .file_preview(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap();
    assert_eq!(again.bytes, preview.bytes);

    // A RAW file with no picture in it says so; other files have no preview.
    let none = n
        .api
        .file_preview(&OWNER, file("broken.nef"))
        .await
        .unwrap_err();
    assert_eq!(none.code, ErrorCode::Invalid, "{none:?}");
    assert!(none.message.contains("holds no preview image"), "{none:?}");
    let other = n
        .api
        .file_preview(&OWNER, file("notes.txt"))
        .await
        .unwrap_err();
    assert_eq!(other.code, ErrorCode::NotFound);
    assert!(n
        .api
        .file_thumbnail(&OWNER, file("notes.txt"))
        .await
        .is_err());
    assert!(n
        .api
        .file_preview(&Caller::Principal { did: did(9) }, file("IMG_0001.DNG"))
        .await
        .is_err());

    // The file changes: its preview is made again.
    let mut bigger = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        96,
        32,
        image::Rgb([10, 90, 160]),
    ))
    .write_to(
        &mut std::io::Cursor::new(&mut bigger),
        image::ImageFormat::Jpeg,
    )
    .unwrap();
    std::fs::write(root.join("IMG_0001.DNG"), raw_file(&bigger, 1)).unwrap();
    n.api
        .refresh_space(
            &OWNER,
            SpaceRef {
                space: "shoot".into(),
            },
        )
        .await
        .unwrap();
    let changed = n
        .api
        .file_preview(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap();
    assert_eq!(changed.bytes, bigger, "upright already: left as stored");

    // Without the plugin a RAW file has no preview.
    n.api
        .remove_plugin(
            &OWNER,
            PluginRef {
                id: "stored-picture".into(),
            },
        )
        .await
        .unwrap();
    let gone = n
        .api
        .file_preview(&OWNER, file("IMG_0001.DNG"))
        .await
        .unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
}
