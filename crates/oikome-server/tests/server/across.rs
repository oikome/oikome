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

//! Reaching a node on another network (3.12), on one computer: every node
//! treats this computer's addresses as the internet, so its Flight endpoint
//! is never recorded and Flight goes over a peer-to-peer stream; only
//! contacts get in; and two nodes that can't be reached meet at a contact
//! that can, then connect directly. The routers themselves are simulated
//! in the Docker check (tests/e2e/docker_nat.sh).

use crate::common;

use std::time::Duration;

use common::{ints, Node};
use oikome_discovery::{Discovery, DiscoveryConfig, EndpointResponse};
use oikome_network::{Reach, ReachWanted};
use oikome_server::{serve_with_peers, OikomeFlightService};
use tokio::net::TcpListener;

/// The peer-to-peer layer of a serving node, "outside" every other node,
/// answering Flight on peer streams too. `reached` says whether it is on
/// the internet (`Some(true)`) or behind a router that won't open a port.
async fn outside(node: &Node, reached: Option<bool>) -> Discovery {
    let (scheme, rest) = node.url().split_once("://").unwrap();
    let port = rest.rsplit_once(':').unwrap().1.parse().unwrap();
    let mut config = DiscoveryConfig::new(
        "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
        EndpointResponse {
            scheme: scheme.to_string(),
            port,
        },
    );
    config.mdns = false;
    config.port_mapping = false;
    config.local_is_outside = true;
    config.reached_directly = reached;
    let discovery = Discovery::start(&node.identity, node.catalog.clone(), config)
        .await
        .unwrap();
    // Flight on the streams peers open; the TCP endpoint is unused.
    let streams = discovery.take_flight_streams();
    let service = OikomeFlightService::new(node.catalog.clone(), node.engines.clone())
        .unwrap()
        .with_access_log(node.access_log.clone());
    let tls = node.identity.tls_identity().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    tokio::spawn(async move {
        let _ = serve_with_peers(
            listener,
            streams,
            service,
            Some(&tls),
            std::future::pending(),
        )
        .await;
    });
    node.engines.set_dialer(discovery.dialer());
    discovery
}

/// `b` keeps where `a` is reached, from the record `a` signed (as an
/// invite carries it).
async fn learn(b: &Discovery, a: &Discovery, a_node: &Node) {
    let record = wait_for(|| a.network().address_record()).await;
    b.network()
        .learn_record(a_node.identity.did(), &record)
        .unwrap();
}

