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

//! `oikome`: run an Oikome node.
//!
//! ```text
//! oikome [--root DIR] identity          print this node's DID
//! oikome [--root DIR] sql "<SQL>"       run statements as the node owner
//! oikome [--root DIR] serve             serve this node's Spaces to peers
//! oikome [--root DIR] extensions install
//!                                       fetch the DuckDB extensions Oikome uses
//! oikome [--root DIR] storage add|list|remove
//!                                       manage storages (a NAS) for Spaces
//! ```
//!
//! DuckDB extensions (`lance` for Lance Spaces, `duckpgq` for their graphs,
//! `sqlite` for analytical Spaces over SQLite files) are loaded from
//! `<root>/.oikome/extensions`, or
//! `--extension-dir` / `OIKOME_EXTENSION_DIR`. Queries never download them;
//! `extensions install` does, signed by DuckDB.
//!
//! The node key is read from `OIKOME_NODE_KEY` (base64url seed) when set,
//! otherwise from `<root>/.oikome/identity/node_ed25519`, created on first use.
//!
//! A storage's password, token and encryption key are kept in the OS
//! keychain (a private file under `<root>/.oikome/identity/` where there is
//! none); `OIKOME_STORAGE_<NAME>_PASSWORD`, `_TOKEN` and `_KEY` override
//! them, for servers and containers.

use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_cast::pretty::pretty_format_batches;
use clap::{Parser, Subcommand};
use iceberg::io::FileIO;
use oikome_catalog::{OikomeCatalog, StorageKind};
use oikome_discovery::{Discovery, DiscoveryConfig, EndpointResponse};
use oikome_engine_duckdb::{DuckDbConfig, DuckDbEngine, Extension};
use oikome_node::{
    spawn_storage_sync, EngineRegistry, NodeConfig, OikomeSession, QueryResult, SessionConfig,
    StorageCredentials,
};
use oikome_server::{serve, serve_webrtc, DtlsIdentity, OikomeFlightService};
use tokio::net::{TcpListener, UdpSocket};
use zeroize::Zeroizing;

/// The node's DTLS certificate for WebRTC, next to its key.
const WEBRTC_DTLS_FILE: &str = "webrtc_dtls.json";
/// How long an access record printed by `SHOW WEBRTC` stays valid.
const WEBRTC_RECORD_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

#[derive(Parser)]
#[command(
    name = "oikome",
    version,
    about = "A decentralized, local-first federated query layer"
)]
struct Cli {
    /// Data directory of this node.
    #[arg(
        long,
        env = "OIKOME_ROOT",
        default_value = "./oikome-data",
        global = true
    )]
    root: PathBuf,

    /// Accept plaintext (http://) peers. By default peers are reached over
    /// TLS, their certificate pinned to their DID.
    #[arg(long, env = "OIKOME_ALLOW_HTTP", global = true)]
    allow_http: bool,

    /// Directory of DuckDB extensions; defaults to `<root>/.oikome/extensions`.
    #[arg(long, env = "OIKOME_EXTENSION_DIR", global = true)]
    extension_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum ExtensionsCommand {
    /// Download the DuckDB extensions Oikome loads (`sqlite`, `lance`,
    /// `duckpgq`) for the linked DuckDB version.
    Install,
}

