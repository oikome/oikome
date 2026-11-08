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

//! The node's peer-to-peer layer (architecture v3 §9, roadmap 3.12).
//!
//! The node's libp2p key is its DID key, so a libp2p `PeerId` and a DID
//! name the same public key, and every connection (Noise) is authenticated
//! against it.
//!
//! **On the local network** mDNS finds nodes; once connected a node asks
//! the other for its Flight endpoint over `/oikome/endpoint/1` and records
//! `DID → scheme://ip:port` in the catalog's Peers table, where every
//! process of the node resolves remote Spaces by owner DID. Flight then
//! goes straight there.
//!
//! **From another network** nothing is published anywhere, and no server
//! of the project's relays anything:
//!
//! * where a node is reached travels in invites (a record signed with its
//!   key, [`record`]) and between contacts (identify, on authenticated
//!   connections), into the Peers table;
//! * while the owner allowed something that needs it (a share, a lent
//!   computer), the node asks its router to open its one peer-to-peer port
//!   ([`portmap`]);
//! * when that doesn't work, a contact that both sides reach introduces
//!   them: each reserves a place at it, the other connects through it, and
//!   the two punch through their routers to a direct connection (DCUtR).
//!   The contact only passes the few kilobytes of the introduction
//!   ([`gate::ContactsOnly`], 16 KiB and 30 s a circuit), and only for its
//!   own contacts, while it is reached directly itself;
//! * from outside, only contacts get in at all ([`gate::Gate`]).
//!
//! Arrow Flight then runs over a stream of the direct connection
//! (`/oikome/flight/1`), with TLS pinned to the DID on top as on the local
//! network ([`Discovery::dialer`], [`Discovery::take_flight_streams`]).
//!
//! mDNS dials and [`Discovery::dial`] go out from a new port: by default
//! libp2p dials from the listening port, so two nodes that find each other
//! at the same moment would open one TCP connection from both ends, each
//! side would start the Noise handshake as the initiator, and neither would
//! ever record the other. Dials to contacts reuse the listening port, so
//! the router keeps one mapping for it, which hole punching relies on.

pub mod addr;
pub mod gate;
pub mod portmap;
pub mod record;

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::StreamExt;
use libp2p::core::transport::ListenerId;
use libp2p::core::ConnectedPoint;
use libp2p::identity::Keypair;
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{self, json, ProtocolSupport};
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{ConnectionId, NetworkBehaviour, SwarmEvent};
use libp2p::{
    dcutr, identify, mdns, noise, relay, tcp, yamux, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use oikome_catalog::OikomeCatalog;
use oikome_identity::{Did, NodeIdentity};
use oikome_network::route::DIAL_TIMEOUT;
use oikome_network::{NetworkStatus, NodeNetwork, PeerDialer, PeerIo, Reach, ReachWanted};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use zeroize::Zeroizing;

use crate::addr::Classifier;
use crate::gate::{ContactsOnly, Gate, Shared};
use crate::portmap::{Mapped, PortMapper};

pub use libp2p::Multiaddr;

/// identify protocol version announced by Oikome nodes.
pub const IDENTIFY_PROTOCOL: &str = "/oikome/id/1";
/// Request-response protocol asking a node for its Flight endpoint.
pub const ENDPOINT_PROTOCOL: &str = "/oikome/endpoint/1";
/// Arrow Flight over a stream of a peer-to-peer connection.
pub const FLIGHT_PROTOCOL: &str = "/oikome/flight/1";
/// Idle connections are kept this long, so identify and the endpoint
/// exchange can finish.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long [`Discovery::start`] waits for the listener to come up.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(10);
/// How often mDNS asks the local network for peers. libp2p's default is five
/// minutes, so a node whose first query went unanswered (a peer still
/// starting, a dropped multicast packet) stayed alone that long.
const MDNS_QUERY_INTERVAL: Duration = Duration::from_secs(10);
/// How often the node looks at its contacts and reach again.
const TICK: Duration = Duration::from_secs(5);
/// How often the router is asked again (mappings are renewed at half their
/// lease) and contacts' places are asked for again.
const REACH_EVERY: Duration = Duration::from_secs(60);
/// What a contact passes for two others who meet at it: enough for them to
/// learn each other's addresses and agree when to connect, and no more.
const CIRCUIT_BYTES: u64 = 16 * 1024;
const CIRCUIT_DURATION: Duration = Duration::from_secs(30);
/// Contacts this node keeps a place at, when its router won't open a port.
const MAX_PLACES: usize = 3;
/// The node's peer-to-peer port, kept so contacts find it at the same one
/// after a restart.
const PORT_FILE: &str = "p2p.json";

/// `/oikome/endpoint/1` request: "where do you serve Flight?"
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointRequest {}

/// `/oikome/endpoint/1` response. The host is the address the connection
/// came from, so only the scheme and port are announced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointResponse {
    /// `https`, or `http` for a plaintext node.
    pub scheme: String,
    pub port: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("libp2p: {0}")]
    Libp2p(String),
    #[error("discovery has stopped")]
    Stopped,
}

fn libp2p_err(e: impl std::fmt::Display) -> DiscoveryError {
    DiscoveryError::Libp2p(e.to_string())
}

