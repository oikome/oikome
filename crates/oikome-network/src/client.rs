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

//! [`PeerClient`]: signed Arrow Flight calls to the owner of a remote Space.
//!
//! Both ways of reading another node's Space go through it: query shipping
//! ([`FlightEngine`](crate::FlightEngine)) and subscription
//! ([`PeerClient::snapshot`], [`PeerClient::fetch_file`]).

use std::fmt;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::error::FlightError;
use arrow_flight::{Action, FlightClient, FlightDescriptor};
use arrow_schema::SchemaRef;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use oikome_engine::EngineError;
use oikome_identity::{unix_now_ms, AuthRequest, Did, NodeIdentity};
use tokio::sync::OnceCell;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::Code;

use crate::protocol::{
    digest, ComputeMethod, ComputeRequest, FetchFileRequest, SnapshotRequest, SnapshotResponse,
    ACTION_COMPUTE_METHODS, ACTION_FETCH_FILE, ACTION_SNAPSHOT, HEADER_AUTH, HEADER_UCAN,
    METHOD_COMPUTE, METHOD_COMPUTE_METHODS, METHOD_FETCH_FILE, METHOD_SNAPSHOT,
};
use crate::route::{PeerDialer, PeerRoute, RouteConnector, CONNECT_TIMEOUT, DIAL_TIMEOUT};
use crate::tls::{install_crypto_provider, DidCertVerifier};

/// TLS server name sent to every peer; peers are identified by DID instead.
const PEER_SERVER_NAME: &str = "oikome.peer";

/// `e` and its sources, so a TLS failure says why the peer was refused.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// A stream of file chunks from [`PeerClient::fetch_file`].
pub type ChunkStream = BoxStream<'static, Result<Bytes, EngineError>>;

/// Calls one peer on behalf of this node, presenting one UCAN. Connects on
/// first use: to the peer's Flight endpoint, and when that doesn't answer
/// (or none is known) over the node's peer-to-peer connection to it.
pub struct PeerClient {
    peer: String,
    owner: Did,
    token: String,
    identity: Arc<NodeIdentity>,
    allow_http: bool,
    dialer: Option<Arc<dyn PeerDialer>>,
    channel: OnceCell<Channel>,
}

impl fmt::Debug for PeerClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerClient")
            .field("peer", &self.peer)
            .field("owner", &self.owner)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

pub(crate) fn status_to_engine(status: &tonic::Status) -> EngineError {
    match status.code() {
        Code::PermissionDenied | Code::Unauthenticated => {
            EngineError::PermissionDenied(status.message().to_string())
        }
        Code::NotFound => EngineError::TableNotFound(status.message().to_string()),
        Code::ResourceExhausted => EngineError::LimitExceeded(status.message().to_string()),
        code => EngineError::Remote(format!("{code:?}: {}", status.message())),
    }
}

pub(crate) fn flight_to_engine(e: FlightError) -> EngineError {
    match e {
        FlightError::Tonic(status) => status_to_engine(&status),
        FlightError::Arrow(e) => EngineError::Arrow(e),
        other => EngineError::Remote(other.to_string()),
    }
}

impl PeerClient {
    /// A client for the node `owner` at `peer` (`https://host:port`; empty
    /// when only a [dialer](Self::with_dialer) reaches it), signing as
    /// `identity` and presenting `token`. Plaintext peers (`http://`) are
    /// refused unless [`allow_http`](Self::allow_http) is set.
    pub fn new(peer: String, owner: Did, token: String, identity: Arc<NodeIdentity>) -> Self {
        Self {
            peer,
            owner,
            token,
            identity,
            allow_http: false,
            dialer: None,
            channel: OnceCell::new(),
        }
    }

    /// A client for the node `owner` along `route`.
    pub fn routed(
        route: PeerRoute,
        owner: Did,
        token: String,
        identity: Arc<NodeIdentity>,
    ) -> Self {
        Self::new(route.endpoint.unwrap_or_default(), owner, token, identity)
            .with_dialer(route.dialer)
    }

