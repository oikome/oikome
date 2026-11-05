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

//! The access log and whole-Space grants between two nodes (M1b): every
//! peer request is recorded with its caller, Spaces and token, refusals
//! included; a whole-Space grant lets a peer copy tables created after it.

use crate::common;

use std::time::Duration;

use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::Action;
use common::{ints, strings, Node};
use oikome_node::{AccessQuery, AccessRecord, Outcome};

/// The node's access log once it holds at least `rows` rows.
async fn log_rows(node: &Node, rows: usize) -> Vec<AccessRecord> {
    for _ in 0..200 {
        let found = node
            .access_log
            .list(&AccessQuery {
                limit: 500,
                ..Default::default()
            })
            .unwrap();
        if found.len() >= rows {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the access log never held {rows} rows");
}

async fn seed(a: &Node, grantee: &str) {
    for sql in [
        "CREATE SPACE blog".to_string(),
        "CREATE TABLE blog.posts (id BIGINT NOT NULL, title VARCHAR)".to_string(),
        "INSERT INTO blog.posts VALUES (1, 'Hello'), (2, 'World')".to_string(),
        format!("GRANT SELECT ON SPACE blog TO \"{grantee}\""),
    ] {
        a.sql(&sql).await;
    }
}

#[tokio::test]
async fn peer_requests_are_recorded_allowed_or_denied() {
    let a = Node::new(31).serve().await;
    let b = Node::new(32);
    seed(&a, &b.did()).await;
    let (id, token) = a
        .issue(&format!(
            "ISSUE UCAN ON SPACE blog TO '{}' EXPIRES IN '1h'",
            b.did()
        ))
        .await;
    b.sql(&format!(
        "CREATE SPACE pub WITH PEER = '{}', REMOTE = 'blog', TOKEN = '{token}'",
        a.url()
    ))
    .await;
    assert_eq!(
        ints(&b.sql("SELECT id FROM pub.posts ORDER BY id").await, 0),
        vec![1, 2]
    );
    a.sql(&format!("REVOKE UCAN '{id}'")).await;
    assert!(b
        .sql_err("SELECT id FROM pub.posts")
        .await
        .contains("permission denied"));

    // Wait for the refusal, the newest entry.
    let mut rows = log_rows(&a, 2).await;
    for _ in 0..200 {
        if rows.iter().any(|r| r.outcome == Outcome::Denied) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        rows = log_rows(&a, 2).await;
    }
    assert!(rows
        .iter()
        .all(|r| r.caller.as_deref() == Some(b.did().as_str())));
    assert!(rows.iter().all(|r| r.spaces == vec!["blog".to_string()]));
    let allowed: Vec<_> = rows
        .iter()
        .filter(|r| r.outcome == Outcome::Allowed)
        .collect();
    assert!(allowed.iter().any(|r| r.method == "do_get"));
    assert!(allowed
        .iter()
        .all(|r| r.token_id.as_deref() == Some(id.as_str())));
    let denied = rows
        .iter()
        .find(|r| r.outcome == Outcome::Denied)
        .expect("the refused query is recorded");
    assert!(denied.reason.as_deref().unwrap_or("").contains("revoked"));
}

#[tokio::test]
async fn failed_authentications_are_merged_into_one_row() {
    let a = Node::allowing_http(33).serve_plaintext().await;
    let channel = tonic::transport::Channel::from_shared(a.url().to_string())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = FlightServiceClient::new(channel);
    for _ in 0..3 {
        let status = client
            .do_action(Action {
                r#type: "oikome.list_tables".to_string(),
                body: br#"{"space":"blog"}"#.to_vec().into(),
            })
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
    }
    for _ in 0..200 {
        let rows = log_rows(&a, 1).await;
        if rows.len() == 1 && rows[0].count == 3 {
            assert_eq!(rows[0].caller, None);
            assert_eq!(rows[0].outcome, Outcome::Denied);
            assert_eq!(rows[0].reason.as_deref(), Some("missing UCAN"));
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("three failed authentications were not merged into one row");
}

#[tokio::test]
async fn a_space_grant_lets_a_subscriber_copy_tables_created_later() {
    let a = Node::new(34).serve().await;
    let b = Node::new(35);
    seed(&a, &b.did()).await;
    let (_, token) = a
        .issue(&format!(
            "ISSUE UCAN ON SPACE blog TO '{}' CAN 'space/read'",
            b.did()
        ))
        .await;
    let report = b
        .sql(&format!(
            "CREATE SPACE copy WITH PEER = '{}', REMOTE = 'blog', TOKEN = '{token}', MODE = 'subscribe'",
            a.url()
        ))
        .await;
    assert_eq!(strings(&report, 0), vec!["posts"]);

    a.sql("CREATE TABLE blog.tags (post BIGINT NOT NULL, tag VARCHAR)")
        .await;
    a.sql("INSERT INTO blog.tags VALUES (1, 'intro')").await;
    b.sql("REFRESH SPACE copy").await;
    assert_eq!(
        strings(&b.sql("SELECT tag FROM copy.tags").await, 0),
        vec!["intro"]
    );
}