/// What a node runs its peer-to-peer layer with.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// libp2p listen address; port 0 takes the port this node used last
    /// time, or a free one.
    pub listen: Multiaddr,
    /// The Flight endpoint announced to peers on the same network.
    pub flight: EndpointResponse,
    /// Find peers on the local network with mDNS.
    pub mdns: bool,
    /// Ask the router to open the port when the owner allowed something
    /// that needs it. Off, the router is never asked.
    pub port_mapping: bool,
    /// Tests: treat this computer's addresses as the internet, so nodes on
    /// one machine connect "from outside".
    pub local_is_outside: bool,
    /// Tests: whether this node counts as reached directly at its listening
    /// addresses (a node on the internet) rather than working it out.
    pub reached_directly: Option<bool>,
}

impl DiscoveryConfig {
    /// Listening on `listen`, announcing `flight`, finding peers with mDNS
    /// and asking the router when needed.
    pub fn new(listen: Multiaddr, flight: EndpointResponse) -> Self {
        Self {
            listen,
            flight,
            mdns: true,
            port_mapping: true,
            local_is_outside: false,
            reached_directly: None,
        }
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    // First: a connection it turns away never reaches the others.
    gate: Gate,
    mdns: Toggle<mdns::tokio::Behaviour>,
    identify: identify::Behaviour,
    endpoint: json::Behaviour<EndpointRequest, EndpointResponse>,
    relay: relay::Behaviour,
    relay_client: relay::client::Behaviour,
    dcutr: dcutr::Behaviour,
    stream: libp2p_stream::Behaviour,
}

enum Command {
    Dial(Multiaddr, oneshot::Sender<Result<(), DiscoveryError>>),
    /// Get a direct connection to a peer.
    Connect(PeerId, oneshot::Sender<Result<(), String>>),
    /// What the router said.
    Mapped(Mapped),
}

/// What the owner's API reads and the task keeps current.
#[derive(Default)]
struct NetState {
    status: RwLock<NetworkStatus>,
    record: RwLock<Option<String>>,
}

/// A running peer-to-peer layer. Dropping it stops it; [`shutdown`]
/// first closes the router port.
///
/// [`shutdown`]: Discovery::shutdown
pub struct Discovery {
    peer_id: PeerId,
    listen_addrs: Vec<Multiaddr>,
    commands: mpsc::Sender<Command>,
    reach: watch::Sender<ReachWanted>,
    control: libp2p_stream::Control,
    flight_streams: std::sync::Mutex<Option<libp2p_stream::IncomingStreams>>,
    net: Arc<NetState>,
    catalog: Arc<OikomeCatalog>,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
    mapper: Option<(watch::Sender<bool>, JoinHandle<()>)>,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discovery")
            .field("peer_id", &self.peer_id)
            .field("listen_addrs", &self.listen_addrs)
            .finish_non_exhaustive()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.task.abort();
        if let Some((_, mapper)) = &self.mapper {
            mapper.abort();
        }
    }
}

/// The libp2p identity of a node: its DID key.
pub fn libp2p_keypair(identity: &NodeIdentity) -> Result<Keypair, DiscoveryError> {
    identity.with_seed(|seed| {
        // Zeroed by libp2p on success and by `Zeroizing` in any case.
        let mut copy = Zeroizing::new(*seed);
        Keypair::ed25519_from_bytes(&mut *copy).map_err(libp2p_err)
    })
}

/// The DID of a libp2p public key, if it is an Ed25519 key.
fn did_of(key: libp2p::identity::PublicKey) -> Option<Did> {
    let key = key.try_into_ed25519().ok()?;
    Did::from_public_key_bytes(&key.to_bytes()).ok()
}