    /// Also reach the peer over the node's peer-to-peer connections.
    pub fn with_dialer(mut self, dialer: Option<Arc<dyn PeerDialer>>) -> Self {
        self.dialer = dialer;
        self
    }

    /// Accept a plaintext (`http://`) peer.
    pub fn allow_http(mut self, allow: bool) -> Self {
        self.allow_http = allow;
        self
    }

    /// The peer's Flight endpoint.
    pub fn peer(&self) -> &str {
        &self.peer
    }

    async fn channel(&self) -> Result<Channel, EngineError> {
        self.channel
            .get_or_try_init(|| async {
                if self.peer.starts_with("http://") && !self.allow_http {
                    return Err(EngineError::PermissionDenied(format!(
                        "{} is plaintext and this node does not allow http peers",
                        self.peer
                    )));
                }
                let route = PeerRoute {
                    endpoint: (!self.peer.is_empty()).then(|| self.peer.clone()),
                    dialer: self.dialer.clone(),
                };
                if route.is_empty() {
                    return Err(EngineError::Remote(format!(
                        "no way to reach {} is known",
                        self.owner
                    )));
                }
                // With a dialer the connector tries both ways itself; the
                // URL only says whether TLS runs on top.
                let url = match route.dialer {
                    Some(_) => format!("{}://{PEER_SERVER_NAME}", route.scheme()),
                    None => self.peer.clone(),
                };
                let mut endpoint = Endpoint::from_shared(url)
                    .map_err(|e| EngineError::Remote(format!("invalid peer address: {e}")))?;
                endpoint = match route.dialer {
                    Some(_) => endpoint.connect_timeout(CONNECT_TIMEOUT + DIAL_TIMEOUT),
                    None => endpoint.connect_timeout(CONNECT_TIMEOUT),
                };
                if route.scheme() == "https" {
                    install_crypto_provider();
                    // The name is not checked (the certificate is pinned to
                    // the owner's DID); a fixed one avoids IPv6 literals.
                    let tls = ClientTlsConfig::new().domain_name(PEER_SERVER_NAME);
                    let pinned = DidCertVerifier::new(self.owner.public_key().to_bytes());
                    endpoint = endpoint
                        .tls_config_with_verifier(tls, pinned)
                        .map_err(|e| EngineError::Remote(format!("TLS setup: {e}")))?;
                }
                let target = if self.peer.is_empty() {
                    self.owner.to_string()
                } else {
                    self.peer.clone()
                };
                let connected = match route.dialer.is_some() {
                    true => {
                        endpoint
                            .connect_with_connector(RouteConnector {
                                route,
                                peer: self.owner.clone(),
                            })
                            .await
                    }
                    false => endpoint.connect().await,
                };
                connected.map_err(|e| {
                    EngineError::Remote(format!("cannot reach {target}: {}", error_chain(&e)))
                })
            })
            .await
            .cloned()
    }

    /// A Flight client whose headers authorize exactly one call: `method`
    /// with `payload`.
    pub async fn signed(&self, method: &str, payload: &[u8]) -> Result<FlightClient, EngineError> {
        let mut client = FlightClient::new(self.channel().await?);
        let digest = digest(method, payload, &self.token);
        let auth = AuthRequest::sign(&self.identity, &self.owner, unix_now_ms(), &digest)
            .and_then(|a| a.to_header())
            .map_err(|e| EngineError::Remote(e.to_string()))?;
        client
            .add_header(HEADER_AUTH, &auth)
            .map_err(flight_to_engine)?;
        client
            .add_header(HEADER_UCAN, &self.token)
            .map_err(flight_to_engine)?;
        Ok(client)
    }

    async fn action(
        &self,
        method: &str,
        action: &'static str,
        body: Vec<u8>,
    ) -> Result<ChunkStream, EngineError> {
        let mut client = self.signed(method, &body).await?;
        let results = client
            .do_action(Action::new(action, body))
            .await
            .map_err(flight_to_engine)?;
        Ok(results.map_err(flight_to_engine).boxed())
    }

