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

//! Compute sharing (3.6 exit criteria): node A runs tasks for node B within
//! B's quota; a task over its limits is refused; revoking B's grant drops
//! its queued tasks; B's compute grant gives no access to A's Spaces, and a
//! data grant does not let B run tasks.

use crate::common;

use std::time::Duration;

use common::{ints, Node};
use oikome_node::{OikomeSession, QueryResult, SessionConfig, SessionError};

/// B's local data: 10 numbers, and a table too large for a 1 MB input.
async fn seed(b: &Node) {
    for sql in [
        "CREATE SPACE nums",
        "CREATE TABLE nums.small (v BIGINT)",
        "INSERT INTO nums.small SELECT i FROM (SELECT unnest(range(1, 11)) AS i) t",
        "CREATE TABLE nums.large (v BIGINT)",
        "INSERT INTO nums.large SELECT i FROM (SELECT unnest(range(200000)) AS i) t",
    ] {
        b.sql(sql).await;
    }
}

async fn run(
    b: &OikomeSession,
    a: &Node,
    token: &str,
    input: &str,
    sql: &str,
) -> Result<QueryResult, SessionError> {
    b.compute_on_peer(Some(a.url()), &a.did(), token, "sql", input, sql)
        .await
}

fn grant(b: &Node, limits: &str) -> String {
    format!(
        "ISSUE UCAN ON COMPUTE 'sql' TO '{}' LIMITS ({limits})",
        b.did()
    )
}

#[tokio::test]
async fn a_peer_runs_tasks_within_its_limits_and_nothing_else() {
    let a = Node::new(71).serve().await;
    let b = Node::new(72);
    seed(&b).await;
    let small = "SELECT v FROM nums.small";
    let (_, token) = a
        .issue(&grant(
            &b,
            "time = '20s', input = '1MB', output = '1MB', memory = '256MB'",
        ))
        .await;

    // Offered methods only.
    let e = run(&b.owner, &a, &token, small, "SELECT count(*) FROM input")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("permission denied"), "{e}");
    a.owner.offer_compute("sql").unwrap();

    // Within its limits: the sum, computed on A over B's input.
    let total = run(
        &b.owner,
        &a,
        &token,
        small,
        "SELECT CAST(sum(v) AS BIGINT) AS total FROM input",
    )
    .await
    .unwrap();
    assert_eq!(ints(&total, 0), vec![55]);

    // Over its limits: input, output, then time.
    let e = run(
        &b.owner,
        &a,
        &token,
        "SELECT v FROM nums.large",
        "SELECT count(*) FROM input",
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("bytes allowed"), "{e}");
    let e = run(
        &b.owner,
        &a,
        &token,
        small,
        "SELECT v, repeat('x', 200000) AS padding FROM input",
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("bytes allowed"), "{e}");
    let (_, quick) = a.issue(&grant(&b, "time = '300ms'")).await;
    let e = run(
        &b.owner,
        &a,
        &quick,
        small,
        "SELECT count(*) FROM range(100000000000) r",
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("longer than"), "{e}");

    // The task sees only its input: not A's Spaces, not A's files.
    a.sql("CREATE SPACE private").await;
    a.sql("CREATE TABLE private.notes (body VARCHAR)").await;
    a.sql("INSERT INTO private.notes VALUES ('mine')").await;
    for (sql, refusal) in [
        ("SELECT * FROM private.notes", "does not exist"),
        (
            "SELECT * FROM read_csv('/etc/hosts')",
            "disabled by configuration",
        ),
    ] {
        let e = run(&b.owner, &a, &token, small, sql).await.unwrap_err();
        assert!(e.to_string().contains(refusal), "{sql}: {e}");
    }

    // A compute grant reads no Space ...
    let e = b
        .owner
        .sql(&format!(
            "CREATE SPACE theirs WITH PEER = '{}', TOKEN = '{token}', REMOTE = 'private'",
            a.url()
        ))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("does not grant space/query"), "{e}");
    // ... and a data grant runs no task.
    let (_, data) = a
        .issue(&format!("ISSUE UCAN ON SPACE private TO '{}'", b.did()))
        .await;
    let e = run(&b.owner, &a, &data, small, "SELECT 1")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("permission denied"), "{e}");
}

#[tokio::test]
async fn quotas_hold_and_revoking_drops_queued_tasks() {
    let a = Node::new(73).serve().await;
    let b = Node::new(74);
    seed(&b).await;
    a.owner.offer_compute("sql").unwrap();
    a.owner.set_compute_quota(&b.did(), 1, 100).unwrap();
    let (id, token) = a.issue(&grant(&b, "time = '2s'")).await;
    let second = OikomeSession::new(
        b.catalog.clone(),
        b.engines.clone(),
        SessionConfig::default(),
    );
    let small = "SELECT v FROM nums.small";

    // One task at a time for B: the second waits for the first, and the
    // grant is revoked while it waits.
    let slow = run(
        &b.owner,
        &a,
        &token,
        small,
        "SELECT count(*) FROM range(100000000000) r",
    );
    let waiting = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        run(&second, &a, &token, small, "SELECT count(*) FROM input").await
    };
    let revoke = async {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        a.sql(&format!("REVOKE UCAN '{id}'")).await;
    };
    let (slow, waiting, ()) = tokio::join!(slow, waiting, revoke);
    assert!(slow.unwrap_err().to_string().contains("longer than"));
    let e = waiting.unwrap_err();
    assert!(e.to_string().contains("permission denied"), "{e}");

    // Two tasks an hour: the third is refused.
    let c = Node::new(75);
    seed(&c).await;
    a.owner.set_compute_quota(&c.did(), 1, 2).unwrap();
    let (_, token) = a.issue(&grant(&c, "time = '5s'")).await;
    for _ in 0..2 {
        run(&c.owner, &a, &token, small, "SELECT count(*) FROM input")
            .await
            .unwrap();
    }
    let e = run(&c.owner, &a, &token, small, "SELECT count(*) FROM input")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("tasks per hour"), "{e}");
}