/// `scheme://ip:port`, or `None` for an unknown scheme.
fn endpoint_url(scheme: &str, ip: IpAddr, port: u16) -> Option<String> {
    if !matches!(scheme, "https" | "http") {
        return None;
    }
    Some(match ip {
        IpAddr::V4(ip) => format!("{scheme}://{ip}:{port}"),
        IpAddr::V6(ip) => format!("{scheme}://[{ip}]:{port}"),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `listen` on the port kept from last time, when it asks for any port.
fn with_kept_port(listen: &Multiaddr, kept: Option<u16>) -> Option<Multiaddr> {
    let kept = kept?;
    if addr::tcp_port_of(listen) != Some(0) {
        return None;
    }
    Some(
        listen
            .iter()
            .map(|p| match p {
                Protocol::Tcp(0) => Protocol::Tcp(kept),
                other => other,
            })
            .collect(),
    )
}

#[derive(Serialize, Deserialize)]
struct KeptPort {
    port: u16,
}

fn kept_port(catalog: &OikomeCatalog) -> Option<u16> {
    let bytes = std::fs::read(catalog.oikome_dir().join(PORT_FILE)).ok()?;
    serde_json::from_slice::<KeptPort>(&bytes)
        .ok()
        .map(|k| k.port)
        .filter(|p| *p != 0)
}

fn keep_port(catalog: &OikomeCatalog, port: u16) {
    let path = catalog.oikome_dir().join(PORT_FILE);
    if let Ok(bytes) = serde_json::to_vec(&KeptPort { port }) {
        if let Err(e) = std::fs::write(&path, bytes) {
            tracing::debug!(error = %e, "could not keep the peer-to-peer port");
        }
    }
}

impl Discovery {
    /// Start the peer-to-peer layer for the node `identity`, recording
    /// peers in `catalog`. Returns once the libp2p listener is up.
    pub async fn start(
        identity: &NodeIdentity,
        catalog: Arc<OikomeCatalog>,
        config: DiscoveryConfig,
    ) -> Result<Self, DiscoveryError> {
        let keypair = libp2p_keypair(identity)?;
        let use_mdns = config.mdns;
        let classifier = Classifier {
            local_is_outside: config.local_is_outside,
        };
        let shared = Arc::new(Shared::new(classifier, catalog.clone()));
        shared.reload_contacts();
        let mut swarm = SwarmBuilder::with_existing_identity(keypair.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(libp2p_err)?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(libp2p_err)?
            .with_behaviour(|key, relay_client| {
                let peer_id = key.public().to_peer_id();
                let mdns = if use_mdns {
                    Some(mdns::tokio::Behaviour::new(
                        mdns::Config {
                            query_interval: MDNS_QUERY_INTERVAL,
                            ..Default::default()
                        },
                        peer_id,
                    )?)
                } else {
                    None
                };
                Ok(Behaviour {
                    gate: Gate::new(shared.clone()),
                    mdns: Toggle::from(mdns),
                    identify: identify::Behaviour::new(
                        identify::Config::new(IDENTIFY_PROTOCOL.to_string(), key.public())
                            .with_push_listen_addr_updates(true),
                    ),
                    endpoint: json::Behaviour::new(
                        [(
                            StreamProtocol::new(ENDPOINT_PROTOCOL),
                            ProtocolSupport::Full,
                        )],
                        request_response::Config::default(),
                    ),
                    relay: relay::Behaviour::new(
                        peer_id,
                        relay::Config {
                            max_reservations: 32,
                            max_reservations_per_peer: 2,
                            reservation_rate_limiters: vec![Box::new(ContactsOnly(shared.clone()))],
                            max_circuits: 16,
                            max_circuits_per_peer: 4,
                            max_circuit_duration: CIRCUIT_DURATION,
                            max_circuit_bytes: CIRCUIT_BYTES,
                            circuit_src_rate_limiters: vec![Box::new(ContactsOnly(shared.clone()))],
                            ..Default::default()
                        },
                    ),
                    relay_client,
                    dcutr: dcutr::Behaviour::new(peer_id),
                    stream: libp2p_stream::Behaviour::new(),
                })
            })
            .map_err(libp2p_err)?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_TIMEOUT))
            .build();

        // The port from last time, so contacts find the node where they
        // last saw it; any free one if it is taken.
        let listener = match with_kept_port(&config.listen, kept_port(&catalog)) {
            Some(kept) => match swarm.listen_on(kept) {
                Ok(id) => id,
                Err(_) => swarm.listen_on(config.listen.clone()).map_err(libp2p_err)?,
            },
            None => swarm.listen_on(config.listen.clone()).map_err(libp2p_err)?,
        };
        let first = tokio::time::timeout(LISTEN_TIMEOUT, async {
            loop {
                match swarm.select_next_some().await {
                    SwarmEvent::NewListenAddr {
                        listener_id,
                        address,
                    } if listener_id == listener => return Ok(address),
                    SwarmEvent::ListenerClosed { reason, .. } => {
                        return Err(libp2p_err(format!("{reason:?}")))
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| DiscoveryError::Libp2p("listener did not start".to_string()))??;
        let port = addr::tcp_port_of(&first).unwrap_or(0);
        keep_port(&catalog, port);
        // IPv6 on the same port, where the computer has it.
        let any_v4 = config
            .listen
            .iter()
            .any(|p| matches!(p, Protocol::Ip4(ip) if ip.is_unspecified()));
        if any_v4 {
            let v6: Multiaddr = format!("/ip6/::/tcp/{port}").parse().map_err(libp2p_err)?;
            if let Err(e) = swarm.listen_on(v6) {
                tracing::debug!(error = %e, "no IPv6 listener");
            }
        }

        let mut control = swarm.behaviour().stream.new_control();
        let flight_streams = control
            .accept(StreamProtocol::new(FLIGHT_PROTOCOL))
            .map_err(libp2p_err)?;

        let peer_id = *swarm.local_peer_id();
        let (commands, receiver) = mpsc::channel(64);
        let (reach, reach_rx) = watch::channel(ReachWanted::default());
        let mapper = config.port_mapping.then(|| {
            let (want, wanted) = watch::channel(false);
            let task = tokio::spawn(run_mapper(port, wanted, commands.clone()));
            (want, task)
        });
        let net = Arc::new(NetState::default());
        let state = State {
            catalog: catalog.clone(),
            flight: config.flight,
            shared: shared.clone(),
            net: net.clone(),
            keypair,
            reached_directly: config.reached_directly,
            mapping: mapper.as_ref().map(|(want, _)| want.clone()),
            wanted: ReachWanted::default(),
            mapped: (!config.port_mapping)
                .then(|| Mapped::Refused("this node doesn't ask its router".to_string())),
            mapped_addr: None,
            public: HashSet::new(),
            dids: HashMap::new(),
            ips: HashMap::new(),
            direct: HashMap::new(),
            relayed: HashMap::new(),
            waiting: HashMap::new(),
            places: HashMap::new(),
            placed: HashSet::new(),
            retry_place: HashMap::new(),
            last_reach: Instant::now() - REACH_EVERY,
            told: None,
        };
        let task = tokio::spawn(run(swarm, receiver, reach_rx, state));
        tracing::info!(%peer_id, addr = %first, mdns = use_mdns, "discovery started");
        Ok(Self {
            peer_id,
            listen_addrs: vec![first],
            commands,
            reach,
            control,
            flight_streams: std::sync::Mutex::new(Some(flight_streams)),
            net,
            catalog,
            shared,
            task,
            mapper,
        })
    }

    /// Tell `listener` of each connection turned away from outside: not a
    /// contact (for Activity).
    pub fn on_turned_away(&self, listener: gate::TurnedAway) {
        self.shared.on_turned_away(listener);
    }

    /// This node's libp2p peer id (its DID key).
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Where this node accepts libp2p connections.
    pub fn listen_addrs(&self) -> &[Multiaddr] {
        &self.listen_addrs
    }

    /// Connect to a node at a known libp2p address; it is recorded like
    /// one found by mDNS.
    pub async fn dial(&self, addr: Multiaddr) -> Result<(), DiscoveryError> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Dial(addr, reply))
            .await
            .map_err(|_| DiscoveryError::Stopped)?;
        answer.await.map_err(|_| DiscoveryError::Stopped)?
    }

    /// Opens Flight streams to peers by DID over direct peer-to-peer
    /// connections (made, if need be, through a contact).
    pub fn dialer(&self) -> Arc<dyn PeerDialer> {
        Arc::new(StreamDialer {
            commands: self.commands.clone(),
            control: self.control.clone(),
        })
    }

    /// The network as the owner's API sees it.
    pub fn network(&self) -> Arc<dyn NodeNetwork> {
        Arc::new(Network {
            net: self.net.clone(),
            reach: self.reach.clone(),
            catalog: self.catalog.clone(),
        })
    }

    /// The Flight streams peers open to this node; for the Flight server,
    /// once.
    pub fn take_flight_streams(&self) -> Option<BoxStream<'static, PeerIo>> {
        let streams = self
            .flight_streams
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()?;
        Some(
            streams
                .map(|(peer, stream)| {
                    tracing::debug!(%peer, "a Flight stream from a peer");
                    Box::new(stream.compat()) as PeerIo
                })
                .boxed(),
        )
    }

    /// Close the router port, then stop.
    pub async fn shutdown(mut self) {
        if let Some((want, mapper)) = self.mapper.take() {
            let _ = want.send(false);
            drop(want);
            let _ = tokio::time::timeout(Duration::from_secs(3), mapper).await;
        }
    }
}

/// Asks the router while `wanted`, and closes the port when not.
async fn run_mapper(port: u16, mut wanted: watch::Receiver<bool>, commands: mpsc::Sender<Command>) {
    let mut mapper = PortMapper::new(port);
    let mut every = tokio::time::interval(REACH_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = wanted.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = every.tick() => {}
        }
        let want = *wanted.borrow_and_update();
        if want {
            let result = mapper.ensure().await;
            if commands.send(Command::Mapped(result)).await.is_err() {
                break;
            }
        } else {
            mapper.release().await;
        }
    }
    mapper.release().await;
}

/// [`PeerDialer`] over the swarm.
struct StreamDialer {
    commands: mpsc::Sender<Command>,
    control: libp2p_stream::Control,
}

impl PeerDialer for StreamDialer {
    fn open(&self, peer: &Did) -> BoxFuture<'static, io::Result<PeerIo>> {
        let commands = self.commands.clone();
        let mut control = self.control.clone();
        let target = record::peer_id_of(peer);
        let did = peer.clone();
        Box::pin(async move {
            let peer = target.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{did} has no node key"),
                )
            })?;
            let (reply, answer) = oneshot::channel();
            commands
                .send(Command::Connect(peer, reply))
                .await
                .map_err(|_| io::Error::other("the peer-to-peer layer has stopped"))?;
            tokio::time::timeout(DIAL_TIMEOUT, answer)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no connection in time"))?
                .map_err(|_| io::Error::other("the peer-to-peer layer has stopped"))?
                .map_err(|e| io::Error::new(io::ErrorKind::NotConnected, e))?;
            let stream = control
                .open_stream(peer, StreamProtocol::new(FLIGHT_PROTOCOL))
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::NotConnected, e.to_string()))?;
            Ok(Box::new(stream.compat()) as PeerIo)
        })
    }
}

