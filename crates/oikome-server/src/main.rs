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
//! oikome [--root DIR] identity restore  restore it from a recovery key
//! oikome [--root DIR] sql "<SQL>"       run statements as the node owner
//! oikome [--root DIR] serve             serve this node's Spaces to peers
//!                                       (--web: also the owner's web API)
//! oikome [--root DIR] web open          open the web app, logged in
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
use oikome_api::Api;
use oikome_catalog::registry::OIKOME_DIR;
use oikome_catalog::{OikomeCatalog, StorageKind};
use oikome_connect::csv::CsvSource;
use oikome_connect::SourceConfig;
use oikome_discovery::{Discovery, DiscoveryConfig, EndpointResponse};
use oikome_engine_duckdb::{DuckDbConfig, DuckDbEngine, Extension};
use oikome_node::{
    spawn_source_sync, spawn_storage_sync, AccessEntry, AccessLog, EngineRegistry, NodeConfig,
    OikomeSession, QueryResult, SessionConfig, StorageCredentials,
};
use oikome_server::web::{login_file, DEFAULT_WEB_LISTEN};
use oikome_server::{serve_webrtc, serve_with_peers, DtlsIdentity, OikomeFlightService, WebApp};
use tokio::net::{TcpListener, UdpSocket};
use zeroize::Zeroizing;

/// The node's DTLS certificate for WebRTC, next to its key.
const WEBRTC_DTLS_FILE: &str = "webrtc_dtls.json";
/// How often the node looks again at whether anything the owner allowed
/// needs it reached from outside.
const REACH_CHECK: Duration = Duration::from_secs(60);
/// Activity's method for a connection from outside turned away by the
/// peer-to-peer layer (not a contact).
const TURNED_AWAY: &str = "connect";
/// How long an access record printed by `SHOW WEBRTC` stays valid.
const WEBRTC_RECORD_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

