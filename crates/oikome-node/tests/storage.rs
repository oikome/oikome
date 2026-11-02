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

//! Spaces on a storage (architecture v3 §7.5): one node writes a Space to a
//! NAS, others copy its newest complete snapshot.
//!
//! The NAS is a WebDAV server in this process (`dav-server`, with Basic
//! authentication and a password made up per test) or a plain folder.
//! The test marked `#[ignore]` needs the `lance` extension
//! (`cargo test -p oikome-node --test storage -- --ignored`).

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arrow_array::{Array, Int64Array};
use base64::Engine;
use dav_server::fakels::FakeLs;
use dav_server::localfs::LocalFs;
use dav_server::DavHandler;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use iceberg::io::FileIO;
use oikome_catalog::{OikomeCatalog, StorageKind};
use oikome_engine_duckdb::DuckDbConfig;
use oikome_identity::NodeIdentity;
use oikome_node::{
    spawn_storage_sync, EngineRegistry, FileSecrets, MemorySecrets, NodeConfig, OikomeSession,
    QueryResult, SecretBackend, SessionConfig, SessionError, StorageCredentials, StorageSecrets,
};
use zeroize::Zeroizing;

const USER: &str = "oikome";
const PASSWORD_VAR: &str = "OIKOME_STORAGE_NAS_PASSWORD";

/// A WebDAV server over a temporary directory, as a NAS would be.
struct Nas {
    dir: tempfile::TempDir,
    endpoint: String,
    password: String,
    /// While set, every request fails with 503.
    down: Arc<AtomicBool>,
}

impl Nas {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // The address names a folder the NAS already has.
        std::fs::create_dir(dir.path().join("nas")).unwrap();
        let handler = DavHandler::builder()
            .filesystem(LocalFs::new(dir.path(), false, false, false))
            .locksystem(FakeLs::new())
            .build_handler();
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        let password = blake3::hash(seed.as_bytes()).to_hex()[..24].to_string();
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{password}"))
        );
        let down = Arc::new(AtomicBool::new(false));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/nas", listener.local_addr().unwrap());
        let is_down = down.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (handler, expected, is_down) =
                    (handler.clone(), expected.clone(), is_down.clone());
                tokio::spawn(async move {
                    let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let handler = handler.clone();
                        let status = if is_down.load(Ordering::Relaxed) {
                            Some(hyper::StatusCode::SERVICE_UNAVAILABLE)
                        } else if req
                            .headers()
                            .get(hyper::header::AUTHORIZATION)
                            .is_none_or(|v| v.as_bytes() != expected.as_bytes())
                        {
                            Some(hyper::StatusCode::UNAUTHORIZED)
                        } else {
                            None
                        };
                        async move {
                            Ok::<_, Infallible>(match status {
                                Some(status) => {
                                    let mut response = hyper::Response::new(
                                        dav_server::body::Body::from(String::new()),
                                    );
                                    *response.status_mut() = status;
                                    response
                                }
                                None => handler.handle(req).await,
                            })
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            dir,
            endpoint,
            password,
            down,
        }
    }

    fn kind(&self) -> StorageKind {
        StorageKind::Webdav {
            endpoint: self.endpoint.clone(),
            username: Some(USER.to_string()),
        }
    }

    fn secrets(&self) -> StorageSecrets {
        let password = self.password.clone();
        StorageSecrets::new(move |name| (name == PASSWORD_VAR).then(|| password.clone()))
    }

    /// The Space folder `path` as stored on the server's disk.
    fn folder(&self, path: &str) -> PathBuf {
        self.dir.path().join("nas").join(path)
    }
}

struct Node {
    dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    owner: OikomeSession,
}

fn node_with(seed: u8, secrets: StorageSecrets, allow_http: bool, duckdb: DuckDbConfig) -> Node {
    node_full(seed, secrets, allow_http, duckdb, None)
}

/// A node; `backend` (default: in memory) keeps its credentials, so tests
/// never touch the machine's keychain.
fn node_full(
    seed: u8,
    secrets: StorageSecrets,
    allow_http: bool,
    duckdb: DuckDbConfig,
    backend: Option<Arc<dyn SecretBackend>>,
) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let config = NodeConfig {
        duckdb,
        identity: Some(Arc::new(NodeIdentity::from_seed(&[seed; 32]))),
        allow_http,
        storage_secrets: secrets,
        secret_backend: Some(backend.unwrap_or_else(|| Arc::new(MemorySecrets::default()))),
        ..Default::default()
    };
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), config).unwrap());
    let owner = OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
    Node {
        dir,
        catalog,
        engines,
        owner,
    }
}

