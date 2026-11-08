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

//! How a call reaches another node's Flight service (architecture v3 §9,
//! roadmap 3.12).
//!
//! On a network both nodes share, a call goes straight to the peer's Flight
//! endpoint over TCP. Across networks it goes over the node's
//! peer-to-peer connection to the peer (its router opened a port, or a
//! contact helped the two punch through their routers): a [`PeerDialer`]
//! opens a byte stream to a DID there. Either way TLS runs on top and the
//! certificate is pinned to the peer's DID, so the path is never trusted.
//!
//! Only the serving process holds the peer-to-peer connections. Other
//! processes of the node (`oikome sql`) borrow them through a socket in the
//! node's directory that only the owner can open ([`serve_peer_socket`],
//! [`SocketDialer`]).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::BoxFuture;
use hyper_util::rt::TokioIo;
use oikome_identity::Did;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tonic::transport::Uri;

/// How long to wait for a TCP connection to a peer's Flight endpoint.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for a peer-to-peer connection: a dial, or a meeting at
/// a contact and the hole punching after it.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(30);
/// The socket in the node's directory (`.oikome/`) other processes of the
/// node reach peers through.
pub const PEER_SOCKET: &str = "peers.sock";
/// A DID is far shorter; anything longer on the socket is not one.
const MAX_SOCKET_LINE: usize = 256;

/// A byte stream to another node.
pub trait PeerStream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> PeerStream for T {}

/// A boxed [`PeerStream`].
pub type PeerIo = Box<dyn PeerStream>;

/// Opens byte streams to other nodes by DID, over the node's peer-to-peer
/// connections.
pub trait PeerDialer: Send + Sync + 'static {
    /// A stream to the Flight service of `peer`.
    fn open(&self, peer: &Did) -> BoxFuture<'static, io::Result<PeerIo>>;
}

/// Where a peer's Flight service is reached: its endpoint on a network this
/// node shares with it, its peer-to-peer connection, or both (the endpoint
/// is tried first).
#[derive(Clone, Default)]
pub struct PeerRoute {
    /// `https://host:port` (`http://` for a plaintext node).
    pub endpoint: Option<String>,
    pub dialer: Option<Arc<dyn PeerDialer>>,
}

impl fmt::Debug for PeerRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerRoute")
            .field("endpoint", &self.endpoint)
            .field("dialer", &self.dialer.is_some())
            .finish()
    }
}

impl PeerRoute {
    /// Straight to `endpoint`.
    pub fn endpoint(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: Some(endpoint.into()),
            dialer: None,
        }
    }

    /// Whether there is any way to the peer.
    pub fn is_empty(&self) -> bool {
        self.endpoint.is_none() && self.dialer.is_none()
    }

    /// `http` for a plaintext endpoint, else `https`.
    pub fn scheme(&self) -> &'static str {
        match &self.endpoint {
            Some(e) if e.starts_with("http://") => "http",
            _ => "https",
        }
    }
}

/// `host:port` of an endpoint URL.
fn authority(endpoint: &str) -> &str {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// Connects a tonic channel along a [`PeerRoute`]: the endpoint over TCP,
/// then the peer-to-peer stream. TLS (pinned to the DID) is added by tonic.
#[derive(Clone)]
pub(crate) struct RouteConnector {
    pub(crate) route: PeerRoute,
    pub(crate) peer: Did,
}

impl tower::Service<Uri> for RouteConnector {
    type Response = TokioIo<PeerIo>;
    type Error = io::Error;
    type Future = BoxFuture<'static, io::Result<TokioIo<PeerIo>>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Uri) -> Self::Future {
        let route = self.route.clone();
        let peer = self.peer.clone();
        Box::pin(async move {
            let mut failures = Vec::new();
            if let Some(endpoint) = &route.endpoint {
                let addr = authority(endpoint).to_string();
                match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr)).await {
                    Ok(Ok(tcp)) => {
                        let _ = tcp.set_nodelay(true);
                        return Ok(TokioIo::new(Box::new(tcp) as PeerIo));
                    }
                    Ok(Err(e)) => failures.push(format!("{addr}: {e}")),
                    Err(_) => failures.push(format!("{addr}: no answer")),
                }
            }
            if let Some(dialer) = &route.dialer {
                match dialer.open(&peer).await {
                    Ok(io) => return Ok(TokioIo::new(io)),
                    Err(e) => failures.push(e.to_string()),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                if failures.is_empty() {
                    "no way to reach it is known".to_string()
                } else {
                    failures.join("; ")
                },
            ))
        })
    }
}

/// The socket other processes of the node at `oikome_dir` reach peers
/// through.
pub fn peer_socket_path(oikome_dir: &Path) -> PathBuf {
    oikome_dir.join(PEER_SOCKET)
}

/// Reaches peers through the serving process of the same node (see
/// [`serve_peer_socket`]).
#[derive(Debug, Clone)]
pub struct SocketDialer {
    path: PathBuf,
}