    /// The current snapshot of the tables of `space` (in the owner's
    /// namespace) this node may copy. Needs `space/read`.
    pub async fn snapshot(&self, space: &str) -> Result<SnapshotResponse, EngineError> {
        let body = serde_json::to_vec(&SnapshotRequest {
            space: space.to_string(),
        })
        .map_err(|e| EngineError::Remote(e.to_string()))?;
        let mut results = self.action(METHOD_SNAPSHOT, ACTION_SNAPSHOT, body).await?;
        let first = results
            .try_next()
            .await?
            .ok_or_else(|| EngineError::Remote("empty snapshot response".to_string()))?;
        serde_json::from_slice(&first)
            .map_err(|e| EngineError::Remote(format!("snapshot response: {e}")))
    }

    /// Run a compute task on the peer (§12): `request` over `batches` of
    /// `schema`. Needs `compute/<method>`.
    pub async fn compute(
        &self,
        request: &ComputeRequest,
        schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(SchemaRef, Vec<RecordBatch>), EngineError> {
        let cmd = serde_json::to_vec(request).map_err(|e| EngineError::Remote(e.to_string()))?;
        let mut client = self.signed(METHOD_COMPUTE, &cmd).await?;
        let input = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .with_flight_descriptor(Some(FlightDescriptor::new_cmd(cmd)))
            .build(futures::stream::iter(batches.into_iter().map(Ok)));
        let mut results = client.do_exchange(input).await.map_err(flight_to_engine)?;
        let mut out = Vec::new();
        while let Some(batch) = results.try_next().await.map_err(flight_to_engine)? {
            out.push(batch);
        }
        let schema = results
            .schema()
            .cloned()
            .ok_or_else(|| EngineError::Remote("a compute result without a schema".to_string()))?;
        Ok((schema, out))
    }

    /// The compute methods the peer offers this node, with its limits.
    pub async fn compute_methods(&self) -> Result<Vec<ComputeMethod>, EngineError> {
        let mut results = self
            .action(
                METHOD_COMPUTE_METHODS,
                ACTION_COMPUTE_METHODS,
                b"{}".to_vec(),
            )
            .await?;
        let first = results
            .try_next()
            .await?
            .ok_or_else(|| EngineError::Remote("empty compute_methods response".to_string()))?;
        serde_json::from_slice(&first)
            .map_err(|e| EngineError::Remote(format!("compute_methods response: {e}")))
    }

    /// Stream one data file of a snapshot. The caller checks size and hash.
    pub async fn fetch_file(&self, request: &FetchFileRequest) -> Result<ChunkStream, EngineError> {
        let body = serde_json::to_vec(request).map_err(|e| EngineError::Remote(e.to_string()))?;
        self.action(METHOD_FETCH_FILE, ACTION_FETCH_FILE, body)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(peer: &str) -> PeerClient {
        PeerClient::new(
            peer.to_string(),
            NodeIdentity::from_seed(&[1; 32]).did().clone(),
            "header.payload.signature".to_string(),
            Arc::new(NodeIdentity::from_seed(&[2; 32])),
        )
    }

    #[tokio::test]
    async fn plaintext_peers_need_allow_http() {
        let refused = client("http://127.0.0.1:1").snapshot("blog").await;
        assert!(
            matches!(&refused, Err(EngineError::PermissionDenied(m)) if m.contains("plaintext")),
            "{refused:?}"
        );
        // Allowed: it gets as far as connecting (nothing listens there).
        let allowed = client("http://127.0.0.1:1")
            .allow_http(true)
            .snapshot("blog")
            .await;
        assert!(
            matches!(allowed, Err(EngineError::Remote(_))),
            "{allowed:?}"
        );
    }

    #[test]
    fn debug_never_prints_the_token() {
        let client = PeerClient::new(
            "http://127.0.0.1:1".to_string(),
            NodeIdentity::from_seed(&[1; 32]).did().clone(),
            "header.payload.signature".to_string(),
            Arc::new(NodeIdentity::from_seed(&[2; 32])),
        );
        let text = format!("{client:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("payload"));
    }
}