fn node(seed: u8, secrets: StorageSecrets) -> Node {
    let offline = tempfile::tempdir().unwrap().keep().join("extensions");
    node_with(
        seed,
        secrets,
        true,
        DuckDbConfig {
            extension_dir: Some(offline),
            ..Default::default()
        },
    )
}

impl Node {
    async fn sql(&self, sql: &str) -> QueryResult {
        self.owner
            .sql(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn err(&self, sql: &str) -> SessionError {
        match self.owner.sql(sql).await {
            Ok(_) => panic!("{sql}: expected an error"),
            Err(e) => e,
        }
    }

    async fn count(&self, table: &str) -> i64 {
        ints(&self.sql(&format!("SELECT count(*) FROM {table}")).await)[0]
    }
}

fn ints(result: &QueryResult) -> Vec<i64> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            let a = b.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            (0..a.len()).map(|i| a.value(i)).collect::<Vec<_>>()
        })
        .collect()
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// The files a stored snapshot names, as paths in the Space's folder.
fn snapshot_files(folder: &Path, seq: u64) -> Vec<String> {
    let bytes = std::fs::read(folder.join(format!("snapshots/{seq:020}.json"))).unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    snapshot["tables"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["files"].as_array().unwrap().clone())
        .map(|f| format!("files/{}.parquet", f["hash"].as_str().unwrap()))
        .collect()
}

fn snapshot_seqs(folder: &Path) -> Vec<u64> {
    let mut seqs: Vec<u64> = std::fs::read_dir(folder.join("snapshots"))
        .unwrap()
        .filter_map(|e| {
            e.unwrap()
                .file_name()
                .to_str()?
                .strip_suffix(".json")?
                .parse()
                .ok()
        })
        .collect();
    seqs.sort_unstable();
    seqs
}

#[tokio::test]
async fn a_space_on_a_nas_is_written_by_one_node_and_read_by_another() {
    let nas = Nas::start().await;
    let a = node(1, nas.secrets());
    let b = node(2, nas.secrets());
    for n in [&a, &b] {
        n.catalog.add_storage("nas", nas.kind(), false).unwrap();
        n.owner.check_storage("nas").await.unwrap();
    }
    for sql in [
        "CREATE SPACE photos WITH STORAGE = 'nas'",
        "CREATE TABLE photos.items (id BIGINT, name VARCHAR)",
        "INSERT INTO photos.items VALUES (1, 'a'), (2, 'b')",
    ] {
        a.sql(sql).await;
    }
    b.sql("CREATE SPACE photos WITH STORAGE = 'nas', ACCESS = 'readonly'")
        .await;
    assert_eq!(b.count("photos.items").await, 2);

    // A later write reaches B on its next refresh, without A.
    a.sql("INSERT INTO photos.items VALUES (3, 'c')").await;
    b.sql("REFRESH SPACE photos").await;
    assert_eq!(b.count("photos.items").await, 3);
    // The NAS keeps the last two snapshots and the files they name.
    let folder = nas.folder("photos");
    assert_eq!(snapshot_seqs(&folder).len(), 2);
    let named: std::collections::BTreeSet<String> = snapshot_seqs(&folder)
        .into_iter()
        .flat_map(|seq| snapshot_files(&folder, seq))
        .collect();
    let held: std::collections::BTreeSet<String> = files_under(&folder.join("files"))
        .iter()
        .map(|p| format!("files/{}", p.file_name().unwrap().to_str().unwrap()))
        .collect();
    assert_eq!(named, held);
    // Nothing new: the push uploads nothing.
    let unchanged = a.sql("REFRESH SPACE photos").await;
    assert!(format!("{unchanged:?}").contains("unchanged"));

    // B only reads; another node cannot write the folder; SQLite stays on
    // the node's disk.
    assert!(matches!(
        b.err("INSERT INTO photos.items VALUES (4, 'd')").await,
        SessionError::ReadOnlySpace(_)
    ));
    for (sql, needle) in [
        (
            "CREATE SPACE mine WITH STORAGE = 'nas', PATH = 'photos'",
            "is written by",
        ),
        (
            "CREATE SPACE ledger WITH KIND = 'transactional', STORAGE = 'nas'",
            "SQLite",
        ),
        (
            "CREATE SPACE x WITH STORAGE = 'cloud'",
            "oikome storage add",
        ),
        (
            "CREATE SPACE y WITH STORAGE = 'nas', PATH = '../photos'",
            "PATH",
        ),
    ] {
        let e = b.err(sql).await.to_string();
        assert!(e.contains(needle), "{sql}: {e}");
    }
    assert!(b.catalog.get_space("mine").is_err());
    assert!(b.catalog.remove_storage("nas").is_err(), "in use");

    // Dropping the copy deletes it on B only.
    b.sql("DROP SPACE photos").await;
    assert!(!b.catalog.replica_dir("photos").exists());
    assert!(folder.join("oikome-space.json").is_file());
    b.catalog.remove_storage("nas").unwrap();

    // The password is never written down.
    for n in [&a, &b] {
        for file in files_under(n.dir.path()) {
            let bytes = std::fs::read(&file).unwrap();
            assert!(
                !bytes
                    .windows(nas.password.len())
                    .any(|w| w == nas.password.as_bytes()),
                "{}",
                file.display()
            );
        }
    }
}

