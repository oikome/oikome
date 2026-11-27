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

//! Datasets in a folder viewed in place (PR 31): a Lance dataset or an
//! Iceberg table another program wrote is one item of the folder and a
//! table of the collection, read where it is, its latest version each time,
//! and refused when it refers to files outside its folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, Int64Array};
use iceberg::io::FileIO;
use oikome_catalog::OikomeCatalog;
use oikome_engine::TableSource;
use oikome_engine_duckdb::DuckDbConfig;
use oikome_node::{
    inplace, EngineRegistry, MemorySecrets, NodeConfig, OikomeSession, QueryResult, SessionConfig,
};

/// Extensions shared by the tests that need them, downloaded on first use.
fn extension_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckdb-extensions")
}

struct Node {
    dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    owner: OikomeSession,
}

fn node_at(dir: tempfile::TempDir) -> Node {
    node_installing(dir, true)
}

/// A node whose DuckDB may install extensions itself, or, as the app's,
/// only loads what was installed.
fn node_installing(dir: tempfile::TempDir, install: bool) -> Node {
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let config = NodeConfig {
        duckdb: DuckDbConfig {
            extension_dir: Some(extension_dir()),
            allow_extension_install: install,
            ..Default::default()
        },
        secret_backend: Some(Arc::new(MemorySecrets::default())),
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

fn node() -> Node {
    node_at(tempfile::Builder::new().prefix("node-").tempdir().unwrap())
}

/// A folder of the owner's: not hidden, as a temporary folder's name is.
fn owner_folder() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("owner-").tempdir().unwrap()
}

impl Node {
    async fn sql(&self, sql: &str) -> QueryResult {
        self.owner
            .sql(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count(&self, table: &str) -> i64 {
        let result = self
            .sql(&format!("SELECT CAST(count(*) AS BIGINT) FROM {table}"))
            .await;
        result.batches()[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0)
    }

    /// The first column of `sql`'s rows, as text.
    async fn texts(&self, sql: &str) -> Vec<String> {
        self.sql(sql)
            .await
            .batches()
            .iter()
            .flat_map(|b| {
                let column = arrow_cast::cast(b.column(0), &arrow_schema::DataType::Utf8).unwrap();
                let column = column
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .unwrap()
                    .clone();
                (0..column.len())
                    .map(|i| column.value(i).to_string())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// An Iceberg table and a Lance dataset written by "another program":
    /// this node's own, as Iceberg and Lance write them.
    async fn write_datasets(&self) {
        for sql in [
            "CREATE SPACE shop",
            "CREATE TABLE shop.orders (id BIGINT, item VARCHAR)",
            "INSERT INTO shop.orders VALUES (1, 'tea'), (2, 'rice')",
            "INSERT INTO shop.orders VALUES (3, 'salt')",
            "CREATE SPACE notes WITH FORMAT = 'lance'",
            "CREATE TABLE notes.items (id BIGINT, title VARCHAR, embedding FLOAT[2])",
            "INSERT INTO notes.items VALUES (1, 'a', [1, 0]), (2, 'b', [0, 1])",
        ] {
            self.sql(sql).await;
        }
    }

    fn orders(&self) -> PathBuf {
        self.root().join("shop").join("orders")
    }

    fn items(&self) -> PathBuf {
        self.root().join(".oikome/lance/notes/items.lance")
    }
}

/// Copy `from` into `to`, adding what is new (files already there stay).
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn newest_metadata(table: &Path) -> PathBuf {
    let mut names: Vec<PathBuf> = std::fs::read_dir(table.join("metadata"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".metadata.json"))
        .collect();
    names.sort();
    names.pop().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn datasets_in_a_folder_are_tables_read_where_they_are() {
    let n = node();
    n.write_datasets().await;
    // The owner's folder, outside the data folder: an Iceberg table moved
    // there, a Lance dataset under a plain name, and a CSV beside them.
    let folder = owner_folder();
    copy_dir(&n.orders(), &folder.path().join("exports/orders"));
    copy_dir(&n.items(), &folder.path().join("lab/items"));
    std::fs::write(folder.path().join("readings.csv"), "day,kwh\n1,3\n").unwrap();
    n.sql(&format!(
        "CREATE SPACE home WITH PLACE = 'local', PATH = '{}'",
        folder.path().display()
    ))
    .await;

    // One item each, of kind `dataset`, none of their files.
    let listed = n
        .sql("SELECT path, kind FROM home.files ORDER BY path")
        .await;
    let rows: Vec<(String, String)> = listed
        .batches()
        .iter()
        .flat_map(|b| {
            let paths = b
                .column(0)
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .unwrap();
            let kinds = b
                .column(1)
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .unwrap();
            (0..b.num_rows())
                .map(|i| (paths.value(i).to_string(), kinds.value(i).to_string()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("exports/orders".to_string(), "dataset".to_string()),
            ("lab/items".to_string(), "dataset".to_string()),
            ("readings.csv".to_string(), "table".to_string()),
        ]
    );
    assert_eq!(
        n.texts("SHOW FILES FROM home").await,
        ["exports", "lab", "readings.csv"]
    );
    assert_eq!(
        n.texts("SHOW FILES FROM home IN 'exports'").await,
        ["orders"]
    );
    assert_eq!(n.texts("SHOW FILES FROM home IN 'lab'").await, ["items"]);

    // Tables of the collection, read where they are, values and all.
    assert_eq!(n.count("home.orders").await, 3);
    assert_eq!(n.count("home.items").await, 2);
    assert_eq!(
        n.texts("SELECT item FROM home.orders ORDER BY id").await,
        ["tea", "rice", "salt"]
    );
    assert_eq!(
        n.texts("SELECT title FROM home.items ORDER BY id").await,
        ["a", "b"]
    );
    let location = n.owner.in_place_location("home").unwrap().unwrap();
    let tables = inplace::dataset_tables(&n.catalog, &location).unwrap();
    assert_eq!(
        tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["orders", "items"]
    );
    let read = inplace::read_dataset(&n.catalog, &n.engines, &location, "exports/orders")
        .await
        .unwrap();
    // Moved from where it was written: read through a rewritten metadata
    // chain in the cache, which names the folder's files.
    let TableSource::IcebergTable { metadata } = &read.source else {
        panic!("{:?}", read.source);
    };
    assert!(metadata.starts_with(n.root()), "{}", metadata.display());
    assert!(read.version.is_some() && read.written_at_ms.is_some());
    let chain = std::fs::read_to_string(metadata).unwrap();
    assert!(
        chain.contains(&folder.path().join("exports/orders").display().to_string()),
        "{chain}"
    );
    assert!(
        !chain.contains(&n.orders().display().to_string()),
        "{chain}"
    );
    let lance = inplace::read_dataset(&n.catalog, &n.engines, &location, "lab/items")
        .await
        .unwrap();
    assert_eq!(lance.version.as_deref(), Some("2"));
    let at = std::fs::canonicalize(folder.path().join("lab/items")).unwrap();
    assert!(
        matches!(&lance.source, TableSource::LanceDataset { path } if std::fs::canonicalize(path).unwrap() == at),
        "{:?}",
        lance.source
    );

    // New versions are followed, as a file's table follows its file.
    n.sql("INSERT INTO shop.orders VALUES (4, 'oil')").await;
    copy_dir(&n.orders(), &folder.path().join("exports/orders"));
    n.sql("INSERT INTO notes.items VALUES (3, 'c', [1, 1])")
        .await;
    copy_dir(&n.items(), &folder.path().join("lab/items"));
    assert_eq!(n.count("home.orders").await, 4);
    assert_eq!(n.count("home.items").await, 3);
    assert_eq!(
        n.texts("SELECT item FROM home.orders WHERE id = 4").await,
        ["oil"]
    );
    let rows = inplace::file_rows(&n.catalog, &n.engines, &location, "exports/orders", 1, 2)
        .await
        .unwrap();
    assert_eq!(rows.rows.iter().map(|b| b.num_rows()).sum::<usize>(), 2);

    // Nothing was written into the owner's folder.
    let mut found = Vec::new();
    for entry in walk(folder.path()) {
        found.push(entry);
    }
    assert!(
        found
            .iter()
            .all(|p| !p.to_string_lossy().contains("oikome")),
        "{found:?}"
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dataset_reaching_outside_its_folder_is_not_read() {
    let n = node();
    n.write_datasets().await;
    let folder = owner_folder();
    let table = folder.path().join("exports/orders");
    copy_dir(&n.orders(), &table);
    // Its newest version names a manifest list elsewhere on this computer.
    let newest = newest_metadata(&table);
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&newest).unwrap()).unwrap();
    for snapshot in json["snapshots"].as_array_mut().unwrap() {
        snapshot["manifest-list"] = serde_json::Value::String(
            n.orders()
                .join("../../.oikome/secret.avro")
                .display()
                .to_string(),
        );
    }
    std::fs::write(&newest, serde_json::to_vec(&json).unwrap()).unwrap();
    n.sql(&format!(
        "CREATE SPACE home WITH PLACE = 'local', PATH = '{}'",
        folder.path().display()
    ))
    .await;
    let location = n.owner.in_place_location("home").unwrap().unwrap();
    let err = inplace::read_dataset(&n.catalog, &n.engines, &location, "exports/orders")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("outside its folder"), "{err}");
    // Its table says why it can't be read; the collection's others are fine.
    let err = n
        .owner
        .sql("SELECT * FROM home.orders")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("outside its folder"), "{err}");
    n.sql("SELECT count(*) FROM home.files").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_iceberg_table_written_in_its_folder_is_read_as_it_is() {
    // Another program's warehouse inside the owner's folder.
    let folder = owner_folder();
    let warehouse = folder.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let writer = node_at(
        tempfile::Builder::new()
            .prefix("writer-")
            .tempdir_in(&warehouse)
            .unwrap(),
    );
    writer.write_datasets().await;
    // The extensions installed once; then read as the app reads, loading
    // only what is installed (`iceberg` and the `avro` it needs).
    writer
        .engines
        .duckdb()
        .unwrap()
        .enable(oikome_engine::Capabilities::READ_ICEBERG)
        .await
        .unwrap();
    let reader = node_installing(
        tempfile::Builder::new().prefix("node-").tempdir().unwrap(),
        false,
    );
    reader
        .sql(&format!(
            "CREATE SPACE home WITH PLACE = 'local', PATH = '{}'",
            folder.path().display()
        ))
        .await;
    let location = reader.owner.in_place_location("home").unwrap().unwrap();
    let tables = inplace::dataset_tables(&reader.catalog, &location).unwrap();
    let orders = tables.iter().find(|t| t.name == "orders").unwrap();
    let read = inplace::read_dataset(&reader.catalog, &reader.engines, &location, &orders.path)
        .await
        .unwrap();
    let TableSource::IcebergTable { metadata } = &read.source else {
        panic!("{:?}", read.source);
    };
    // Its own metadata file, every path in it checked to be its own.
    let own = std::fs::canonicalize(folder.path()).unwrap();
    assert!(
        std::fs::canonicalize(metadata).unwrap().starts_with(&own),
        "{}",
        metadata.display()
    );
    assert_eq!(reader.count("home.orders").await, 3);
    assert_eq!(
        reader
            .texts("SELECT item FROM home.orders ORDER BY id")
            .await,
        ["tea", "rice", "salt"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn datasets_on_object_storage_are_fetched_and_read() {
    let Some(rustfs) = crate::storage::Rustfs::start().await else {
        return;
    };
    rustfs.create_bucket("family").await;
    let n = node();
    n.write_datasets().await;
    let bucket = rustfs.bucket("family");
    for (from, to) in [
        (n.orders(), "data/exports/orders"),
        (n.items(), "data/lab/items"),
    ] {
        for file in walk(&from) {
            let rel = file
                .strip_prefix(&from)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            bucket
                .write(
                    &format!("{to}/{rel}"),
                    bytes::Bytes::from(std::fs::read(&file).unwrap()),
                )
                .await
                .unwrap();
        }
    }
    n.owner
        .add_storage("rustfs", rustfs.kind("family"), false, rustfs.credentials())
        .await
        .unwrap();
    n.sql("CREATE SPACE remote WITH PLACE = 'rustfs', PATH = 'data'")
        .await;
    assert_eq!(n.count("remote.orders").await, 3);
    assert_eq!(n.count("remote.items").await, 2);
    assert_eq!(
        n.texts("SELECT item FROM remote.orders ORDER BY id").await,
        ["tea", "rice", "salt"]
    );
    let location = n.owner.in_place_location("remote").unwrap().unwrap();
    let read = inplace::read_dataset(&n.catalog, &n.engines, &location, "lab/items")
        .await
        .unwrap();
    // Fetched into the cache: the version read, not the others.
    assert!(read.base.starts_with(n.root()), "{}", read.base.display());
    assert!(read.files.iter().all(|f| read.base.join(f).is_file()));
}