#[derive(Parser)]
#[command(
    name = "oikome",
    version,
    about = "A decentralized, local-first federated query layer"
)]
struct Cli {
    /// Data directory of this node; by default an `Oikome` folder in your
    /// home folder.
    #[arg(long, env = "OIKOME_ROOT", default_value_os_t = default_root(), global = true)]
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
        #[arg(long, group = "place", required_unless_present_any = ["folder", "drive", "dropbox", "rustfs"])]
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
        /// A Dropbox app key; give its refresh token with --token-stdin.
        /// Always encrypted.
        #[arg(long, group = "place", requires = "token_stdin")]
        dropbox: Option<String>,
        /// A Full Dropbox app, so folders of the owner's other files can be
        /// viewed in place too.
        #[arg(long, requires = "dropbox")]
        read_all: bool,
        /// A RustFS object storage server, http(s)://host[:port]; give
        /// --bucket and --access-key-id, and the secret key with
        /// --password-stdin. Encrypted unless --no-encrypt.
        #[arg(long, group = "place", requires_all = ["bucket", "access_key_id"])]
        rustfs: Option<String>,
        /// The bucket, which must exist already.
        #[arg(long, requires = "rustfs")]
        bucket: Option<String>,
        /// The access key's id.
        #[arg(long, requires = "rustfs")]
        access_key_id: Option<String>,
        /// Encrypt files before they reach the storage. A new key is made
        /// unless --key-stdin gives one exported on another node.
        #[arg(long)]
        encrypt: bool,
        /// RustFS only: keep files as they are, readable by whoever runs
        /// the storage.
        #[arg(long, requires = "rustfs", conflicts_with = "encrypt")]
        no_encrypt: bool,
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
    /// Remember your own Google app for Google Drive: its client ID and
    /// then its secret, one per line on standard input. The apps then
    /// connect Google Drive without asking for them.
    GoogleApp,
    /// Remember your own Dropbox app: its App key on standard input.
    DropboxApp,
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
enum IdentityCommand {
    /// Restore this node's identity from its recovery key, read from
    /// standard input. Only a node that holds nothing yet can be restored;
    /// restart a running node afterwards.
    Restore,
}

#[derive(Subcommand)]
enum WebCommand {
    /// Open the running node's web app in the default browser, logged in
    /// with the one-time link `serve --web` keeps in the data directory.
    Open,
}

#[derive(Subcommand)]
enum SourceCommand {
    /// Add a source and pull it once. Its data goes to a transactional
    /// Space of the same name; a token is read from standard input and
    /// kept in the OS keychain (or a private file where there is none).
    Add {
        /// Also the name of the Space: a letter or `_`, then letters,
        /// digits or `_`.
        name: String,
        /// A CSV file with a header row.
        #[arg(long, group = "kind", required_unless_present_any = ["flomo_export", "flomo_mcp"])]
        csv: Option<PathBuf>,
        /// The CSV column(s) that identify a row, comma-separated.
        #[arg(long, requires = "csv", value_delimiter = ',')]
        key: Vec<String>,
        /// The table the CSV rows go to.
        #[arg(long, requires = "csv", default_value = "rows")]
        table: String,
        /// flomo's HTML export: flomo_backup.zip, or its HTML file.
        #[arg(long, group = "kind")]
        flomo_export: Option<PathBuf>,
        /// flomo through its MCP server (flomo MAX); needs --token-stdin
        /// or OIKOME_SOURCE_<NAME>_TOKEN.
        #[arg(long, group = "kind")]
        flomo_mcp: bool,
        /// The MCP server's address.
        #[arg(long, requires = "flomo_mcp", default_value = oikome_connect::flomo_mcp::DEFAULT_ENDPOINT)]
        endpoint: String,
        /// The MCP tool that lists memos, if not the one found by name.
        #[arg(long, requires = "flomo_mcp")]
        mcp_tool: Option<String>,
        /// Arguments for that tool, as a JSON object.
        #[arg(long, requires = "flomo_mcp")]
        mcp_arguments: Option<String>,
        /// Read the source's token from standard input.
        #[arg(long)]
        token_stdin: bool,
        /// Pull this often while the node serves, e.g. 30m, 1h, 90s.
        #[arg(long, value_parser = parse_every)]
        every: Option<Duration>,
    },
    /// Pull a source now.
    Pull { name: String },
    /// List the sources and their last pull.
    List,
    /// Remove a source and forget its token; its Space and data stay.
    Remove { name: String },
}

/// `90s`, `30m`, `2h` or plain seconds.
fn parse_every(text: &str) -> Result<Duration, String> {
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => text.split_at(i),
        None => (text, "s"),
    };
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("'{text}' is not a duration like 30m"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(format!("'{text}': use s, m or h")),
    };
    Ok(Duration::from_secs(secs))
}

#[derive(Subcommand)]
enum ComputeCommand {
    /// Offer one of Oikome's compute methods (`sql`) to peers that hold a
    /// `compute/<method>` grant (`ISSUE UCAN ON COMPUTE`).
    Offer { method: String },
    /// Stop offering a method; tasks still waiting are refused.
    Withdraw { method: String },
    /// Set how much of this node's compute a peer may use.
    Quota {
        /// The peer's DID.
        peer: String,
        /// Tasks it may have running at once.
        #[arg(long, default_value_t = oikome_node::compute::DEFAULT_CONCURRENT)]
        concurrent: u32,
        /// Tasks it may start per hour.
        #[arg(long, default_value_t = oikome_node::compute::DEFAULT_PER_HOUR)]
        per_hour: u32,
    },
    /// The methods this node offers and the quotas set.
    List,
    /// The methods a peer offers this node, with the grant's limits. The
    /// grant is read from standard input.
    Methods {
        /// The peer's DID.
        #[arg(long)]
        owner: String,
        /// Its address, if discovery has not seen it.
        #[arg(long)]
        peer: Option<String>,
    },
    /// For development: run a task on a peer over the result of a local
    /// query. The grant is read from standard input.
    Run {
        /// The peer's DID.
        #[arg(long)]
        owner: String,
        /// Its address, if discovery has not seen it.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long, default_value = oikome_node::compute::METHOD_SQL)]
        method: String,
        /// The local query whose result is the task's input.
        #[arg(long)]
        input: String,
        /// The task: one query over the table `input`.
        #[arg(long)]
        sql: String,
        /// Print tab-separated rows without a header.
        #[arg(long)]
        raw: bool,
    },
}