#[tokio::test]
async fn wrong_credentials_and_plaintext_are_refused() {
    let nas = Nas::start().await;
    let wrong = node(1, StorageSecrets::new(|_| Some("wrong".to_string())));
    wrong.catalog.add_storage("nas", nas.kind(), false).unwrap();
    let e = wrong.owner.check_storage("nas").await.unwrap_err();
    assert!(e.to_string().contains("refused the credentials"), "{e}");
    let e = wrong.err("CREATE SPACE photos WITH STORAGE = 'nas'").await;
    assert!(e.to_string().contains("refused the credentials"), "{e}");
    assert!(wrong.catalog.get_space("photos").is_err());

    let strict = node_with(2, nas.secrets(), false, DuckDbConfig::default());
    strict
        .catalog
        .add_storage("nas", nas.kind(), false)
        .unwrap();
    let e = strict.err("CREATE SPACE photos WITH STORAGE = 'nas'").await;
    assert!(e.to_string().contains("plaintext"), "{e}");
}

#[tokio::test]
async fn a_reader_skips_a_snapshot_whose_files_are_missing() {
    let shared = tempfile::tempdir().unwrap();
    let folder = StorageKind::Folder {
        path: shared.path().to_str().unwrap().to_string(),
    };
    let a = node(1, StorageSecrets::default());
    let b = node(2, StorageSecrets::default());
    for n in [&a, &b] {
        n.catalog.add_storage("nas", folder.clone(), false).unwrap();
    }
    for sql in [
        "CREATE SPACE photos WITH STORAGE = 'nas', PATH = 'family/photos'",
        "CREATE TABLE photos.items (id BIGINT)",
        "INSERT INTO photos.items VALUES (1)",
        "INSERT INTO photos.items VALUES (2)",
    ] {
        a.sql(sql).await;
    }
    // A file only the newest snapshot names goes missing, as with an upload
    // a sync client has not finished.
    let space = shared.path().join("family/photos");
    let seqs = snapshot_seqs(&space);
    let (older, newest) = (seqs[0], seqs[1]);
    let only_newest: Vec<String> = snapshot_files(&space, newest)
        .into_iter()
        .filter(|f| !snapshot_files(&space, older).contains(f))
        .collect();
    std::fs::remove_file(space.join(&only_newest[0])).unwrap();

    b.sql("CREATE SPACE photos WITH STORAGE = 'nas', PATH = 'family/photos', ACCESS = 'readonly'")
        .await;
    assert_eq!(b.count("photos.items").await, 1);

    // The writer uploads the file again; the reader catches up.
    a.sql("REFRESH SPACE photos").await;
    assert!(space.join(&only_newest[0]).is_file());
    b.sql("REFRESH SPACE photos").await;
    assert_eq!(b.count("photos.items").await, 2);
}

#[tokio::test]
async fn a_failed_push_is_made_up_by_the_next_write() {
    let nas = Nas::start().await;
    let a = node(1, nas.secrets());
    let b = node(2, nas.secrets());
    for n in [&a, &b] {
        n.catalog.add_storage("nas", nas.kind(), false).unwrap();
    }
    for sql in [
        "CREATE SPACE photos WITH STORAGE = 'nas'",
        "CREATE TABLE photos.items (id BIGINT)",
        "INSERT INTO photos.items VALUES (1)",
    ] {
        a.sql(sql).await;
    }
    // The NAS is unreachable: the write still commits on A.
    nas.down.store(true, Ordering::Relaxed);
    a.sql("INSERT INTO photos.items VALUES (2)").await;
    assert_eq!(a.count("photos.items").await, 2);
    nas.down.store(false, Ordering::Relaxed);

    b.sql("CREATE SPACE photos WITH STORAGE = 'nas', ACCESS = 'readonly'")
        .await;
    assert_eq!(b.count("photos.items").await, 1);
    a.sql("INSERT INTO photos.items VALUES (3)").await;
    b.sql("REFRESH SPACE photos").await;
    assert_eq!(b.count("photos.items").await, 3);
}