/// [`NodeNetwork`] over the task's state.
struct Network {
    net: Arc<NetState>,
    reach: watch::Sender<ReachWanted>,
    catalog: Arc<OikomeCatalog>,
}

impl NodeNetwork for Network {
    fn status(&self) -> NetworkStatus {
        self.net
            .status
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn set_reach(&self, wanted: ReachWanted) {
        self.reach.send_if_modified(|now| {
            let changed = *now != wanted;
            *now = wanted;
            changed
        });
    }

    fn address_record(&self) -> Option<String> {
        self.net
            .record
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn learn_record(&self, did: &Did, text: &str) -> Result<(), String> {
        let (addrs, at) = record::read(did, text)?;
        let addrs: Vec<String> = addrs.iter().map(ToString::to_string).collect();
        self.catalog
            .record_peer_addrs(did.as_str(), &addrs, at)
            .map_err(|e| e.to_string())
    }
}

/// What the task knows.
struct State {
    catalog: Arc<OikomeCatalog>,
    flight: EndpointResponse,
    shared: Arc<Shared>,
    net: Arc<NetState>,
    keypair: Keypair,
    reached_directly: Option<bool>,
    /// Tells the mapper whether to keep the port open; `None` when it
    /// never asks.
    mapping: Option<watch::Sender<bool>>,
    wanted: ReachWanted,
    /// What the router last said.
    mapped: Option<Mapped>,
    /// The address the router opened, as given to the swarm.
    mapped_addr: Option<Multiaddr>,
    /// This node's own addresses on the internet, as given to the swarm.
    public: HashSet<Multiaddr>,
    /// DID of each peer, from its verified identify key.
    dids: HashMap<PeerId, Did>,
    /// Address each peer's connection came from, on the local network.
    ips: HashMap<PeerId, IpAddr>,
    direct: HashMap<PeerId, HashSet<ConnectionId>>,
    relayed: HashMap<PeerId, HashSet<ConnectionId>>,
    /// Callers waiting for a direct connection.
    waiting: HashMap<PeerId, Vec<oneshot::Sender<Result<(), String>>>>,
    /// Places held (or asked for) at contacts, by contact.
    places: HashMap<PeerId, ListenerId>,
    /// Places the contact agreed to.
    placed: HashSet<PeerId>,
    /// When to ask a contact for a place again.
    retry_place: HashMap<PeerId, Instant>,
    last_reach: Instant,
    /// The addresses last given to contacts.
    told: Option<Vec<Multiaddr>>,
}

impl State {
    fn handle(&mut self, swarm: &mut Swarm<Behaviour>, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(found))) => {
                for (peer, addr) in found {
                    if peer != *swarm.local_peer_id() && !swarm.is_connected(&peer) {
                        let dial = DialOpts::peer_id(peer)
                            .addresses(vec![addr.clone()])
                            .allocate_new_port()
                            .build();
                        if let Err(e) = swarm.dial(dial) {
                            tracing::debug!(%peer, %addr, error = %e, "dialing a discovered peer failed");
                        }
                    }
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                let addr = match &endpoint {
                    ConnectedPoint::Dialer { address, .. } => address,
                    ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr,
                };
                if endpoint.is_relayed() {
                    self.relayed
                        .entry(peer_id)
                        .or_default()
                        .insert(connection_id);
                    if self.direct.contains_key(&peer_id) {
                        swarm.close_connection(connection_id);
                    }
                } else {
                    self.direct
                        .entry(peer_id)
                        .or_default()
                        .insert(connection_id);
                    // Flight endpoints are only for a peer on this network.
                    if let Some(ip) = addr::ip_of(addr) {
                        if !self.shared.classifier().is_outside(addr) {
                            self.ips.insert(peer_id, ip);
                        }
                    }
                    // A direct connection replaces one through a contact.
                    for relayed in self.relayed.get(&peer_id).into_iter().flatten() {
                        swarm.close_connection(*relayed);
                    }
                }
                self.answer(peer_id);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                ..
            } => {
                for map in [&mut self.direct, &mut self.relayed] {
                    if let Some(ids) = map.get_mut(&peer_id) {
                        ids.remove(&connection_id);
                        if ids.is_empty() {
                            map.remove(&peer_id);
                        }
                    }
                }
                if !self.direct.contains_key(&peer_id) {
                    self.ips.remove(&peer_id);
                }
                self.answer(peer_id);
            }
            SwarmEvent::OutgoingConnectionError {
                peer_id: Some(peer),
                error,
                ..
            } => {
                // Still waiting on a meeting through a contact, or already
                // connected some other way.
                if !self.direct.contains_key(&peer) && !self.relayed.contains_key(&peer) {
                    self.fail(peer, &format!("couldn't connect: {error}"));
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result: Err(error),
            })) => {
                tracing::info!(peer = %remote_peer_id, error = %error, "hole punching failed");
                if !self.direct.contains_key(&remote_peer_id) {
                    for relayed in self.relayed.get(&remote_peer_id).into_iter().flatten() {
                        swarm.close_connection(*relayed);
                    }
                    self.fail(
                        remote_peer_id,
                        "met through a contact, but couldn't connect directly",
                    );
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result: Ok(_),
            })) => {
                tracing::info!(peer = %remote_peer_id, "connected directly after meeting through a contact");
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                if info.public_key.to_peer_id() != peer_id {
                    tracing::warn!(%peer_id, "identify key does not match the peer id");
                    return;
                }
                let Some(did) = did_of(info.public_key) else {
                    return;
                };
                // The connection is authenticated, so these are the peer's
                // own word on where it is reached.
                let classifier = self.shared.classifier();
                let addrs: Vec<String> = info
                    .listen_addrs
                    .iter()
                    .filter(|a| {
                        addr::ip_of(a).is_some_and(|ip| {
                            !ip.is_unspecified()
                                && (classifier.local_is_outside || !ip.is_loopback())
                        }) || addr::is_relayed(a)
                    })
                    .map(ToString::to_string)
                    .collect();
                if !addrs.is_empty() {
                    if let Err(e) = self
                        .catalog
                        .record_peer_addrs(did.as_str(), &addrs, now_ms())
                    {
                        tracing::debug!(%did, error = %e, "could not record a peer's addresses");
                    }
                }
                self.dids.insert(peer_id, did);
                if self.ips.contains_key(&peer_id) {
                    swarm
                        .behaviour_mut()
                        .endpoint
                        .send_request(&peer_id, EndpointRequest::default());
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Endpoint(request_response::Event::Message {
                peer,
                message,
                ..
            })) => match message {
                request_response::Message::Request { channel, .. } => {
                    let _ = swarm
                        .behaviour_mut()
                        .endpoint
                        .send_response(channel, self.flight.clone());
                }
                request_response::Message::Response { response, .. } => {
                    self.record(peer, &response);
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::RelayClient(
                relay::client::Event::ReservationReqAccepted { relay_peer_id, .. },
            )) => {
                if self.places.contains_key(&relay_peer_id) {
                    tracing::info!(contact = %relay_peer_id, "a contact keeps a place for this node");
                    self.placed.insert(relay_peer_id);
                    self.evaluate(swarm);
                }
            }
            SwarmEvent::ListenerClosed {
                listener_id,
                reason,
                ..
            } => {
                let contact = self
                    .places
                    .iter()
                    .find(|(_, id)| **id == listener_id)
                    .map(|(c, _)| *c);
                if let Some(contact) = contact {
                    tracing::info!(%contact, ?reason, "a contact no longer keeps a place for this node");
                    self.places.remove(&contact);
                    self.placed.remove(&contact);
                    self.retry_place
                        .insert(contact, Instant::now() + REACH_EVERY);
                    self.evaluate(swarm);
                }
            }
            SwarmEvent::NewListenAddr { .. } | SwarmEvent::ExpiredListenAddr { .. } => {
                self.evaluate(swarm);
            }
            _ => {}
        }
    }