#[derive(Subcommand)]
enum StorageCommand {
    /// Register a storage Spaces can live on: a WebDAV server such as a NAS,
    /// a mounted folder, or a cloud drive's sync folder. Credentials given
    /// on standard input are kept in the OS keychain (or a private file
    /// where there is none); OIKOME_STORAGE_<NAME>_PASSWORD, _TOKEN and _KEY
    /// override them. The storage is checked before it is registered.
    Add {
        /// Lowercase letters, digits and `_`, starting with a letter.
        name: String,
        /// WebDAV address, https://host[:port][/path].
        #[arg(long, group = "place", required_unless_present_any = ["folder", "drive"])]
        webdav: Option<String>,
        /// WebDAV user name.
        #[arg(long, requires = "webdav")]
        username: Option<String>,
        /// A mounted folder (absolute path), such as a NAS share.
        #[arg(long, group = "place")]
        folder: Option<PathBuf>,
        /// A folder a cloud drive's desktop client syncs (iCloud Drive,
        /// Dropbox, OneDrive, Google Drive); always encrypted.
        #[arg(long, group = "place")]
        drive: Option<PathBuf>,
        /// Encrypt files before they reach the storage. A new key is made
        /// unless --key-stdin gives one exported on another node.
        #[arg(long)]
        encrypt: bool,
        /// Read the password from standard input (first line).
        #[arg(long, conflicts_with = "token_stdin")]
        password_stdin: bool,
        /// Read a bearer token from standard input (first line).
        #[arg(long)]
        token_stdin: bool,
        /// Read the encryption key from standard input (the line after a
        /// password or token, if any).
        #[arg(long)]
        key_stdin: bool,
    },
    /// List the registered storages.
    List,
    /// Print the key of an encrypted storage, to add it on another node.
    ExportKey { name: String },
    /// Unregister a storage no Space is on and forget its credentials; its
    /// files are left in place.
    Remove { name: String },
}

/// `path` with a leading `~/` replaced by the home directory, as UTF-8.
fn path_text(path: PathBuf) -> Result<String, BoxError> {
    let path = match (path.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => path,
    };
    Ok(path
        .to_str()
        .ok_or("the folder path must be valid UTF-8")?
        .to_string())
}

#[derive(Subcommand)]
enum Command {
    /// Manage DuckDB extensions.
    Extensions {
        #[command(subcommand)]
        command: ExtensionsCommand,
    },
    /// Manage storages (a NAS) that Spaces can live on.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },
    /// Print this node's DID.
    Identity,
    /// Run SQL statements (separated by `;`) as the node owner.
    Sql {
        /// The statements; `-` reads them from standard input.
        sql: String,
        /// Space that bare table names resolve against.
        #[arg(long)]
        space: Option<String>,
        /// Print rows as tab-separated values without a header.
        #[arg(long)]
        raw: bool,
    },
    /// Serve this node's Spaces to peers over Arrow Flight (TLS).
    Serve {
        /// Address to listen on.
        #[arg(long, env = "OIKOME_LISTEN", default_value = "0.0.0.0:50051")]
        listen: SocketAddr,
        /// Serve plaintext gRPC instead of TLS; needs --allow-http.
        #[arg(long)]
        plaintext: bool,
        /// libp2p address used to find peers; port 0 picks a free one.
        #[arg(long, env = "OIKOME_P2P_LISTEN", default_value = "/ip4/0.0.0.0/tcp/0")]
        p2p_listen: String,
        /// Do not look for peers on the local network (mDNS).
        #[arg(long)]
        no_mdns: bool,
        /// libp2p address of a known peer to connect to (repeatable).
        #[arg(long)]
        dial: Vec<String>,
        /// UDP address for browsers over WebRTC (e.g. 0.0.0.0:4433); off
        /// when unset. `SHOW WEBRTC` prints the record browsers connect with.
        #[arg(long, env = "OIKOME_WEBRTC_LISTEN")]
        webrtc_listen: Option<SocketAddr>,
        /// IP browsers reach the WebRTC endpoint at (published in the access
        /// record); defaults to the listen IP.
        #[arg(long, env = "OIKOME_WEBRTC_ADVERTISE")]
        webrtc_advertise: Option<IpAddr>,
        /// Seconds between syncs of Spaces on a storage: copies pull the
        /// newest snapshot, writers push one a failed push left behind. 0
        /// turns it off.
        #[arg(long, env = "OIKOME_STORAGE_SYNC", default_value_t = 60)]
        storage_sync: u64,
    },
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Stops a background task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The DuckDB configuration of the node at `root`.
fn duckdb_config(root: &Path, extension_dir: Option<&Path>) -> DuckDbConfig {
    DuckDbConfig {
        extension_dir: Some(
            extension_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.join(".oikome").join("extensions")),
        ),
        ..Default::default()
    }
}