#[tokio::test]
async fn a_copy_follows_the_storage_while_the_node_serves() {
    let shared = tempfile::tempdir().unwrap();
    let folder = StorageKind::Folder {
        path: shared.path().to_str().unwrap().to_string(),
    };
    let a = node(1, StorageSecrets::default());
    let b = node(2, StorageSecrets::default());
    for n in [&a, &b] {
        n.catalog.add_storage("nas", folder.clone(), false).unwrap();
    }
    for sql in [
        "CREATE SPACE photos WITH STORAGE = 'nas'",
        "CREATE TABLE photos.items (id BIGINT)",
    ] {
        a.sql(sql).await;
    }
    b.sql("CREATE SPACE photos WITH STORAGE = 'nas', ACCESS = 'readonly'")
        .await;
    let serving = OikomeSession::new(
        b.catalog.clone(),
        b.engines.clone(),
        SessionConfig::default(),
    );
    let sync = spawn_storage_sync(Arc::new(serving), std::time::Duration::from_millis(100));
    a.sql("INSERT INTO photos.items VALUES (1), (2)").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while b.count("photos.items").await != 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the copy never caught up"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    sync.abort();
}

fn no_env() -> StorageSecrets {
    StorageSecrets::new(|_| None)
}

fn drive_at(dir: &Path) -> StorageKind {
    StorageKind::Drive {
        path: dir.to_str().unwrap().to_string(),
    }
}

/// Whether any file under `dir` contains `needle`.
fn any_file_contains(dir: &Path, needle: &[u8]) -> bool {
    files_under(dir).iter().any(|f| {
        std::fs::read(f)
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
    })
}

