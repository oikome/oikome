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

//! The Oikome node server (architecture v3 §9): the Arrow Flight endpoint
//! through which peers query this node's Spaces with UCANs, the owner's web
//! API (§13.4), and the `oikome` binary.

pub mod mcp;
pub mod replay;
pub mod service;
pub mod web;

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use arrow_flight::flight_service_server::FlightServiceServer;
use futures::stream::BoxStream;
use futures::StreamExt;
use oikome_identity::TlsIdentity;
use oikome_network::{install_crypto_provider, PeerIo};
pub use oikome_webrtc::DtlsIdentity;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tonic::transport::server::{Connected, TcpConnectInfo, TcpIncoming};
use tonic::transport::{Identity, Server, ServerTlsConfig};

pub use service::OikomeFlightService;
pub use web::WebApp;

/// Largest header block accepted: the UCAN, with its embedded proofs,
/// travels in a header (gRPC servers default to 16 KiB).
pub const MAX_HEADER_LIST_BYTES: u32 = 256 * 1024;

/// A connection to the Flight service: TCP from this network, or a stream
/// of a peer-to-peer connection from another (3.12). TLS runs on top of
/// either.
pub enum FlightIo {
    Tcp(TcpStream),
    Peer(PeerIo),
}

impl AsyncRead for FlightIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            FlightIo::Tcp(io) => Pin::new(io).poll_read(cx, buf),
            FlightIo::Peer(io) => Pin::new(io).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for FlightIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            FlightIo::Tcp(io) => Pin::new(io).poll_write(cx, buf),
            FlightIo::Peer(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            FlightIo::Tcp(io) => Pin::new(io).poll_flush(cx),
            FlightIo::Peer(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            FlightIo::Tcp(io) => Pin::new(io).poll_shutdown(cx),
            FlightIo::Peer(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}

impl Connected for FlightIo {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> TcpConnectInfo {
        match self {
            FlightIo::Tcp(io) => io.connect_info(),
            // The caller is known by its DID, not an address.
            FlightIo::Peer(_) => TcpConnectInfo {
                local_addr: None,
                remote_addr: None,
            },
        }
    }
}

/// Serve `service` on `listener` until `shutdown` resolves: over TLS with
/// `tls` (the node's own certificate, see `NodeIdentity::tls_identity`), in
/// plaintext with `None`.
pub async fn serve(
    listener: TcpListener,
    service: impl Into<Arc<OikomeFlightService>>,
    tls: Option<&TlsIdentity>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), tonic::transport::Error> {
    serve_with_peers(listener, None, service, tls, shutdown).await
}

/// [`serve`], also answering Flight on the streams peers open over
/// peer-to-peer connections (`oikome_discovery::Discovery::take_flight_streams`).
pub async fn serve_with_peers(
    listener: TcpListener,
    peers: Option<BoxStream<'static, PeerIo>>,
    service: impl Into<Arc<OikomeFlightService>>,
    tls: Option<&TlsIdentity>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), tonic::transport::Error> {
    let tcp = TcpIncoming::from(listener).map(|r| r.map(FlightIo::Tcp));
    let peers = peers
        .unwrap_or_else(|| futures::stream::pending().boxed())
        .map(|io| Ok::<_, io::Error>(FlightIo::Peer(io)));
    let incoming = futures::stream::select(tcp, peers);
    let mut builder = Server::builder();
    if let Some(tls) = tls {
        install_crypto_provider();
        builder = builder.tls_config(
            ServerTlsConfig::new()
                .identity(Identity::from_pem(&tls.cert_pem, tls.key_pem.as_bytes())),
        )?;
    }
    builder
        .http2_max_header_list_size(MAX_HEADER_LIST_BYTES)
        .add_service(FlightServiceServer::from_arc(service.into()))
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await
}

/// Serve browsers over WebRTC data channels on `socket` (architecture v3
/// §10) until `shutdown` resolves, answering with the same `service` as
/// Arrow Flight. `dtls` is the certificate the node's access record pins.
pub async fn serve_webrtc(
    socket: tokio::net::UdpSocket,
    dtls: &DtlsIdentity,
    service: Arc<OikomeFlightService>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), oikome_webrtc::ServerError> {
    oikome_webrtc::serve(socket, dtls, service, shutdown).await
}
