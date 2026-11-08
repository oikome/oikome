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

//! Only contacts get in from outside (3.12). A connection from this node's
//! own network is let in as before (nodes find each other there); one from
//! the internet, or through another node, only from a contact's key. And a
//! node helps only its own contacts meet, a few kilobytes at a time.

use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Instant;

use libp2p::core::transport::PortUse;
use libp2p::core::Endpoint;
use libp2p::swarm::{
    dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
    THandlerOutEvent, ToSwarm,
};
use libp2p::{relay, Multiaddr, PeerId};
use oikome_catalog::OikomeCatalog;
use oikome_identity::Did;

use crate::addr::Classifier;
use crate::record::peer_id_of;

/// Contacts other processes of the node added (`oikome sql`) are read
/// from disk this often.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Told of each connection turned away from outside (for Activity).
pub type TurnedAway = Arc<dyn Fn(&PeerId, &Multiaddr) + Send + Sync>;

/// What the gate and the meeting point check against, shared with the
/// discovery task that keeps it current.
pub struct Shared {
    contacts: RwLock<HashSet<PeerId>>,
    turned_away: RwLock<Option<TurnedAway>>,
    /// Contacts may meet here now.
    helping: AtomicBool,
    classifier: Classifier,
    catalog: Option<Arc<OikomeCatalog>>,
    refreshed: std::sync::Mutex<Option<Instant>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("helping", &self.helping())
            .field("classifier", &self.classifier)
            .finish_non_exhaustive()
    }
}

impl Shared {
    /// Contacts are read from `catalog`.
    pub fn new(classifier: Classifier, catalog: Arc<OikomeCatalog>) -> Self {
        Self {
            classifier,
            catalog: Some(catalog),
            ..Self::empty(classifier)
        }
    }

    fn empty(classifier: Classifier) -> Self {
        Self {
            contacts: RwLock::default(),
            turned_away: RwLock::new(None),
            helping: AtomicBool::new(false),
            classifier,
            catalog: None,
            refreshed: std::sync::Mutex::new(None),
        }
    }

    pub fn classifier(&self) -> Classifier {
        self.classifier
    }

    /// Tell `listener` of each connection turned away from outside.
    pub fn on_turned_away(&self, listener: TurnedAway) {
        *self.turned_away.write().unwrap_or_else(|p| p.into_inner()) = Some(listener);
    }

    fn turn_away(&self, peer: &PeerId, remote: &Multiaddr) {
        let listener = self
            .turned_away
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(listener) = listener {
            listener(peer, remote);
        }
    }

    /// Read the contacts again (from disk, once in a while).
    pub fn reload_contacts(&self) {
        let Some(catalog) = &self.catalog else {
            return;
        };
        {
            let mut refreshed = self.refreshed.lock().unwrap_or_else(|p| p.into_inner());
            if refreshed.is_none_or(|at| at.elapsed() >= REFRESH_EVERY) {
                if let Err(e) = catalog.refresh() {
                    tracing::debug!(error = %e, "could not read the catalog again");
                }
                *refreshed = Some(Instant::now());
            }
        }
        let contacts = catalog
            .list_contacts()
            .iter()
            .filter_map(|c| peer_id_of(&Did::parse(&c.did).ok()?))
            .collect();
        self.set_contacts(contacts);
    }

    pub fn set_contacts(&self, contacts: HashSet<PeerId>) {
        *self.contacts.write().unwrap_or_else(|p| p.into_inner()) = contacts;
    }

    /// Whether `peer` is a contact; one added a moment ago counts.
    pub fn is_contact(&self, peer: &PeerId) -> bool {
        let known = self
            .contacts
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(peer);
        if known || self.catalog.is_none() {
            return known;
        }
        self.reload_contacts();
        self.contacts
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(peer)
    }