#[tokio::test]
async fn a_space_on_a_drive_is_encrypted_and_read_with_the_same_key() {
    let drive = tempfile::tempdir().unwrap();
    let a = node(1, no_env());
    a.owner
        .add_storage(
            "drive",
            drive_at(drive.path()),
            true,
            StorageCredentials::default(),
        )
        .await
        .unwrap();
    for sql in [
        "CREATE SPACE vault WITH STORAGE = 'drive'",
        "CREATE TABLE vault.lighthouse_log (id BIGINT, note VARCHAR)",
        "INSERT INTO vault.lighthouse_log VALUES (1, 'lantern'), (2, 'beacon')",
        "INSERT INTO vault.lighthouse_log VALUES (3, 'flare')",
    ] {
        a.sql(sql).await;
    }

    // Another node reads it with the key exported on the first.
    let key = a.owner.export_storage_key("drive").unwrap();
    let b = node(2, no_env());
    b.owner
        .add_storage(
            "drive",
            drive_at(drive.path()),
            true,
            StorageCredentials {
                key: Some(key),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    b.sql("CREATE SPACE vault WITH STORAGE = 'drive', ACCESS = 'readonly'")
        .await;
    assert_eq!(b.count("vault.lighthouse_log").await, 3);

    // The drive sees neither names nor content.
    let folder = drive.path().join("vault");
    let mut top: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    assert_eq!(top, vec!["objects", "oikome-space.sealed", "snapshots"]);
    for needle in [&b"lighthouse_log"[..], b"PAR1", b"parquet", b"iceberg"] {
        assert!(!any_file_contains(drive.path(), needle), "{needle:?}");
    }

    // Without the right key, or registered as unencrypted, nothing is read.
    let c = node(3, no_env());
    c.owner
        .add_storage(
            "drive",
            drive_at(drive.path()),
            true,
            StorageCredentials::default(),
        )
        .await
        .unwrap();
    let e = c
        .err("CREATE SPACE vault WITH STORAGE = 'drive', ACCESS = 'readonly'")
        .await
        .to_string();
    assert!(e.contains("another key"), "{e}");
    let d = node(4, no_env());
    d.catalog
        .add_storage(
            "drive",
            StorageKind::Folder {
                path: drive.path().to_str().unwrap().to_string(),
            },
            false,
        )
        .unwrap();
    let e = d
        .err("CREATE SPACE vault WITH STORAGE = 'drive', ACCESS = 'readonly'")
        .await
        .to_string();
    assert!(e.contains("is encrypted"), "{e}");

    // A drive is never left unencrypted.
    assert!(d
        .catalog
        .add_storage("plain", drive_at(drive.path()), false)
        .is_err());
}

#[tokio::test]
async fn credentials_given_to_the_node_stay_with_the_node() {
    let nas = Nas::start().await;
    let identity = tempfile::tempdir().unwrap();
    let backend: Arc<dyn SecretBackend> = Arc::new(FileSecrets::new(identity.path()));
    let n = node_full(
        1,
        no_env(),
        true,
        DuckDbConfig::default(),
        Some(backend.clone()),
    );
    let password = || StorageCredentials {
        password: Some(Zeroizing::new(nas.password.clone())),
        ..Default::default()
    };

    // A wrong password is refused and leaves nothing behind.
    let wrong = StorageCredentials {
        password: Some(Zeroizing::new("wrong".to_string())),
        ..Default::default()
    };
    let e = n
        .owner
        .add_storage("nas", nas.kind(), false, wrong)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("refused the credentials"), "{e}");
    assert!(n.catalog.get_storage("nas").is_err());
    assert!(backend.get("storage/nas/password").unwrap().is_none());

    n.owner
        .add_storage("nas", nas.kind(), false, password())
        .await
        .unwrap();
    n.sql("CREATE SPACE photos WITH STORAGE = 'nas'").await;
    // Only the secret store holds the password, readable by this user only.
    let store = identity.path().join("storage_access.json");
    assert!(any_file_contains(identity.path(), nas.password.as_bytes()));
    assert!(!any_file_contains(n.dir.path(), nas.password.as_bytes()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&store).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(n.owner.export_storage_key("nas").is_err(), "not encrypted");

    // The environment wins over the store.
    let overridden = node_full(
        2,
        StorageSecrets::new(|name| (name == "OIKOME_STORAGE_NAS_PASSWORD").then(|| "wrong".into())),
        true,
        DuckDbConfig::default(),
        Some(Arc::new(FileSecrets::new(identity.path()))),
    );
    overridden
        .catalog
        .add_storage("nas", nas.kind(), false)
        .unwrap();
    let e = overridden.owner.check_storage("nas").await.unwrap_err();
    assert!(e.to_string().contains("refused the credentials"), "{e}");

    // A storage with a Space on it stays; removing one forgets its
    // credentials.
    assert!(
        n.owner.remove_storage("nas").is_err(),
        "a location is on it"
    );
    n.owner
        .add_storage("spare", nas.kind(), false, password())
        .await
        .unwrap();
    assert!(backend.get("storage/spare/password").unwrap().is_some());
    n.owner.remove_storage("spare").unwrap();
    assert!(backend.get("storage/spare/password").unwrap().is_none());
}

#[tokio::test]
#[ignore = "downloads the DuckDB lance extension"]
async fn a_lance_space_on_a_nas_is_searched_with_the_writers_index() {
    let nas = Nas::start().await;
    let lance = || DuckDbConfig {
        extension_dir: Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckdb-extensions")),
        allow_extension_install: true,
        ..Default::default()
    };
    let a = node_with(1, nas.secrets(), true, lance());
    let b = node_with(2, nas.secrets(), true, lance());
    for n in [&a, &b] {
        n.catalog.add_storage("nas", nas.kind(), false).unwrap();
    }
    for sql in [
        "CREATE SPACE notes WITH STORAGE = 'nas', FORMAT = 'lance'",
        "CREATE TABLE notes.items (id BIGINT, body VARCHAR)",
        "INSERT INTO notes.items SELECT i, \
         CASE WHEN i % 3 = 0 THEN 'apple banana' ELSE 'cherry date' END || ' note ' || i \
         FROM (SELECT unnest(range(300)) AS i) t",
        "CREATE INDEX body ON notes.items USING INVERTED (body)",
    ] {
        a.sql(sql).await;
    }
    b.sql("CREATE SPACE notes WITH STORAGE = 'nas', ACCESS = 'readonly'")
        .await;
    assert_eq!(b.count("notes.items").await, 300);
    let found = b
        .sql("SELECT id FROM lance_fts(notes.items, 'body', 'banana', k => 5)")
        .await;
    assert_eq!(found.num_rows(), 5);
    assert!(ints(&found).iter().all(|id| id % 3 == 0));

    // B searches with A's index files: the same bytes, never rebuilt.
    let index_files = |dir: &Path| -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = files_under(&dir.join("items.lance/_indices"))
            .into_iter()
            .map(|p| {
                let name = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(p).unwrap())
            })
            .collect();
        files.sort();
        files
    };
    let a_root = a.dir.path().join(".oikome/lance/notes");
    let b_root = b.catalog.replica_dir("notes").join("lance");
    let ours = index_files(&a_root);
    assert!(!ours.is_empty());
    assert_eq!(ours, index_files(&b_root));
}
