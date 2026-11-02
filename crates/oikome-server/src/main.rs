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
//!                                       (--web: also the owner's web API)
//! oikome [--root DIR] web open          open the web app, logged in
//! oikome [--root DIR] extensions install
//!                                       fetch the DuckDB extensions Oikome uses
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

use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_cast::pretty::pretty_format_batches;
use clap::{Parser, Subcommand};
use iceberg::io::FileIO;
use oikome_api::Api;
use oikome_catalog::registry::OIKOME_DIR;
use oikome_catalog::OikomeCatalog;
use oikome_discovery::{Discovery, DiscoveryConfig, EndpointResponse};
use oikome_engine_duckdb::{DuckDbConfig, DuckDbEngine, Extension};
use oikome_node::{
    AccessLog, EngineRegistry, NodeConfig, OikomeSession, QueryResult, SessionConfig,
};
use oikome_server::web::{login_file, DEFAULT_WEB_LISTEN};
use oikome_server::{serve, serve_webrtc, DtlsIdentity, OikomeFlightService, WebApp};
use tokio::net::{TcpListener, UdpSocket};

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
enum WebCommand {
    /// Open the running node's web app in the default browser, logged in
    /// with the one-time link `serve --web` keeps in the data directory.
    Open,
}

#[derive(Subcommand)]
enum Command {
    /// Manage DuckDB extensions.
    Extensions {
        #[command(subcommand)]
        command: ExtensionsCommand,
    },
    /// Print this node's DID.
    Identity,
    /// The owner's web app.
    Web {
        #[command(subcommand)]
        command: WebCommand,
    },
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
        /// Also serve the owner's web API on --web-listen (loopback only);
        /// `oikome web open` logs in.
        #[arg(long, env = "OIKOME_WEB")]
        web: bool,
        /// Loopback address of the web API; port 0 picks a free one.
        #[arg(long, env = "OIKOME_WEB_LISTEN", default_value = DEFAULT_WEB_LISTEN)]
        web_listen: SocketAddr,
    },
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

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

/// Open `url` in the default browser. The URL is one argument of the
/// opener; no shell is involved.
fn open_browser(url: &str) -> Result<(), BoxError> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(opener).arg(url).status()?;
    // explorer exits with 1 even when it opened the URL.
    if status.success() || cfg!(windows) {
        Ok(())
    } else {
        Err(format!("{opener} could not open the browser").into())
    }
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
        Command::Identity => {
            let (_, engines) = open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            println!("{}", engines.identity()?.did());
        }
        Command::Web {
            command: WebCommand::Open,
        } => {
            let path = login_file(&cli.root.join(OIKOME_DIR));
            let link = std::fs::read_to_string(&path)
                .map_err(|_| "no login link: start the node with `oikome serve --web` first")?;
            let link = link.trim();
            if !link.starts_with("http://127.0.0.1:") || link.contains(char::is_whitespace) {
                return Err(format!("{} does not hold a login link", path.display()).into());
            }
            open_browser(link)?;
            println!("Opened the web app in your browser.");
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
            web,
            web_listen,
        } => {
            if plaintext && !cli.allow_http {
                return Err("--plaintext needs --allow-http".into());
            }
            if web && !web_listen.ip().is_loopback() {
                return Err("--web-listen must be a loopback address (127.0.0.1 or ::1)".into());
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
            let access_log = AccessLog::open(&catalog_dir)?;
            let service = OikomeFlightService::new(catalog.clone(), engines.clone())?
                .with_access_log(access_log.clone());
            let listener = TcpListener::bind(listen).await?;
            let scheme = if plaintext { "http" } else { "https" };
            tracing::info!(did = %service.did(), addr = %listener.local_addr()?, %scheme, "serving Arrow Flight");
            let web = if web {
                let web_listener = TcpListener::bind(web_listen).await?;
                let addr = web_listener.local_addr()?;
                let api = Api::new(catalog.clone(), engines.clone())
                    .with_access_log(access_log)
                    .with_flight_url(format!("{scheme}://{}", listener.local_addr()?));
                let app = WebApp::new(api, addr, login_file(&catalog_dir))?;
                tracing::info!(%addr, "serving the web API (`oikome web open` logs in)");
                Some((app, web_listener))
            } else {
                None
            };
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
            let browsers = async {
                let Some((socket, dtls)) = webrtc else {
                    return Ok::<(), BoxError>(());
                };
                let result = serve_webrtc(socket, &dtls, service.clone(), until_stopped()).await;
                // Browsers can no longer connect with it.
                let _ = std::fs::remove_file(&record_path);
                Ok(result?)
            };
            let owner = async {
                match web {
                    Some((app, web_listener)) => {
                        Ok::<(), BoxError>(app.serve(web_listener, until_stopped()).await?)
                    }
                    None => Ok(()),
                }
            };
            let (flight, browsers, owner) = tokio::join!(flight, browsers, owner);
            flight?;
            browsers?;
            owner?;
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