    fn record(&self, peer: PeerId, response: &EndpointResponse) {
        let (Some(did), Some(ip)) = (self.dids.get(&peer), self.ips.get(&peer)) else {
            return;
        };
        let Some(url) = endpoint_url(&response.scheme, *ip, response.port) else {
            tracing::debug!(%peer, scheme = %response.scheme, "peer announced an unknown scheme");
            return;
        };
        match self.catalog.record_peer(did.as_str(), &url) {
            Ok(()) => tracing::info!(did = %did, endpoint = %url, "peer discovered"),
            Err(e) => {
                tracing::warn!(did = %did, endpoint = %url, error = %e, "could not record a peer")
            }
        }
    }

    /// Answer the callers waiting for `peer`, if it is connected directly
    /// and nothing through a contact is left that a stream could take.
    fn answer(&mut self, peer: PeerId) {
        if self.direct.contains_key(&peer) && !self.relayed.contains_key(&peer) {
            for waiter in self.waiting.remove(&peer).unwrap_or_default() {
                let _ = waiter.send(Ok(()));
            }
        }
    }

    fn fail(&mut self, peer: PeerId, why: &str) {
        for waiter in self.waiting.remove(&peer).unwrap_or_default() {
            let _ = waiter.send(Err(why.to_string()));
        }
    }

