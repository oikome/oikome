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

//! Sources (3.5 exit criteria): flomo memos land in `flomo.memos`; a second
//! pull touches only changed memos; an edited memo is updated, not
//! duplicated; a failed pull leaves data and cursor unchanged. Exercised
//! with flomo's HTML export and a CSV file, written by the tests.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{Array, Int64Array, StringArray};
use async_trait::async_trait;
use iceberg::io::FileIO;
use oikome_catalog::OikomeCatalog;
use oikome_connect::csv::CsvSource;
use oikome_connect::{ConnectError, Connector, Pull, SourceConfig, TableDef, Value};
use oikome_engine_duckdb::DuckDbConfig;
use oikome_node::{
    EngineRegistry, MemorySecrets, NodeConfig, OikomeSession, PullReport, QueryResult,
    SessionConfig,
};

struct Node {
    _dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    owner: OikomeSession,
}

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let config = NodeConfig {
        duckdb: DuckDbConfig {
            extension_dir: Some(dir.path().join("no-extensions")),
            ..Default::default()
        },
        secret_backend: Some(Arc::new(MemorySecrets::default())),
        ..Default::default()
    };
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), config).unwrap());
    let owner = OikomeSession::new(catalog.clone(), engines, SessionConfig::default());
    Node {
        _dir: dir,
        catalog,
        owner,
    }
}

