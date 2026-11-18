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

//! Calls that arrive over a browser's WebRTC channel as envelopes go through
//! the same authentication, authorization and semantic layer as Arrow
//! Flight calls (§10).

use crate::common;

use arrow_array::{Array, Int64Array};
use common::{strings, Node};
use oikome_api::{
    Api, Caller, NewPublicShare, PublicShareRef, SaveDestination, SaveTo, ViewInPlace,
};
use oikome_identity::{unix_now, unix_now_ms, AuthRequest, NodeIdentity};
use oikome_network::protocol::webrtc::{decode_reply, AccessRecord, Envelope};
use oikome_network::protocol::{
    decode_stream, digest, schema_payload, ListTablesResponse, QueryTicket, METHOD_DO_GET,
    METHOD_GET_SCHEMA, METHOD_LIST_TABLES, METHOD_PUBLIC_CHALLENGE, METHOD_PUBLIC_QUERY,
    METHOD_PUBLIC_VERIFY,
};
use oikome_server::OikomeFlightService;
use oikome_webrtc::{DtlsIdentity, Handler};

/// An envelope for `method` signed by `caller` for the node `server`.
fn envelope(
    caller: &NodeIdentity,
    server: &Node,
    method: &str,
    payload: &[u8],
    ucan: &str,
) -> Envelope {
    let auth = AuthRequest::sign(
        caller,
        server.identity.did(),
        unix_now_ms(),
        &digest(method, payload, ucan),
    )
    .unwrap()
    .to_header()
    .unwrap();
    Envelope::new(method, payload, ucan, &auth)
}

fn ticket(sql: &str) -> Vec<u8> {
    serde_json::to_vec(&QueryTicket {
        sql: sql.to_string(),
    })
    .unwrap()
}

/// The reply status and body of a call.
async fn call(service: &OikomeFlightService, envelope: Envelope) -> (String, Vec<u8>) {
    let reply = service.call(envelope).await;
    let (header, body) = decode_reply(&reply).unwrap();
    (header.status, body.to_vec())
}