    /// The addresses kept for `peer`.
    fn addrs_of(&self, peer: &PeerId) -> Vec<Multiaddr> {
        let Some(did) = self.did_of_peer(peer) else {
            return Vec::new();
        };
        self.catalog
            .get_peer(did.as_str())
            .map(|p| p.addrs.iter().filter_map(|a| a.parse().ok()).collect())
            .unwrap_or_default()
    }

    fn did_of_peer(&self, peer: &PeerId) -> Option<Did> {
        if let Some(did) = self.dids.get(peer) {
            return Some(did.clone());
        }
        // An Ed25519 peer id holds its key (an identity multihash).
        let hash = peer.as_ref();
        if hash.code() != 0 {
            return None;
        }
        did_of(libp2p::identity::PublicKey::try_decode_protobuf(hash.digest()).ok()?)
    }

    /// Contacts that might introduce this node to others: ones with an
    /// address on the internet.
    fn meeting_points(&self, except: &PeerId) -> Vec<(PeerId, Multiaddr)> {
        let classifier = self.shared.classifier();
        let mut out = Vec::new();
        for contact in self.shared.contacts() {
            if contact == *except || Some(&contact) == Some(&self.keypair.public().to_peer_id()) {
                continue;
            }
            if let Some(addr) = self
                .addrs_of(&contact)
                .into_iter()
                .find(|a| classifier.is_public(a) && addr::tcp_port_of(a).is_some())
            {
                out.push((contact, addr));
            }
        }
        out
    }