    pub fn contacts(&self) -> HashSet<PeerId> {
        self.contacts
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_helping(&self, on: bool) {
        self.helping.store(on, Ordering::Relaxed);
    }

    pub fn helping(&self) -> bool {
        self.helping.load(Ordering::Relaxed)
    }

    /// Whether `peer` may connect from `remote`.
    pub fn admits(&self, peer: &PeerId, remote: &Multiaddr) -> bool {
        !self.classifier.is_outside(remote) || self.is_contact(peer)
    }
}

/// Why a connection was turned away.
#[derive(Debug)]
pub struct NotAContact;

impl std::fmt::Display for NotAContact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("only contacts connect from outside this network")
    }
}

impl std::error::Error for NotAContact {}

/// Turns away connections from outside that aren't from a contact.
pub struct Gate {
    shared: Arc<Shared>,
}

impl Gate {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

impl NetworkBehaviour for Gate {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = Infallible;

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        remote: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        if self.shared.admits(&peer, remote) {
            Ok(dummy::ConnectionHandler)
        } else {
            tracing::info!(%peer, %remote, "turned away a connection from outside: not a contact");
            self.shared.turn_away(&peer, remote);
            Err(ConnectionDenied::new(NotAContact))
        }
    }

    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, _: FromSwarm) {}

    fn on_connection_handler_event(
        &mut self,
        _: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        libp2p::core::util::unreachable(event)
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

/// Lets only contacts reserve a place here or open a circuit through it,
/// and only while this node helps.
pub struct ContactsOnly(pub Arc<Shared>);

impl relay::RateLimiter for ContactsOnly {
    fn try_next(&mut self, peer: PeerId, _: &Multiaddr, _: Instant) -> bool {
        self.0.helping() && self.0.is_contact(&peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;
    use relay::RateLimiter;

    fn internet_addr() -> Multiaddr {
        "/ip4/203.0.113.9/tcp/4001".parse().unwrap()
    }

    #[test]
    fn only_contacts_get_in_from_outside() {
        let contact = Keypair::generate_ed25519().public().to_peer_id();
        let stranger = Keypair::generate_ed25519().public().to_peer_id();
        let shared = Shared::empty(Classifier::default());
        shared.set_contacts([contact].into());
        let lan: Multiaddr = "/ip4/192.168.1.9/tcp/4001".parse().unwrap();
        let internet: Multiaddr = "/ip4/203.0.113.9/tcp/4001".parse().unwrap();
        let relayed: Multiaddr = format!("/ip4/192.168.1.9/tcp/4001/p2p/{contact}/p2p-circuit")
            .parse()
            .unwrap();
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = told.clone();
        shared.on_turned_away(Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let mut gate = Gate::new(Arc::new(Shared::empty(Classifier::default())));
        gate.shared.on_turned_away(Arc::new({
            let told = told.clone();
            move |_, _| {
                told.fetch_add(1, Ordering::Relaxed);
            }
        }));
        let local: Multiaddr = "/ip4/192.168.1.1/tcp/4001".parse().unwrap();
        assert!(gate
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(1),
                stranger,
                &local,
                &internet_addr()
            )
            .is_err());
        assert_eq!(told.load(Ordering::Relaxed), 1);
        assert!(shared.admits(&stranger, &lan));
        assert!(shared.admits(&contact, &internet));
        assert!(!shared.admits(&stranger, &internet));
        assert!(!shared.admits(&stranger, &relayed));
        assert!(shared.admits(&contact, &relayed));

        // Tests treat this computer as outside.
        let test = Shared::empty(Classifier {
            local_is_outside: true,
        });
        let here: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert!(!test.admits(&stranger, &here));
    }

    #[test]
    fn only_contacts_meet_here_and_only_while_helping() {
        let contact = Keypair::generate_ed25519().public().to_peer_id();
        let stranger = Keypair::generate_ed25519().public().to_peer_id();
        let shared = Arc::new(Shared::empty(Classifier::default()));
        shared.set_contacts([contact].into());
        let mut limit = ContactsOnly(shared.clone());
        let addr: Multiaddr = "/ip4/203.0.113.9/tcp/4001".parse().unwrap();
        let now = Instant::now();
        assert!(!limit.try_next(contact, &addr, now));
        shared.set_helping(true);
        assert!(limit.try_next(contact, &addr, now));
        assert!(!limit.try_next(stranger, &addr, now));
    }
}