impl SocketDialer {
    /// A dialer through the socket at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl PeerDialer for SocketDialer {
    #[cfg(unix)]
    fn open(&self, peer: &Did) -> BoxFuture<'static, io::Result<PeerIo>> {
        let path = self.path.clone();
        let line = format!("{peer}\n");
        Box::pin(async move {
            let socket = tokio::net::UnixStream::connect(&path).await.map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "the node isn't serving, so it can't reach peers on other networks ({e})"
                    ),
                )
            })?;
            let mut socket = BufReader::new(socket);
            socket.get_mut().write_all(line.as_bytes()).await?;
            let mut answer = String::new();
            tokio::time::timeout(
                DIAL_TIMEOUT + Duration::from_secs(5),
                (&mut socket)
                    .take(MAX_SOCKET_LINE as u64)
                    .read_line(&mut answer),
            )
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no answer from the node"))??;
            match answer.trim_end().strip_prefix("err ") {
                None if answer.trim_end() == "ok" => Ok(Box::new(socket) as PeerIo),
                Some(message) => Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    message.to_string(),
                )),
                None => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "an unexpected answer from the node",
                )),
            }
        })
    }

    #[cfg(not(unix))]
    fn open(&self, _: &Did) -> BoxFuture<'static, io::Result<PeerIo>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "reaching peers on other networks works from the serving process only here",
            ))
        })
    }
}

/// Let the node's other processes reach peers through `dialer`, on a
/// socket at `path` that only the owner can open, until `shutdown`
/// resolves. Each connection names a DID on its first line and is answered
/// `ok` (then the stream to that peer follows) or `err <why>`.
#[cfg(unix)]
pub async fn serve_peer_socket(
    path: &Path,
    dialer: Arc<dyn PeerDialer>,
    shutdown: impl std::future::Future<Output = ()>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // One left by a process that stopped without removing it.
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    tokio::pin!(shutdown);
    loop {
        let (socket, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            () = &mut shutdown => break,
        };
        let dialer = dialer.clone();
        tokio::spawn(async move {
            if let Err(e) = proxy(socket, dialer).await {
                tracing::debug!(error = %e, "a peer stream for another process ended");
            }
        });
    }
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(unix)]
async fn proxy(socket: tokio::net::UnixStream, dialer: Arc<dyn PeerDialer>) -> io::Result<()> {
    let mut socket = BufReader::new(socket);
    let mut line = String::new();
    (&mut socket)
        .take(MAX_SOCKET_LINE as u64)
        .read_line(&mut line)
        .await?;
    let Ok(peer) = Did::parse(line.trim_end()) else {
        socket.get_mut().write_all(b"err not a DID\n").await?;
        return Ok(());
    };
    match dialer.open(&peer).await {
        Ok(mut stream) => {
            socket.get_mut().write_all(b"ok\n").await?;
            tokio::io::copy_bidirectional(&mut socket, &mut stream).await?;
        }
        Err(e) => {
            let why: String = e
                .to_string()
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            socket
                .get_mut()
                .write_all(format!("err {why}\n").as_bytes())
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_dialed_by_their_authority() {
        assert_eq!(authority("https://10.0.0.5:50051"), "10.0.0.5:50051");
        assert_eq!(authority("http://[::1]:7/"), "[::1]:7");
        assert_eq!(PeerRoute::endpoint("http://a:1").scheme(), "http");
        assert_eq!(PeerRoute::default().scheme(), "https");
        assert!(PeerRoute::default().is_empty());
    }

    /// Answers every open but one with a stream that echoes, as a peer
    /// would carry bytes.
    struct Echo {
        unreachable: Did,
    }

    impl PeerDialer for Echo {
        fn open(&self, peer: &Did) -> BoxFuture<'static, io::Result<PeerIo>> {
            let refuse = *peer == self.unreachable;
            Box::pin(async move {
                if refuse {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "no route\nhere",
                    ));
                }
                let (near, mut far) = tokio::io::duplex(1024);
                tokio::spawn(async move {
                    let (mut r, mut w) = tokio::io::split(&mut far);
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
                Ok(Box::new(near) as PeerIo)
            })
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn other_processes_reach_peers_through_the_socket() {
        let unreachable = oikome_identity::NodeIdentity::from_seed(&[4; 32])
            .did()
            .clone();
        let dir = tempfile::Builder::new().prefix("sock").tempdir().unwrap();
        let path = dir.path().join(PEER_SOCKET);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn({
            let path = path.clone();
            async move {
                serve_peer_socket(&path, Arc::new(Echo { unreachable }), async {
                    let _ = stopped.await;
                })
                .await
            }
        });
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let peer = oikome_identity::NodeIdentity::from_seed(&[3; 32])
            .did()
            .clone();
        let dialer = SocketDialer::new(path.clone());
        let mut stream = dialer.open(&peer).await.unwrap();
        stream.write_all(b"hello").await.unwrap();
        let mut back = [0u8; 5];
        stream.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"hello");

        // A refusal comes back as an error, on one line.
        let unreachable = oikome_identity::NodeIdentity::from_seed(&[4; 32])
            .did()
            .clone();
        let err = dialer.open(&unreachable).await.err().unwrap();
        assert!(err.to_string().contains("no route here"), "{err}");

        let _ = stop.send(());
        server.await.unwrap().unwrap();
        assert!(!path.exists());
    }
}