    fn connect(
        &mut self,
        swarm: &mut Swarm<Behaviour>,
        peer: PeerId,
        reply: oneshot::Sender<Result<(), String>>,
    ) {
        if peer == *swarm.local_peer_id() {
            let _ = reply.send(Err("that is this node".to_string()));
            return;
        }
        if self.direct.contains_key(&peer) && !self.relayed.contains_key(&peer) {
            let _ = reply.send(Ok(()));
            return;
        }
        let first = !self.waiting.contains_key(&peer);
        self.waiting.entry(peer).or_default().push(reply);
        if !first || self.relayed.contains_key(&peer) || self.direct.contains_key(&peer) {
            // Already on its way: a dial, or a meeting through a contact.
            return;
        }
        let mut addrs = self.addrs_of(&peer);
        // Through each contact that might know it.
        for (contact, at) in self.meeting_points(&peer) {
            addrs.push(at.with(Protocol::P2p(contact)).with(Protocol::P2pCircuit));
        }
        if addrs.is_empty() {
            self.fail(
                peer,
                "no address is known for it, and no contact can introduce it",
            );
            return;
        }
        let dial = DialOpts::peer_id(peer)
            .condition(PeerCondition::DisconnectedAndNotDialing)
            .addresses(addrs)
            .build();
        if let Err(e) = swarm.dial(dial) {
            tracing::debug!(%peer, error = %e, "not dialing again");
        }
    }

    /// Work out how this node is reached, and act on it: ask the router,
    /// hold places at contacts, help contacts, update the signed record.
    fn evaluate(&mut self, swarm: &mut Swarm<Behaviour>) {
        let classifier = self.shared.classifier();
        let open = self.wanted.open;
        if let Some(mapping) = &self.mapping {
            mapping.send_if_modified(|on| {
                let changed = *on != open;
                *on = open;
                changed
            });
        }
        // The address the router opened, given to peers while wanted.
        let opened = match (&self.mapped, open) {
            (Some(Mapped::Open { external }), true) => Some(*external),
            _ => None,
        };
        let opened_addr: Option<Multiaddr> = opened.and_then(|e| {
            addr::is_public(IpAddr::V4(*e.ip()))
                .then(|| format!("/ip4/{}/tcp/{}", e.ip(), e.port()).parse().ok())
                .flatten()
        });
        if self.mapped_addr != opened_addr {
            if let Some(old) = self.mapped_addr.take() {
                swarm.remove_external_address(&old);
            }
            if let Some(new) = &opened_addr {
                swarm.add_external_address(new.clone());
            }
            self.mapped_addr = opened_addr.clone();
        }
        // Listening addresses others on the internet connect to.
        let public: HashSet<Multiaddr> = swarm
            .listeners()
            .filter(|a| {
                !addr::is_relayed(a)
                    && match self.reached_directly {
                        Some(reached) => {
                            reached && addr::ip_of(a).is_some_and(|ip| !ip.is_unspecified())
                        }
                        None => classifier.is_public(a),
                    }
            })
            .cloned()
            .collect();
        if public != self.public {
            for gone in self.public.difference(&public) {
                swarm.remove_external_address(gone);
            }
            for new in public.difference(&self.public) {
                swarm.add_external_address(new.clone());
            }
            self.public = public;
        }
        let on_the_internet = !self.public.is_empty();
        let direct = on_the_internet || opened_addr.is_some();

        // Places at contacts, while wanted and not reached directly.
        let want_places = open && !direct;
        if !want_places {
            for (_, id) in self.places.drain() {
                swarm.remove_listener(id);
            }
            self.placed.clear();
        } else if self.places.len() < MAX_PLACES {
            let now = Instant::now();
            let me = *swarm.local_peer_id();
            for (contact, at) in self.meeting_points(&me) {
                if self.places.len() >= MAX_PLACES {
                    break;
                }
                if self.places.contains_key(&contact)
                    || self.retry_place.get(&contact).is_some_and(|t| *t > now)
                {
                    continue;
                }
                let place = at.with(Protocol::P2p(contact)).with(Protocol::P2pCircuit);
                match swarm.listen_on(place) {
                    Ok(id) => {
                        self.places.insert(contact, id);
                    }
                    Err(e) => {
                        tracing::debug!(%contact, error = %e, "couldn't ask a contact for a place");
                        self.retry_place.insert(contact, now + REACH_EVERY);
                    }
                }
            }
        }

        let reach = if direct {
            Reach::Direct
        } else if !open {
            Reach::LocalOnly
        } else if let Some(contact) = self
            .placed
            .iter()
            .filter_map(|c| self.did_of_peer(c))
            .min_by(|a, b| a.as_str().cmp(b.as_str()))
        {
            Reach::ThroughContact(contact)
        } else {
            match &self.mapped {
                None => Reach::Opening,
                Some(Mapped::Open { .. }) => Reach::SharedAddress,
                Some(Mapped::Refused(_)) => Reach::RouterRefused,
            }
        };
        let helping = self.wanted.help_contacts && direct;
        if helping != self.shared.helping() {
            // Offered only now: libp2p would offer it on its own only once
            // it confirmed an address on the internet.
            swarm.behaviour_mut().relay.set_status(Some(if helping {
                relay::Status::Enable
            } else {
                relay::Status::Disable
            }));
        }
        self.shared.set_helping(helping);
        let status = NetworkStatus {
            wanted: self.wanted,
            reach,
            helping,
        };
        // The signed record, for invites.
        let mut addrs: Vec<Multiaddr> = swarm
            .listeners()
            .filter(|a| {
                addr::is_relayed(a)
                    || addr::ip_of(a).is_some_and(|ip| {
                        !ip.is_unspecified() && (classifier.local_is_outside || !ip.is_loopback())
                    })
            })
            .cloned()
            .collect();
        for external in swarm.external_addresses() {
            if !addrs.contains(external) {
                addrs.push(external.clone());
            }
        }
        // Outside addresses first: they are what an invite is for.
        addrs.sort_by_key(|a| !classifier.is_public(a) && !addr::is_relayed(a));
        if self.told.as_ref() != Some(&addrs) {
            let signed = record::sign(&self.keypair, addrs.clone());
            *self.net.record.write().unwrap_or_else(|p| p.into_inner()) = signed;
            // Contacts not connected now learn the new addresses when they
            // connect (identify); those connected are told at once.
            if open && self.told.is_some() {
                self.tell_contacts(swarm);
            }
            self.told = Some(addrs);
        }
        // Last, so an invite made once the status settles carries the
        // record that goes with it.
        let mut now = self.net.status.write().unwrap_or_else(|p| p.into_inner());
        if *now != status {
            tracing::info!(?status, "reach");
            *now = status;
        }
    }

