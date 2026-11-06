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

//! The Peers table (architecture v3 §3.4): nodes found by discovery, with
//! the Flight endpoint each announced on a network this node shares with
//! it, and where its peer-to-peer connections are accepted (its libp2p
//! addresses, from an invite it signed or from the node itself, 3.12).
//!
//! Discovery runs in the serving process; recording peers in the catalog
//! lets every process of the node (`oikome sql` included) resolve a remote
//! Space's owner by DID. An entry only says where a DID was last seen: the
//! connection itself is authenticated against the DID (TLS certificate and
//! signed requests), so a wrong entry can fail a connection but never
//! impersonate a peer.

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};

use crate::catalog::OikomeCatalog;
use crate::registry::now_ms;
use crate::ucan::validate_peer_url;

/// Re-recording an unchanged endpoint more often than this is skipped, so
/// discovery does not rewrite the registry on every announcement.
const PEER_REFRESH_MS: u64 = 60_000;
/// Most addresses kept for one peer.
pub const MAX_PEER_ADDRS: usize = 16;
/// Longest address kept.
const MAX_ADDR_CHARS: usize = 256;

/// A node found by discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// The node's DID (`did:key:z…`).
    pub did: String,
    /// Its Flight endpoint on a network this node shares with it,
    /// `https://host:port` (`http://` for a plaintext node); empty when it
    /// was only ever reached from another network.
    #[serde(default)]
    pub endpoint: String,
    pub last_seen_ms: u64,
    /// Where it accepts peer-to-peer connections (libp2p multiaddrs), as it
    /// last said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<String>,
    /// When it said so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addrs_at_ms: Option<u64>,
}

impl OikomeCatalog {
    /// Record that `did` was seen serving Flight at `endpoint`.
    pub fn record_peer(&self, did: &str, endpoint: &str) -> Result<(), CatalogError> {
        if !did.starts_with("did:") {
            return Err(CatalogError::InvalidProperty(format!(
                "peer '{did}' is not a DID"
            )));
        }
        let endpoint = validate_peer_url(endpoint)?;
        let now = now_ms();
        let fresh = self.with_registry(|reg| {
            reg.peers.get(did).is_some_and(|p| {
                p.endpoint == endpoint && now.saturating_sub(p.last_seen_ms) < PEER_REFRESH_MS
            })
        });
        if fresh {
            return Ok(());
        }
        self.mutate_registry(|reg| {
            let peer = reg.peers.entry(did.to_string()).or_insert_with(|| Peer {
                did: did.to_string(),
                endpoint: String::new(),
                last_seen_ms: now,
                addrs: Vec::new(),
                addrs_at_ms: None,
            });
            peer.endpoint = endpoint;
            peer.last_seen_ms = now;
            Ok(())
        })
    }

    /// Record where `did` accepts peer-to-peer connections, as it said at
    /// `said_at_ms`; an older word than the one kept is ignored. The node
    /// itself is reached by its DID, so a wrong address can fail a
    /// connection but never impersonate it.
    pub fn record_peer_addrs(
        &self,
        did: &str,
        addrs: &[String],
        said_at_ms: u64,
    ) -> Result<(), CatalogError> {
        if !did.starts_with("did:") {
            return Err(CatalogError::InvalidProperty(format!(
                "peer '{did}' is not a DID"
            )));
        }
        let mut kept: Vec<String> = Vec::new();
        for addr in addrs {
            let ok = addr.starts_with('/')
                && addr.len() <= MAX_ADDR_CHARS
                && !addr.chars().any(|c| c.is_control() || c.is_whitespace());
            if ok && !kept.contains(addr) && kept.len() < MAX_PEER_ADDRS {
                kept.push(addr.clone());
            }
        }
        let same = self.with_registry(|reg| {
            reg.peers.get(did).is_some_and(|p| {
                p.addrs_at_ms.is_some_and(|at| at >= said_at_ms)
                    || (p.addrs == kept
                        && p.addrs_at_ms
                            .is_some_and(|at| said_at_ms.saturating_sub(at) < PEER_REFRESH_MS))
            })
        });
        if same {
            return Ok(());
        }
        let now = now_ms();
        self.mutate_registry(|reg| {
            let peer = reg.peers.entry(did.to_string()).or_insert_with(|| Peer {
                did: did.to_string(),
                endpoint: String::new(),
                last_seen_ms: now,
                addrs: Vec::new(),
                addrs_at_ms: None,
            });
            peer.addrs = kept;
            peer.addrs_at_ms = Some(said_at_ms);
            Ok(())
        })
    }

    /// Where `did` was last seen, if discovery found it.
    pub fn get_peer(&self, did: &str) -> Option<Peer> {
        self.with_registry(|reg| reg.peers.get(did).cloned())
    }

    /// Every discovered peer, ordered by DID.
    pub fn list_peers(&self) -> Vec<Peer> {
        self.with_registry(|reg| reg.peers.values().cloned().collect())
    }
}
