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

//! Address records (3.12): where a node accepts peer-to-peer connections,
//! signed with its DID key (a libp2p peer record), so an invite can carry
//! them and the receiver knows they come from the node that invited it.
//! Records only travel in invites and between contacts; nothing publishes
//! them.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use libp2p::core::{PeerRecord, SignedEnvelope};
use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use oikome_catalog::peer::MAX_PEER_ADDRS;
use oikome_identity::Did;

/// A record is a few addresses and a signature; anything longer is not one.
const MAX_RECORD_BYTES: usize = 4096;

/// The libp2p peer id of a DID (both name the same Ed25519 key).
pub fn peer_id_of(did: &Did) -> Option<PeerId> {
    let key =
        libp2p::identity::ed25519::PublicKey::try_from_bytes(&did.public_key().to_bytes()).ok()?;
    Some(libp2p::identity::PublicKey::from(key).to_peer_id())
}

/// `addrs`, signed by `key`, as text for an invite.
pub fn sign(key: &Keypair, addrs: Vec<Multiaddr>) -> Option<String> {
    let addrs = addrs.into_iter().take(MAX_PEER_ADDRS).collect();
    let record = PeerRecord::new(key, addrs).ok()?;
    let bytes = record.into_signed_envelope().into_protobuf_encoding();
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

/// The addresses in a record `did` signed, and when it signed it (ms).
pub fn read(did: &Did, text: &str) -> Result<(Vec<Multiaddr>, u64), String> {
    let invalid = || "that address record isn't valid".to_string();
    if text.len() > MAX_RECORD_BYTES * 2 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
    let envelope = SignedEnvelope::from_protobuf_encoding(&bytes).map_err(|_| invalid())?;
    let record = PeerRecord::from_signed_envelope(envelope).map_err(|_| invalid())?;
    let expected = peer_id_of(did).ok_or_else(invalid)?;
    if record.peer_id() != expected {
        return Err("that address record was signed by another node".to_string());
    }
    Ok((
        record.addresses().to_vec(),
        record.seq().saturating_mul(1000),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libp2p_keypair;
    use oikome_identity::NodeIdentity;

    #[test]
    fn records_are_read_only_for_the_node_that_signed_them() {
        let a = NodeIdentity::from_seed(&[1; 32]);
        let b = NodeIdentity::from_seed(&[2; 32]);
        let key = libp2p_keypair(&a).unwrap();
        assert_eq!(peer_id_of(a.did()), Some(key.public().to_peer_id()));
        let addrs: Vec<Multiaddr> = vec![
            "/ip4/192.168.1.5/tcp/4001".parse().unwrap(),
            "/ip4/203.0.113.9/tcp/4001".parse().unwrap(),
        ];
        let text = sign(&key, addrs.clone()).unwrap();
        let (read_back, at) = read(a.did(), &text).unwrap();
        assert_eq!(read_back, addrs);
        assert!(at > 1_700_000_000_000);

        // Someone else's record, a changed one, or not a record at all.
        let err = read(b.did(), &text).unwrap_err();
        assert!(err.contains("another node"), "{err}");
        let mut bytes = URL_SAFE_NO_PAD.decode(&text).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(read(a.did(), &URL_SAFE_NO_PAD.encode(&bytes)).is_err());
        assert!(read(a.did(), "not a record").is_err());
    }
}