async fn wait_for<T>(mut what: impl FnMut() -> Option<T>) -> T {
    for _ in 0..400 {
        if let Some(found) = what() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out");
}

fn befriend(a: &Node, b: &Node, name: &str) {
    a.catalog.add_contact(&b.did(), name).unwrap();
}

async fn seed(a: &Node) {
    for sql in [
        "CREATE SPACE blog",
        "CREATE TABLE blog.posts (id BIGINT NOT NULL, title VARCHAR)",
        "INSERT INTO blog.posts VALUES (1, 'Hello'), (2, 'World')",
    ] {
        a.sql(sql).await;
    }
}

/// `owner` shares blog with `reader`, who adds it without an address.
async fn share(owner: &Node, reader: &Node) -> String {
    owner
        .sql(&format!(
            "GRANT SELECT ON blog.posts TO \"{}\"",
            reader.did()
        ))
        .await;
    let (id, token) = owner
        .issue(&format!("ISSUE UCAN ON SPACE blog TO '{}'", reader.did()))
        .await;
    reader
        .sql(&format!(
            "CREATE SPACE pub WITH TOKEN = '{token}', REMOTE = 'blog'"
        ))
        .await;
    id
}

#[tokio::test]
async fn flight_reaches_a_contact_on_another_network_over_its_peer_connection() {
    let a = Node::new(91).serve().await;
    let b = Node::new(92).serve().await;
    seed(&a).await;
    befriend(&a, &b, "Bob");
    befriend(&b, &a, "Home");
    let da = outside(&a, Some(true)).await;
    let db = outside(&b, Some(false)).await;
    learn(&db, &da, &a).await;

    let id = share(&a, &b).await;
    assert_eq!(
        ints(&b.sql("SELECT count(*) FROM pub.posts").await, 0),
        vec![2]
    );
    // Never through an endpoint on a shared network: there is none.
    let peer = b.catalog.get_peer(&a.did()).unwrap();
    assert!(peer.endpoint.is_empty(), "{peer:?}");
    assert!(!peer.addrs.is_empty());

    // Revoking still cuts it off.
    a.sql(&format!("REVOKE UCAN '{id}'")).await;
    let err = b.sql_err("SELECT count(*) FROM pub.posts").await;
    assert!(
        err.to_lowercase().contains("revoked") || err.to_lowercase().contains("permission"),
        "{err}"
    );
}

#[tokio::test]
async fn only_contacts_get_in_from_outside() {
    let a = Node::new(93).serve().await;
    let c = Node::new(94).serve().await;
    seed(&a).await;
    // C holds a valid grant, but A doesn't know C as a contact.
    befriend(&c, &a, "Home");
    let da = outside(&a, Some(true)).await;
    let turned_away = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    da.on_turned_away(std::sync::Arc::new({
        let turned_away = turned_away.clone();
        move |_, _| {
            turned_away.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }));
    let dc = outside(&c, Some(false)).await;
    learn(&dc, &da, &a).await;
    share(&a, &c).await;
    let err = c.sql_err("SELECT count(*) FROM pub.posts").await;
    assert!(err.contains("cannot reach"), "{err}");
    // Activity hears of it.
    assert!(turned_away.load(std::sync::atomic::Ordering::Relaxed) > 0);

    // Added as a contact, C gets in.
    befriend(&a, &c, "Cleo");
    assert_eq!(
        ints(&c.sql("SELECT count(*) FROM pub.posts").await, 0),
        vec![2]
    );
}

/// Two nodes behind routers that won't open a port meet at a contact
/// both reach, then connect directly; the contact passes no data.
#[tokio::test]
async fn nodes_that_cant_be_reached_meet_at_a_contact_and_connect_directly() {
    let a = Node::new(95).serve().await;
    let b = Node::new(96).serve().await;
    let c = Node::new(97).serve().await;
    seed(&a).await;
    for (x, y, name) in [
        (&a, &b, "Bob"),
        (&b, &a, "Home"),
        (&a, &c, "Cleo"),
        (&c, &a, "Home"),
        (&b, &c, "Cleo"),
        (&c, &b, "Bob"),
    ] {
        befriend(x, y, name);
    }
    let da = outside(&a, Some(false)).await;
    let db = outside(&b, Some(false)).await;
    let dc = outside(&c, Some(true)).await;
    // Each knows where Cleo is; neither knows where the other is.
    learn(&da, &dc, &c).await;
    learn(&db, &dc, &c).await;
    dc.network().set_reach(ReachWanted {
        open: false,
        help_contacts: true,
    });
    wait_for(|| dc.network().status().helping.then_some(())).await;

    // A shares, so it wants to be reached: its router won't, Cleo keeps
    // a place for it.
    da.network().set_reach(ReachWanted {
        open: true,
        help_contacts: true,
    });
    let through = wait_for(|| match da.network().status().reach {
        Reach::ThroughContact(did) => Some(did),
        _ => None,
    })
    .await;
    assert_eq!(through.to_string(), c.did());

    share(&a, &b).await;
    assert!(b
        .catalog
        .get_peer(&a.did())
        .is_none_or(|p| p.addrs.is_empty()));
    assert_eq!(
        ints(&b.sql("SELECT count(*) FROM pub.posts").await, 0),
        vec![2]
    );
    // Cleo served nothing.
    let served = c
        .access_log
        .list(&oikome_node::AccessQuery {
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert!(served.is_empty(), "{served:?}");
}

#[tokio::test]
async fn a_contact_that_doesnt_help_introduces_no_one() {
    let a = Node::new(98).serve().await;
    let b = Node::new(99).serve().await;
    let c = Node::new(100).serve().await;
    seed(&a).await;
    for (x, y, name) in [
        (&a, &b, "Bob"),
        (&b, &a, "Home"),
        (&a, &c, "Cleo"),
        (&c, &a, "Home"),
        (&b, &c, "Cleo"),
        (&c, &b, "Bob"),
    ] {
        befriend(x, y, name);
    }
    let da = outside(&a, Some(false)).await;
    let db = outside(&b, Some(false)).await;
    let dc = outside(&c, Some(true)).await;
    learn(&da, &dc, &c).await;
    learn(&db, &dc, &c).await;
    // Cleo turned helping off.
    dc.network().set_reach(ReachWanted {
        open: false,
        help_contacts: false,
    });
    da.network().set_reach(ReachWanted {
        open: true,
        help_contacts: true,
    });
    wait_for(|| (da.network().status().reach == Reach::RouterRefused).then_some(())).await;
    // Still no place a while later.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(da.network().status().reach, Reach::RouterRefused);

    share(&a, &b).await;
    let err = b.sql_err("SELECT count(*) FROM pub.posts").await;
    assert!(err.contains("cannot reach"), "{err}");
}