impl Node {
    async fn sql(&self, sql: &str) -> QueryResult {
        self.owner
            .sql(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }
}

fn strings(result: &QueryResult, column: usize) -> Vec<String> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            let a = b
                .column(column)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            (0..a.len())
                .map(|i| a.value(i).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
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

/// Counts of a report: (upserted, deleted) over all tables.
fn counts(report: &PullReport) -> (u64, u64) {
    report
        .tables
        .iter()
        .fold((0, 0), |(u, d), t| (u + t.upserted, d + t.deleted))
}

/// A flomo export holding `memos` as (time, paragraph).
fn write_export(path: &Path, memos: &[(&str, &str)]) {
    let body: String = memos
        .iter()
        .map(|(time, text)| {
            format!(
                r#"<div class="memo"><div class="time">{time}</div><div class="content"><p>{text}</p></div></div>"#
            )
        })
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "flomo@me/index.html",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    std::io::Write::write_all(
        &mut zip,
        format!("<html><body><div class=\"memos\">{body}</div></body></html>").as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();
}

#[tokio::test]
async fn flomo_memos_land_once_and_later_imports_change_only_what_changed() {
    let n = node();
    let dir = tempfile::tempdir().unwrap();
    let export = dir.path().join("flomo_backup.zip");
    write_export(
        &export,
        &[
            ("2026-10-01 08:00:00", "first #reading"),
            ("2026-10-01 08:00:00", "same second"),
            ("2026-10-02 09:00:00", "third"),
        ],
    );
    let config = SourceConfig::FlomoExport {
        path: export.to_str().unwrap().to_string(),
    };
    let first = n
        .owner
        .add_source("flomo", config, None, None)
        .await
        .unwrap();
    assert_eq!(counts(&first), (3, 0));
    let memos = n
        .sql("SELECT id, content, tags FROM flomo.memos ORDER BY id")
        .await;
    assert_eq!(
        strings(&memos, 0),
        vec![
            "2026-10-01 08:00:00#0",
            "2026-10-01 08:00:00#1",
            "2026-10-02 09:00:00#0"
        ]
    );
    assert_eq!(strings(&memos, 2)[0], r#"["reading"]"#);

    // The same export again: nothing to do.
    assert_eq!(counts(&n.owner.pull_source("flomo").await.unwrap()), (0, 0));

    // A newer export: one memo edited, one deleted, one new, one as it was.
    write_export(
        &export,
        &[
            ("2026-10-01 08:00:00", "first, edited #reading"),
            ("2026-10-02 09:00:00", "third"),
            ("2026-10-03 10:00:00", "fourth"),
        ],
    );
    let second = n.owner.pull_source("flomo").await.unwrap();
    // The edited and the new memo; the second one of 08:00:00 is gone.
    assert_eq!(counts(&second), (2, 1));
    let memos = n
        .sql("SELECT id, content FROM flomo.memos ORDER BY id")
        .await;
    assert_eq!(
        strings(&memos, 1),
        vec!["first, edited #reading", "third", "fourth"]
    );
    assert_eq!(
        ints(&n.sql("SELECT count(*) FROM flomo.memos").await),
        vec![3],
        "updated, not duplicated"
    );

    // Publishing shares the memos, never the cursor.
    n.sql("PUBLISH SPACE flomo").await;
    let published = n.catalog.get_space("flomo_published").unwrap();
    assert_eq!(
        n.catalog.space_tables(&published).await.unwrap(),
        vec!["memos"]
    );

    // An export without memos is refused, and changes nothing.
    std::fs::write(&export, b"not a zip").unwrap();
    assert!(n.owner.pull_source("flomo").await.is_err());
    assert_eq!(
        ints(&n.sql("SELECT count(*) FROM flomo.memos").await),
        vec![3]
    );
}

/// A connector that writes a good change, then a row with a missing value.
struct HalfBroken(TableDef);

#[async_trait]
impl Connector for HalfBroken {
    fn kind(&self) -> &'static str {
        "test"
    }
    fn tables(&self) -> Vec<TableDef> {
        vec![self.0.clone()]
    }
    async fn pull(&self, _cursor: Option<&str>) -> Result<Pull, ConnectError> {
        Ok(Pull {
            upserts: vec![
                (
                    "transactions".into(),
                    vec![vec![Value::Int64(1), Value::Float64(999.0)]],
                ),
                ("transactions".into(), vec![vec![Value::Int64(9)]]),
            ],
            deletes: Vec::new(),
            cursor: Some("never stored".into()),
            complete: true,
        })
    }
}

#[tokio::test]
async fn csv_rows_are_upserted_and_a_failed_pull_changes_nothing() {
    let n = node();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bank.csv");
    std::fs::write(&file, "txn_id,amount\n1,10\n2,20\n3,30\n").unwrap();
    let table = CsvSource::inspect(&file, "transactions", &["txn_id".to_string()]).unwrap();
    let config = SourceConfig::Csv {
        path: file.to_str().unwrap().to_string(),
        table: table.clone(),
    };
    assert_eq!(
        counts(
            &n.owner
                .add_source("bank", config, None, None)
                .await
                .unwrap()
        ),
        (3, 0)
    );

    // One row changed, one gone, one new.
    std::fs::write(&file, "txn_id,amount\n1,10\n2,25\n4,40\n").unwrap();
    assert_eq!(counts(&n.owner.pull_source("bank").await.unwrap()), (2, 1));
    let sum = "SELECT CAST(sum(amount) AS BIGINT) FROM bank.transactions";
    assert_eq!(ints(&n.sql(sum).await), vec![75]);
    let before = n.owner.last_pull("bank").await.unwrap();

    // A pull that fails half-way: the first change is rolled back too.
    let e = n
        .owner
        .pull_source_with("bank", &HalfBroken(table))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("1 values for 2 columns"), "{e}");
    assert_eq!(ints(&n.sql(sum).await), vec![75]);
    assert_eq!(n.owner.last_pull("bank").await.unwrap(), before);
    // The cursor is still the file's: unchanged, nothing to do.
    assert_eq!(counts(&n.owner.pull_source("bank").await.unwrap()), (0, 0));

    // The cursor table is not one of the Space's tables.
    assert_eq!(
        n.owner.readable_relations("bank").await.unwrap(),
        vec!["transactions"]
    );

    // Removing the source keeps the data.
    n.owner.remove_source("bank").unwrap();
    assert!(n.owner.pull_source("bank").await.is_err());
    assert_eq!(ints(&n.sql(sum).await), vec![75]);
}