fn open_node(
    root: &Path,
    allow_http: bool,
    extension_dir: Option<&Path>,
) -> Result<(Arc<OikomeCatalog>, Arc<EngineRegistry>), BoxError> {
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let config = NodeConfig {
        allow_http,
        duckdb: duckdb_config(&root, extension_dir),
        ..Default::default()
    };
    let root = root
        .to_str()
        .ok_or("the data directory path must be valid UTF-8")?;
    let catalog = Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), root)?);
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), config)?);
    Ok((catalog, engines))
}

fn print_result(result: &QueryResult, raw: bool) -> Result<(), BoxError> {
    match result {
        QueryResult::Batches { batches, .. } if raw => {
            let options = FormatOptions::default().with_null("");
            for batch in batches {
                let formatters = batch
                    .columns()
                    .iter()
                    .map(|c| ArrayFormatter::try_new(c.as_ref(), &options))
                    .collect::<Result<Vec<_>, _>>()?;
                for row in 0..batch.num_rows() {
                    let line: Vec<String> = formatters
                        .iter()
                        .map(|f| f.value(row).to_string())
                        .collect();
                    println!("{}", line.join("\t"));
                }
            }
        }
        QueryResult::Batches { batches, .. } => println!("{}", pretty_format_batches(batches)?),
        QueryResult::RowsAffected(n) => println!("{n} row(s) affected"),
        QueryResult::Empty => println!("OK"),
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<(), BoxError> {
    match cli.command {
        Command::Extensions {
            command: ExtensionsCommand::Install,
        } => {
            std::fs::create_dir_all(&cli.root)?;
            let root = cli.root.canonicalize()?;
            let engine =
                DuckDbEngine::open_in_memory(duckdb_config(&root, cli.extension_dir.as_deref()))?;
            for ext in Extension::INSTALLABLE {
                let version = engine.install_extension(ext).await?;
                println!("{}\t{version}", ext.name());
            }
        }
        Command::Storage { command } => {
            let (catalog, engines) =
                open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            match command {
                StorageCommand::Add {
                    name,
                    webdav,
                    username,
                    folder,
                    drive,
                    encrypt,
                    password_stdin,
                    token_stdin,
                    key_stdin,
                } => {
                    let (kind, encrypted) = match (webdav, folder, drive) {
                        (Some(endpoint), _, _) => {
                            (StorageKind::Webdav { endpoint, username }, encrypt)
                        }
                        (None, Some(path), _) => (
                            StorageKind::Folder {
                                path: path_text(path)?,
                            },
                            encrypt,
                        ),
                        (None, None, Some(path)) => (
                            StorageKind::Drive {
                                path: path_text(path)?,
                            },
                            true,
                        ),
                        (None, None, None) => {
                            return Err("give --webdav, --folder or --drive".into())
                        }
                    };
                    let mut lines = std::io::stdin().lines();
                    let mut next = |what: &str| -> Result<Zeroizing<String>, BoxError> {
                        match lines.next() {
                            Some(line) => Ok(Zeroizing::new(line?)),
                            None => Err(format!("expected the {what} on standard input").into()),
                        }
                    };
                    let credentials = StorageCredentials {
                        password: password_stdin.then(|| next("password")).transpose()?,
                        token: token_stdin.then(|| next("token")).transpose()?,
                        key: key_stdin.then(|| next("key")).transpose()?,
                    };
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    session
                        .add_storage(&name, kind, encrypted, credentials)
                        .await?;
                    println!("OK");
                }
                StorageCommand::List => {
                    for def in catalog.list_storages() {
                        let (kind, address, user) = match &def.kind {
                            StorageKind::Webdav { endpoint, username } => {
                                ("webdav", endpoint.as_str(), username.as_deref())
                            }
                            StorageKind::Folder { path } => ("folder", path.as_str(), None),
                            StorageKind::Drive { path } => ("drive", path.as_str(), None),
                        };
                        let encrypted = if def.encrypted { "encrypted" } else { "" };
                        println!(
                            "{}\t{kind}\t{address}\t{}\t{encrypted}",
                            def.name,
                            user.unwrap_or("")
                        );
                    }
                }
                StorageCommand::ExportKey { name } => {
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    println!("{}", session.export_storage_key(&name)?.as_str());
                }
                StorageCommand::Remove { name } => {
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    session.remove_storage(&name)?;
                    println!("OK");
                }
            }
        }
        Command::Identity => {
            let (_, engines) = open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            println!("{}", engines.identity()?.did());
        }
        Command::Sql { sql, space, raw } => {
            let sql = if sql == "-" {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                sql
            };
            let (catalog, engines) =
                open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            let session = OikomeSession::new(
                catalog,
                engines,
                SessionConfig {
                    default_space: space,
                    principal: None,
                },
            );
            for statement in oikome_semantic::split_statements(&sql) {
                let result = session.sql(&statement).await?;
                print_result(&result, raw)?;
            }
        }
        Command::Serve {
            listen,
            plaintext,
            p2p_listen,
            no_mdns,
            dial,
            webrtc_listen,
            webrtc_advertise,
            storage_sync,
        } => {
            if plaintext && !cli.allow_http {
                return Err("--plaintext needs --allow-http".into());
            }
            let (catalog, engines) =
                open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            let catalog_dir = catalog.oikome_dir();
            let record_path = catalog.webrtc_record_path();
            let tls = if plaintext {
                None
            } else {
                Some(engines.identity()?.tls_identity()?)
            };
            let identity = engines.identity()?;
            // Spaces on a storage follow it while the node serves (§7.5).
            let _storage_sync = (storage_sync > 0).then(|| {
                let owner =
                    OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
                AbortOnDrop(spawn_storage_sync(
                    Arc::new(owner),
                    Duration::from_secs(storage_sync),
                ))
            });
            let service = OikomeFlightService::new(catalog.clone(), engines)?;
            let listener = TcpListener::bind(listen).await?;
            let scheme = if plaintext { "http" } else { "https" };
            tracing::info!(did = %service.did(), addr = %listener.local_addr()?, %scheme, "serving Arrow Flight");
            // Announce this endpoint to peers and record theirs (§9).
            let discovery = Discovery::start(
                &identity,
                catalog,
                DiscoveryConfig {
                    listen: p2p_listen.parse()?,
                    flight: EndpointResponse {
                        scheme: scheme.to_string(),
                        port: listener.local_addr()?.port(),
                    },
                    mdns: !no_mdns,
                },
            )
            .await?;
            for addr in dial {
                discovery.dial(addr.parse()?).await?;
            }
            let webrtc = match webrtc_listen {
                Some(addr) => {
                    let socket = UdpSocket::bind(addr).await?;
                    let local = socket.local_addr()?;
                    let ip = webrtc_advertise.unwrap_or(local.ip());
                    if ip.is_unspecified() {
                        return Err(
                            "--webrtc-listen on an unspecified address needs --webrtc-advertise <ip>"
                                .into(),
                        );
                    }
                    let dtls = DtlsIdentity::load_or_create(
                        &catalog_dir.join("identity").join(WEBRTC_DTLS_FILE),
                    )?;
                    let record = dtls.access_record(
                        &identity,
                        SocketAddr::new(ip, local.port()),
                        WEBRTC_RECORD_TTL,
                    )?;
                    std::fs::write(&record_path, &record)?;
                    tracing::info!(addr = %local, fingerprint = dtls.fingerprint(), "serving WebRTC (SHOW WEBRTC prints the access record)");
                    Some((socket, dtls))
                }
                None => {
                    // A record from an earlier run no longer points anywhere.
                    let _ = std::fs::remove_file(&record_path);
                    None
                }
            };
            let service = Arc::new(service);
            let (stop, stopped) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("shutting down");
                let _ = stop.send(true);
            });
            let until_stopped = || {
                let mut stopped = stopped.clone();
                async move {
                    let _ = stopped.wait_for(|s| *s).await;
                }
            };
            let flight = serve(listener, service.clone(), tls.as_ref(), until_stopped());
            match webrtc {
                Some((socket, dtls)) => {
                    let browsers = serve_webrtc(socket, &dtls, service, until_stopped());
                    let (flight, browsers) = tokio::join!(flight, browsers);
                    // Browsers can no longer connect with it.
                    let _ = std::fs::remove_file(&record_path);
                    flight?;
                    browsers?;
                }
                None => flight.await?,
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