/// A grant read from standard input, kept out of the process list.
fn token_from_stdin() -> Result<Zeroizing<String>, BoxError> {
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().read_line(&mut line)?;
    let token = Zeroizing::new(line.trim().to_string());
    if token.is_empty() {
        return Err("give the grant (a UCAN) on standard input".into());
    }
    Ok(token)
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
    /// Manage sources: apps whose data is brought into a Space (CSV files,
    /// flomo).
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    /// Share this node's compute with peers, or use theirs.
    Compute {
        #[command(subcommand)]
        command: ComputeCommand,
    },
    /// Print this node's DID.
    Identity {
        #[command(subcommand)]
        command: Option<IdentityCommand>,
    },
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
        /// Never ask the router to open a port, even when the owner allowed
        /// this node to be reached from outside (contacts can still help).
        #[arg(long, env = "OIKOME_NO_PORT_MAPPING")]
        no_port_mapping: bool,
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
        /// Seconds between checks for sources due to be pulled (each has its
        /// own interval). 0 turns scheduled pulls off.
        #[arg(long, env = "OIKOME_SOURCE_SYNC", default_value_t = 60)]
        source_sync: u64,
        /// Also serve the owner's web API on --web-listen (loopback only);
        /// `oikome web open` logs in.
        #[arg(long, env = "OIKOME_WEB")]
        web: bool,
        /// Loopback address of the web API; port 0 picks a free one.
        #[arg(long, env = "OIKOME_WEB_LISTEN", default_value = DEFAULT_WEB_LISTEN)]
        web_listen: SocketAddr,
        /// The web app's files (the `apps/web` build, `apps/web/dist`);
        /// without it the web API serves no pages.
        #[arg(long, env = "OIKOME_WEB_ROOT", requires = "web")]
        web_root: Option<PathBuf>,
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
/// Where a node keeps its data unless told otherwise: an `Oikome` folder in
/// the user's home (`~/Oikome`; `C:\Users\<name>\Oikome` on Windows).
/// It is the owner's, in plain sight, like a notes app's vault, and not in
/// Documents, which iCloud and OneDrive often sync: syncing a live database
/// breaks it. Cloud drives are added as encrypted places instead. Without a
/// home folder, `./oikome-data`.
fn default_root() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty());
    match home {
        Some(home) => PathBuf::from(home).join("Oikome"),
        None => PathBuf::from("oikome-data"),
    }
}

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
                    dropbox,
                    read_all,
                    rustfs,
                    bucket,
                    access_key_id,
                    encrypt,
                    no_encrypt,
                    password_stdin,
                    token_stdin,
                    key_stdin,
                } => {
                    let (kind, encrypted) = match (webdav, folder, drive) {
                        _ if dropbox.is_some() => (
                            StorageKind::Dropbox {
                                client_id: dropbox.unwrap_or_default(),
                                reads_all: read_all,
                            },
                            true,
                        ),
                        _ if rustfs.is_some() => (
                            StorageKind::S3 {
                                endpoint: rustfs.unwrap_or_default(),
                                bucket: bucket.unwrap_or_default(),
                                access_key_id: access_key_id.unwrap_or_default(),
                            },
                            !no_encrypt,
                        ),
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
                            return Err(
                                "give --webdav, --folder, --drive, --dropbox or --rustfs".into()
                            )
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
                        // Name, kind, address, then the user name: the
                        // access key on RustFS, the Google app's client ID
                        // on Google Drive.
                        let (kind, address, user) = match &def.kind {
                            StorageKind::Webdav { endpoint, username } => (
                                "webdav",
                                endpoint.clone(),
                                username.clone().unwrap_or_default(),
                            ),
                            StorageKind::Folder { path } => ("folder", path.clone(), String::new()),
                            StorageKind::Drive { path } => ("drive", path.clone(), String::new()),
                            StorageKind::Gdrive { client_id, .. } => (
                                "gdrive",
                                format!("Google Drive/{}", oikome_catalog::OBJECT_STORAGE_FOLDER),
                                client_id.clone(),
                            ),
                            StorageKind::Dropbox { client_id, .. } => (
                                "dropbox",
                                format!("Dropbox/{}", oikome_catalog::OBJECT_STORAGE_FOLDER),
                                client_id.clone(),
                            ),
                            StorageKind::S3 {
                                endpoint,
                                bucket,
                                access_key_id,
                                ..
                            } => (
                                "rustfs",
                                format!("{endpoint}/{bucket}"),
                                access_key_id.clone(),
                            ),
                        };
                        let encrypted = if def.encrypted { "encrypted" } else { "" };
                        println!("{}\t{kind}\t{address}\t{user}\t{encrypted}", def.name);
                    }
                }
                StorageCommand::ExportKey { name } => {
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    println!("{}", session.export_storage_key(&name)?.as_str());
                }
                StorageCommand::GoogleApp => {
                    // Read from standard input, kept out of the process list.
                    let mut id = String::new();
                    std::io::stdin().read_line(&mut id)?;
                    let mut secret = Zeroizing::new(String::new());
                    std::io::stdin().read_line(&mut secret)?;
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    session.set_google_app(id.trim(), secret.trim())?;
                    println!("Remembered the Google app {}.", id.trim());
                }
                StorageCommand::DropboxApp => {
                    let mut id = String::new();
                    std::io::stdin().read_line(&mut id)?;
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    session.set_dropbox_app(id.trim())?;
                    println!("Remembered the Dropbox app {}.", id.trim());
                }
                StorageCommand::Remove { name } => {
                    let session = OikomeSession::new(catalog, engines, SessionConfig::default());
                    session.remove_storage(&name)?;
                    println!("OK");
                }
            }
        }
        Command::Source { command } => {
            let (catalog, engines) =
                open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            let session = OikomeSession::new(catalog.clone(), engines, SessionConfig::default());
            match command {
                SourceCommand::Add {
                    name,
                    csv,
                    key,
                    table,
                    flomo_export,
                    flomo_mcp,
                    endpoint,
                    mcp_tool,
                    mcp_arguments,
                    token_stdin,
                    every,
                } => {
                    let config = match (csv, flomo_export, flomo_mcp) {
                        (Some(path), _, _) => SourceConfig::Csv {
                            table: CsvSource::inspect(&path, &table, &key)?,
                            path: path_text(path)?,
                        },
                        (None, Some(path), _) => SourceConfig::FlomoExport {
                            path: path_text(path)?,
                        },
                        (None, None, true) => SourceConfig::FlomoMcp {
                            endpoint,
                            tool: mcp_tool,
                            arguments: mcp_arguments
                                .map(|a| serde_json::from_str(&a))
                                .transpose()
                                .map_err(|e| format!("--mcp-arguments: {e}"))?,
                        },
                        _ => return Err("give --csv, --flomo-export or --flomo-mcp".into()),
                    };
                    let token = if token_stdin {
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line)?;
                        Some(Zeroizing::new(line.trim_end().to_string()))
                    } else {
                        None
                    };
                    let report = session.add_source(&name, config, every, token).await?;
                    print_result(&report.into_result()?, false)?;
                }
                SourceCommand::Pull { name } => {
                    let report = session.pull_source(&name).await?;
                    print_result(&report.into_result()?, false)?;
                }
                SourceCommand::List => {
                    for def in catalog.list_sources() {
                        let kind = def
                            .config
                            .get("kind")
                            .and_then(|k| k.as_str())
                            .unwrap_or("");
                        let every = def
                            .every_secs
                            .map(|s| format!("every {s}s"))
                            .unwrap_or_else(|| "on demand".to_string());
                        let last = match session.last_pull(&def.name).await {
                            Ok(Some(p)) => format!(
                                "pulled at {} ms (+{} -{})",
                                p.pulled_at_ms, p.upserted, p.deleted
                            ),
                            _ => "never pulled".to_string(),
                        };
                        println!("{}\t{kind}\t{every}\t{last}", def.name);
                    }
                }
                SourceCommand::Remove { name } => {
                    session.remove_source(&name)?;
                    println!("OK");
                }
            }
        }
        Command::Compute { command } => {
            let (catalog, engines) =
                open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            let session = OikomeSession::new(catalog.clone(), engines, SessionConfig::default());
            match command {
                ComputeCommand::Offer { method } => {
                    session.offer_compute(&method)?;
                    println!("OK");
                }
                ComputeCommand::Withdraw { method } => {
                    if !session.withdraw_compute(&method)? {
                        return Err(format!("'{method}' is not offered").into());
                    }
                    println!("OK");
                }
                ComputeCommand::Quota {
                    peer,
                    concurrent,
                    per_hour,
                } => {
                    session.set_compute_quota(&peer, concurrent, per_hour)?;
                    println!("OK");
                }
                ComputeCommand::List => {
                    for method in catalog.offered_compute() {
                        println!("offered\t{method}");
                    }
                    for quota in catalog.compute_quotas() {
                        println!(
                            "quota\t{}\t{} at once\t{} per hour",
                            quota.peer, quota.concurrent, quota.per_hour
                        );
                    }
                }
                ComputeCommand::Methods { owner, peer } => {
                    let token = token_from_stdin()?;
                    for m in session
                        .compute_methods_on_peer(peer.as_deref(), &owner, &token)
                        .await?
                    {
                        println!(
                            "{}\t{} ms\tinput {} B\toutput {} B\tmemory {} B",
                            m.method, m.time_ms, m.input_bytes, m.output_bytes, m.memory_bytes
                        );
                    }
                }
                ComputeCommand::Run {
                    owner,
                    peer,
                    method,
                    input,
                    sql,
                    raw,
                } => {
                    let token = token_from_stdin()?;
                    let result = session
                        .compute_on_peer(peer.as_deref(), &owner, &token, &method, &input, &sql)
                        .await?;
                    print_result(&result, raw)?;
                }
            }
        }
        Command::Identity { command: None } => {
            let (_, engines) = open_node(&cli.root, cli.allow_http, cli.extension_dir.as_deref())?;
            println!("{}", engines.identity()?.did());
        }
        Command::Identity {
            command: Some(IdentityCommand::Restore),
        } => {
            std::fs::create_dir_all(&cli.root)?;
            let root = cli.root.canonicalize()?;
            let root = root
                .to_str()
                .ok_or("the data directory path must be valid UTF-8")?;
            let catalog = OikomeCatalog::open(FileIO::new_with_fs(), root)?;
            eprintln!("Type or paste the recovery key, then press Enter:");
            let mut key = String::new();
            std::io::stdin().read_line(&mut key)?;
            let did = oikome_api::restore_identity(&catalog, &key);
            // The key is the identity: do not keep it around.
            key.replace_range(.., &"\0".repeat(key.len()));
            println!("Restored {}. If this node is running, restart it.", did?);
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
            no_port_mapping,
            webrtc_listen,
            webrtc_advertise,
            storage_sync,
            source_sync,
            web,
            web_listen,
            web_root,
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
            // Spaces on a storage follow it while the node serves (§7.5).
            let _storage_sync = (storage_sync > 0).then(|| {
                let owner =
                    OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
                AbortOnDrop(spawn_storage_sync(
                    Arc::new(owner),
                    Duration::from_secs(storage_sync),
                ))
            });
            // Folders of photos and notes that sources copied are viewed in
            // place from now on (§7.7), once.
            if let Err(e) = Api::new(catalog.clone(), engines.clone())
                .view_folders_in_place()
                .await
            {
                tracing::warn!(error = %e, "viewing folder sources in place");
            }
            // Sources are pulled on their schedule while the node serves (§11).
            let _source_sync = (source_sync > 0).then(|| {
                let owner =
                    OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
                AbortOnDrop(spawn_source_sync(
                    Arc::new(owner),
                    Duration::from_secs(source_sync),
                ))
            });
            let access_log = AccessLog::open(&catalog_dir)?;
            let service = OikomeFlightService::new(catalog.clone(), engines.clone())?
                .with_access_log(access_log.clone());
            let listener = TcpListener::bind(listen).await?;
            let scheme = if plaintext { "http" } else { "https" };
            tracing::info!(did = %service.did(), addr = %listener.local_addr()?, %scheme, "serving Arrow Flight");
            // Announce this endpoint to peers and record theirs; reach peers
            // on other networks (§9, 3.12).
            let mut p2p = DiscoveryConfig::new(
                p2p_listen.parse()?,
                EndpointResponse {
                    scheme: scheme.to_string(),
                    port: listener.local_addr()?.port(),
                },
            );
            p2p.mdns = !no_mdns;
            p2p.port_mapping = !no_port_mapping;
            let discovery = Discovery::start(&identity, catalog.clone(), p2p).await?;
            for addr in dial {
                discovery.dial(addr.parse()?).await?;
            }
            engines.set_dialer(discovery.dialer());
            // Someone who isn't a contact, turned away from outside, is in
            // Activity too: the prompt that opened the port says every
            // attempt shows there. Merged per minute, as unknown callers are.
            let refused = access_log.clone();
            discovery.on_turned_away(Arc::new(move |_, _| {
                refused.record(AccessEntry {
                    at_ms: oikome_identity::unix_now_ms(),
                    caller: None,
                    method: TURNED_AWAY.to_string(),
                    spaces: Vec::new(),
                    token_id: None,
                    outcome: oikome_node::Outcome::Denied,
                    reason: Some("not a contact, connecting from outside this network".to_string()),
                    detail: None,
                    merge: true,
                });
            }));
            let network = discovery.network();
            // What the owner allowed decides whether the router is asked;
            // looked at again now and then (a share runs out on its own).
            let reach_api =
                Api::new(catalog.clone(), engines.clone()).with_network(network.clone());
            let _reach = AbortOnDrop(tokio::spawn(async move {
                let mut every = tokio::time::interval(REACH_CHECK);
                loop {
                    every.tick().await;
                    reach_api.update_reach();
                }
            }));
            let web = if web {
                let web_listener = TcpListener::bind(web_listen).await?;
                let addr = web_listener.local_addr()?;
                let api = Api::new(catalog.clone(), engines.clone())
                    .with_access_log(access_log)
                    .with_flight_url(format!("{scheme}://{}", listener.local_addr()?))
                    .with_network(network);
                let mut app = WebApp::new(api, addr, login_file(&catalog_dir))?;
                if let Some(root) = &web_root {
                    app = app.with_web_root(root)?;
                }
                tracing::info!(%addr, "serving the web API (`oikome web open` logs in)");
                Some((app, web_listener))
            } else {
                None
            };
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
            let flight = serve_with_peers(
                listener,
                discovery.take_flight_streams(),
                service.clone(),
                tls.as_ref(),
                until_stopped(),
            );
            // Other processes of the node (`oikome sql`) reach peers on
            // other networks through this one.
            let socket = async {
                #[cfg(unix)]
                {
                    let path = oikome_network::route::peer_socket_path(&catalog_dir);
                    if let Err(e) = oikome_network::route::serve_peer_socket(
                        &path,
                        discovery.dialer(),
                        until_stopped(),
                    )
                    .await
                    {
                        tracing::warn!(error = %e, path = %path.display(), "other processes can't reach peers through this one");
                    }
                }
            };
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
            let (flight, browsers, owner, ()) = tokio::join!(flight, browsers, owner, socket);
            // The router port closes with the node.
            discovery.shutdown().await;
            flight?;
            browsers?;
            owner?;
        }
    }
    Ok(())
}

/// The stack of each of the runtime's threads. Queries run on them: a
/// federated plan (DataFusion over Flight, §5) nests its streams deeply, and
/// tokio's default of 2 MiB was found too little for a contact's query in a
/// debug build.
const THREAD_STACK_BYTES: usize = 8 << 20;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(THREAD_STACK_BYTES)
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: starting the runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = runtime.block_on(run(Cli::parse())) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