    /// Connect to contacts this node isn't connected to, so they learn
    /// where it is reached now.
    fn tell_contacts(&mut self, swarm: &mut Swarm<Behaviour>) {
        for contact in self.shared.contacts() {
            if swarm.is_connected(&contact) {
                continue;
            }
            let addrs = self.addrs_of(&contact);
            if addrs.is_empty() {
                continue;
            }
            let dial = DialOpts::peer_id(contact)
                .condition(PeerCondition::DisconnectedAndNotDialing)
                .addresses(addrs)
                .build();
            let _ = swarm.dial(dial);
        }
    }

    fn tick(&mut self, swarm: &mut Swarm<Behaviour>) {
        self.shared.reload_contacts();
        self.waiting.retain(|_, waiters| {
            waiters.retain(|w| !w.is_closed());
            !waiters.is_empty()
        });
        if self.last_reach.elapsed() >= REACH_EVERY {
            self.last_reach = Instant::now();
            self.retry_place.retain(|_, t| *t > Instant::now());
            self.evaluate(swarm);
        }
    }
}

async fn run(
    mut swarm: Swarm<Behaviour>,
    mut commands: mpsc::Receiver<Command>,
    mut reach: watch::Receiver<ReachWanted>,
    mut state: State,
) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    state.evaluate(&mut swarm);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Dial(addr, reply)) => {
                    let dial = DialOpts::unknown_peer_id()
                        .address(addr)
                        .allocate_new_port()
                        .build();
                    let _ = reply.send(swarm.dial(dial).map_err(libp2p_err));
                }
                Some(Command::Connect(peer, reply)) => state.connect(&mut swarm, peer, reply),
                Some(Command::Mapped(mapped)) => {
                    if state.mapped.as_ref() != Some(&mapped) {
                        tracing::info!(?mapped, "the router answered");
                    }
                    state.mapped = Some(mapped);
                    state.evaluate(&mut swarm);
                }
                None => break,
            },
            changed = reach.changed() => {
                if changed.is_err() {
                    break;
                }
                state.wanted = *reach.borrow_and_update();
                if !state.wanted.open && state.mapping.is_some() {
                    // Asked again when wanted again.
                    state.mapped = None;
                }
                state.evaluate(&mut swarm);
            }
            _ = tick.tick() => state.tick(&mut swarm),
            event = swarm.select_next_some() => state.handle(&mut swarm, event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_libp2p_identity_is_the_did_key() {
        let identity = NodeIdentity::from_seed(&[9; 32]);
        let keypair = libp2p_keypair(&identity).unwrap();
        assert_eq!(did_of(keypair.public()).unwrap(), *identity.did());
    }

    #[test]
    fn endpoints_are_built_from_the_connection_address() {
        let v4: IpAddr = "10.0.0.5".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        assert_eq!(
            endpoint_url("https", v4, 50051).as_deref(),
            Some("https://10.0.0.5:50051")
        );
        assert_eq!(
            endpoint_url("http", v6, 7).as_deref(),
            Some("http://[fe80::1]:7")
        );
        assert_eq!(endpoint_url("ftp", v4, 1), None);
    }

    #[test]
    fn the_port_from_last_time_is_asked_for_only_when_any_will_do() {
        let any: Multiaddr = "/ip4/0.0.0.0/tcp/0".parse().unwrap();
        let fixed: Multiaddr = "/ip4/0.0.0.0/tcp/4001".parse().unwrap();
        assert_eq!(
            with_kept_port(&any, Some(52000)),
            Some("/ip4/0.0.0.0/tcp/52000".parse().unwrap())
        );
        assert_eq!(with_kept_port(&fixed, Some(52000)), None);
        assert_eq!(with_kept_port(&any, None), None);
    }
}