fn ids(ipc: Vec<u8>) -> Vec<i64> {
    let (_, batches) = decode_stream(ipc).unwrap();
    batches
        .iter()
        .flat_map(|b| {
            let ids = b.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            (0..ids.len()).map(|i| ids.value(i)).collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn envelopes_are_authenticated_authorized_and_answered_in_arrow() {
    let a = Node::new(21);
    let browser = NodeIdentity::from_seed(&[22; 32]);
    let grantee = browser.did().to_string();
    for sql in [
        "CREATE SPACE blog".to_string(),
        "CREATE TABLE blog.posts (id BIGINT NOT NULL, author VARCHAR, draft BOOLEAN)".to_string(),
        "INSERT INTO blog.posts VALUES (1, 'alice', false), (2, 'bob', true), (3, 'carol', false)"
            .to_string(),
        "CREATE TABLE blog.secrets (k VARCHAR)".to_string(),
        format!("GRANT SELECT (id, author) ON blog.posts TO \"{grantee}\""),
        format!("CREATE POLICY published ON blog.posts TO \"{grantee}\" USING (NOT draft)"),
    ] {
        a.sql(&sql).await;
    }
    let (id, token) = a
        .issue(&format!("ISSUE UCAN ON SPACE blog TO '{grantee}'"))
        .await;
    let service = OikomeFlightService::new(a.catalog.clone(), a.engines.clone()).unwrap();

    // A query runs as the grantee: its row policy and column grants apply.
    let query = ticket("SELECT id FROM blog.posts ORDER BY id");
    let (status, body) = call(
        &service,
        envelope(&browser, &a, METHOD_DO_GET, &query, &token),
    )
    .await;
    assert_eq!(status, "ok");
    assert_eq!(ids(body), vec![1, 3]);

    let (status, body) = call(
        &service,
        envelope(
            &browser,
            &a,
            METHOD_GET_SCHEMA,
            &schema_payload("blog", "posts"),
            &token,
        ),
    )
    .await;
    assert_eq!(status, "ok");
    let (schema, batches) = decode_stream(body).unwrap();
    let columns: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(columns, vec!["id", "author"]);
    assert!(batches.is_empty());

    let (status, body) = call(
        &service,
        envelope(
            &browser,
            &a,
            METHOD_LIST_TABLES,
            br#"{"space":"blog"}"#,
            &token,
        ),
    )
    .await;
    assert_eq!(status, "ok");
    let listed: ListTablesResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(listed.tables, vec!["posts"]);

    // The same envelope twice is a replay.
    let once = envelope(&browser, &a, METHOD_DO_GET, &query, &token);
    assert_eq!(call(&service, once.clone()).await.0, "ok");
    assert_eq!(call(&service, once).await.0, "unauthenticated");

    // A signature over another payload does not authenticate this one.
    let mut swapped = envelope(&browser, &a, METHOD_DO_GET, &query, &token);
    swapped.payload = Envelope::new(METHOD_DO_GET, &ticket("SELECT 1"), "", "").payload;
    assert_eq!(call(&service, swapped).await.0, "unauthenticated");

    // Another caller cannot use the browser's token.
    let other = NodeIdentity::from_seed(&[23; 32]);
    let (status, _) = call(
        &service,
        envelope(&other, &a, METHOD_DO_GET, &query, &token),
    )
    .await;
    assert_eq!(status, "permission_denied");

    // Revocation takes effect on the next call.
    a.sql(&format!("REVOKE UCAN '{id}'")).await;
    let (status, _) = call(
        &service,
        envelope(&browser, &a, METHOD_DO_GET, &query, &token),
    )
    .await;
    assert_eq!(status, "permission_denied");

    let (status, _) = call(
        &service,
        envelope(&browser, &a, "do_action:oikome.snapshot", b"{}", &token),
    )
    .await;
    assert_eq!(status, "unimplemented");
}

#[tokio::test]
async fn public_enquiries_work_without_opening_private_peer_methods() {
    let a = Node::new(25);
    let api = Api::new(a.catalog.clone(), a.engines.clone());
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("note.txt"), "public").unwrap();
    api.view_in_place(
        &Caller::Owner,
        ViewInPlace {
            place: None,
            path: source.to_string_lossy().into_owned(),
            name: Some("source".into()),
            files: None,
        },
    )
    .await
    .unwrap();
    api.save_to(
        &Caller::Owner,
        SaveTo {
            source: "source".into(),
            paths: None,
            destination: SaveDestination::Space {
                name: "published".into(),
                storage: None,
                title: "Published".into(),
                cover: None,
                captions: Default::default(),
            },
        },
    )
    .await
    .unwrap();
    let share = api
        .create_public_share(
            &Caller::Owner,
            NewPublicShare {
                space: "published".into(),
                allow_copy: Some(false),
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
    let service = OikomeFlightService::new(a.catalog.clone(), a.engines.clone())
        .unwrap()
        .with_public_api(api.clone());
    let visitor = NodeIdentity::from_seed(&[26; 32]);
    let public_call = |method: &str, value: serde_json::Value| {
        Envelope::new(method, &serde_json::to_vec(&value).unwrap(), "", "")
    };
    let (status, body) = call(
        &service,
        public_call(
            METHOD_PUBLIC_CHALLENGE,
            serde_json::json!({"id": share.id, "did": visitor.did().to_string()}),
        ),
    )
    .await;
    assert_eq!(status, "ok");
    let challenge: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let message = challenge["message"].as_str().unwrap();
    let (status, body) = call(
        &service,
        public_call(
            METHOD_PUBLIC_VERIFY,
            serde_json::json!({
                "id": share.id,
                "did": visitor.did().to_string(),
                "message": message,
                "signature": hex::encode(visitor.sign(message.as_bytes())),
            }),
        ),
    )
    .await;
    assert_eq!(status, "ok");
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let query = public_call(
        METHOD_PUBLIC_QUERY,
        serde_json::json!({
            "id": share.id,
            "token": session["token"],
            "sql": "SELECT path FROM _oikome_files",
        }),
    );
    let (status, body) = call(&service, query.clone()).await;
    assert_eq!(status, "ok");
    assert_eq!(decode_stream(body).unwrap().1[0].num_rows(), 1);
    let private = Envelope::new(
        METHOD_DO_GET,
        &ticket("SELECT * FROM published._oikome_files"),
        "",
        "",
    );
    assert_eq!(call(&service, private).await.0, "unauthenticated");
    api.revoke_public_share(
        &Caller::Owner,
        PublicShareRef {
            id: share.id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(call(&service, query).await.0, "unauthenticated");
}

#[tokio::test]
async fn show_webrtc_reads_the_record_the_server_published() {
    let a = Node::new(24);
    let err = a.sql_err("SHOW WEBRTC").await;
    assert!(err.contains("--webrtc-listen"), "{err}");

    let dir = tempfile::tempdir().unwrap();
    let dtls = DtlsIdentity::load_or_create(&dir.path().join("dtls.json")).unwrap();
    let record = dtls
        .access_record(
            &a.identity,
            "127.0.0.1:4433".parse().unwrap(),
            std::time::Duration::from_secs(3600),
        )
        .unwrap();
    std::fs::write(a.catalog.webrtc_record_path(), &record).unwrap();

    let shown = a.sql("SHOW WEBRTC").await;
    let text = strings(&shown, 0).remove(0);
    let verified = AccessRecord::verify(&text, unix_now()).unwrap();
    assert_eq!(verified.did, a.did());
    assert_eq!(verified.addr, "127.0.0.1:4433");
    assert_eq!(verified.fingerprint, dtls.fingerprint());
}
