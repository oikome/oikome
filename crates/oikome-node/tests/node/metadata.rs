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

//! Metadata statements follow the session's grants and analysis types.

use std::sync::Arc;

use arrow_array::{Array, Int64Array, StringArray};
use iceberg::io::FileIO;
use oikome_catalog::OikomeCatalog;
use oikome_node::{
    EngineRegistry, NodeConfig, OikomeSession, QueryResult, SessionConfig, SessionError,
};

struct Node {
    _dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    owner: OikomeSession,
}

impl Node {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(
            OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap(),
        );
        let engines =
            Arc::new(EngineRegistry::new(catalog.clone(), NodeConfig::default()).unwrap());
        let owner = OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
        Self {
            _dir: dir,
            catalog,
            engines,
            owner,
        }
    }

    fn principal(&self, name: &str) -> OikomeSession {
        OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                principal: Some(name.into()),
                ..Default::default()
            },
        )
    }

    async fn run(&self, statements: &[&str]) {
        for sql in statements {
            self.owner
                .sql(sql)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
    }
}

fn strings(result: &QueryResult, column: usize) -> Vec<String> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            let values = b
                .column(column)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            (0..values.len())
                .filter(|i| !values.is_null(*i))
                .map(|i| values.value(i).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn integers(result: &QueryResult, column: usize) -> Vec<i64> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            b.column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

#[tokio::test]
async fn metadata_lists_and_describes_only_readable_collections_tables_and_columns() {
    let n = Node::new();
    n.run(&[
        "CREATE SPACE blog",
        "CREATE SPACE private",
        "CREATE TABLE blog.posts (id BIGINT NOT NULL, title VARCHAR, score DOUBLE, day DATE, changed_at TIMESTAMP, visible BOOLEAN)",
        "CREATE TABLE blog.hidden (secret VARCHAR)",
        "CREATE TABLE private.notes (body VARCHAR)",
        "CREATE VIEW blog.titles AS SELECT id, title FROM blog.posts",
        "INSERT INTO blog.posts VALUES (1, 'hello', 2.5, '2026-01-01', '2026-01-01 10:00:00', true)",
        "GRANT SELECT (id, title) ON blog.posts TO alice",
        "GRANT SELECT ON blog.titles TO alice",
    ])
    .await;

    assert!(n.owner.sql("SHOW TABLES").await.is_err());
    let shown = n.owner.sql("SHOW COLLECTIONS").await.unwrap();
    assert_eq!(strings(&shown, 0), ["blog", "private"]);
    let tables = n.owner.sql("SHOW TABLES FROM blog").await.unwrap();
    assert_eq!(strings(&tables, 0), ["hidden", "posts", "titles"]);
    assert_eq!(strings(&tables, 1), ["managed", "managed", "view"]);
    assert_eq!(strings(&tables, 2), ["iceberg", "iceberg"]);
    assert!(tables.batches()[0].column(2).is_null(2));

    let alice = n.principal("alice");
    let shown = alice.sql("SHOW COLLECTIONS").await.unwrap();
    assert_eq!(strings(&shown, 0), ["blog"]);
    let tables = alice.sql("SHOW TABLES FROM blog").await.unwrap();
    assert_eq!(strings(&tables, 0), ["posts", "titles"]);
    let described = alice.sql("DESCRIBE blog.posts").await.unwrap();
    assert_eq!(strings(&described, 0), ["id", "title"]);
    assert_eq!(strings(&described, 1), ["BIGINT", "VARCHAR"]);
    assert_eq!(strings(&described, 2), ["integer", "text"]);
    assert!(described
        .schema()
        .unwrap()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .eq(["name", "type", "chart_type", "read_type", "nullable"]));

    assert!(alice.sql("SHOW TABLES FROM private").await.is_err());
    assert!(alice.sql("DESCRIBE blog.hidden").await.is_err());
    assert!(alice.sql("DESCRIBE private.notes").await.is_err());
    assert!(alice
        .sql("SELECT * FROM information_schema.tables")
        .await
        .is_err());

    alice.sql("SET SPACE blog").await.unwrap();
    assert_eq!(
        strings(&alice.sql("SHOW TABLES").await.unwrap(), 0),
        ["posts", "titles"]
    );
    assert_eq!(
        strings(&alice.sql("DESCRIBE posts").await.unwrap(), 0),
        ["id", "title"]
    );

    n.run(&["REVOKE SELECT ON blog.titles FROM alice"]).await;
    assert_eq!(
        strings(&alice.sql("SHOW TABLES").await.unwrap(), 0),
        ["posts"]
    );
}

#[tokio::test]
async fn describe_shows_analysis_types_and_detail_without_leaking_paths_to_principals() {
    let n = Node::new();
    n.run(&[
        "CREATE SPACE blog",
        "CREATE TABLE blog.posts (id BIGINT, title VARCHAR, score DOUBLE)",
        "INSERT INTO blog.posts VALUES (1, 'one', 1.0), (2, 'two', 2.0)",
        "GRANT SELECT ON SPACE blog TO alice",
        "CREATE POLICY first_only ON blog.posts TO alice USING (id = 1)",
    ])
    .await;
    let analysis = n.catalog.oikome_dir().join("analysis");
    std::fs::create_dir_all(&analysis).unwrap();
    std::fs::write(
        analysis.join("blog.json"),
        r#"{"tables":{"posts":{"files":[{"path":"data/posts.csv","size":10,"modified_ms":1}],"types":{"id":"decimal","score":"text"},"decimals":{"id":[20,4]},"built_at_ms":123,"rows":1}}}"#,
    )
    .unwrap();

    let described = n.owner.sql("DESCRIBE blog.posts").await.unwrap();
    assert_eq!(strings(&described, 0), ["id", "title", "score"]);
    assert_eq!(strings(&described, 1), ["BIGINT", "VARCHAR", "DOUBLE"]);
    assert_eq!(strings(&described, 2), ["decimal", "text", "text"]);
    assert_eq!(strings(&described, 3), ["DECIMAL(20, 4)", "VARCHAR"]);

    let detail = n.owner.sql("DESCRIBE DETAIL blog.posts").await.unwrap();
    assert_eq!(strings(&detail, 0), ["blog"]);
    assert_eq!(strings(&detail, 1), ["posts"]);
    assert_eq!(strings(&detail, 2), ["managed"]);
    assert_eq!(strings(&detail, 3), ["iceberg"]);
    assert!(strings(&detail, 4)[0].contains("blog"));
    assert_eq!(strings(&detail, 5), ["data/posts.csv"]);
    assert_eq!(integers(&detail, 6), [2]);
    assert_eq!(integers(&detail, 7), [123]);

    let alice = n.principal("alice");
    let detail = alice.sql("DESCRIBE DETAIL blog.posts").await.unwrap();
    assert!(detail.batches()[0].column(4).is_null(0));
    assert!(detail.batches()[0].column(5).is_null(0));
    assert_eq!(integers(&detail, 6), [1]);
    assert!(detail.batches()[0].column(7).is_null(0));
}

#[tokio::test]
async fn metadata_distinguishes_managed_external_and_their_formats() {
    let n = Node::new();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("docs");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("items.csv"), "id,title\n1,hello\n").unwrap();
    n.owner
        .sql(&format!(
            "CREATE SPACE docs WITH PLACE = 'local', PATH = '{}'",
            source.display()
        ))
        .await
        .unwrap();
    n.owner.sql("REFRESH SPACE docs").await.unwrap();

    let tables = n.owner.sql("SHOW TABLES FROM docs").await.unwrap();
    assert_eq!(strings(&tables, 0), ["files", "items"]);
    assert_eq!(strings(&tables, 1), ["managed", "external"]);
    assert_eq!(strings(&tables, 2), ["parquet", "csv"]);

    let analysis = n.catalog.oikome_dir().join("analysis");
    std::fs::create_dir_all(&analysis).unwrap();
    let path = analysis.join("docs.json");
    std::fs::write(
        &path,
        r#"{"tables":{"items":{"files":[],"types":{},"decimals":{},"built_at_ms":1,"rows":1}}}"#,
    )
    .unwrap();
    let tables = n.owner.sql("SHOW TABLES FROM docs").await.unwrap();
    assert_eq!(strings(&tables, 1), ["managed", "managed"]);
    assert_eq!(strings(&tables, 2), ["parquet", "iceberg"]);

    std::fs::write(
        &path,
        r#"{"tables":{"items":{"files":[],"types":{},"decimals":{},"built_at_ms":1,"rows":1,"dataset":{"kind":"lance","columns":[]}}}}"#,
    )
    .unwrap();
    let tables = n.owner.sql("SHOW TABLES FROM docs").await.unwrap();
    assert_eq!(strings(&tables, 1), ["managed", "external"]);
    assert_eq!(strings(&tables, 2), ["parquet", "lance"]);
    let detail = n.owner.sql("DESCRIBE DETAIL docs.items").await.unwrap();
    assert_eq!(strings(&detail, 2), ["external"]);
    assert_eq!(strings(&detail, 3), ["lance"]);
}

#[tokio::test]
async fn show_files_lists_one_folder_with_formats_limits_and_file_table_grants() {
    let n = Node::new();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("docs");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("items.csv"), "id,title\n1,hello\n").unwrap();
    std::fs::write(source.join("notes.md"), "# Notes\n").unwrap();
    std::fs::write(source.join("blob.xyz"), b"unknown").unwrap();
    std::fs::create_dir(source.join("spending")).unwrap();
    std::fs::write(
        source.join("spending/2025-01.csv"),
        "day,amount\n2025-01-01,2\n",
    )
    .unwrap();
    n.owner
        .sql(&format!(
            "CREATE SPACE docs WITH PLACE = 'local', PATH = '{}'",
            source.display()
        ))
        .await
        .unwrap();
    n.owner.sql("REFRESH SPACE docs").await.unwrap();
    n.owner.sql("CREATE SPACE blog").await.unwrap();
    n.owner
        .sql("GRANT SELECT ON docs.files TO alice")
        .await
        .unwrap();
    n.owner
        .sql("GRANT SELECT ON docs.items TO bob")
        .await
        .unwrap();

    assert!(n.owner.sql("SHOW FILES").await.is_err());
    let listed = n.owner.sql("SHOW FILES FROM docs").await.unwrap();
    assert_eq!(
        listed
            .schema()
            .unwrap()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["name", "kind", "format", "size", "modified"]
    );
    assert_eq!(
        listed.schema().unwrap().field(4).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None,)
    );
    assert_eq!(
        strings(&listed, 0),
        ["spending", "blob.xyz", "items.csv", "notes.md"]
    );
    assert_eq!(strings(&listed, 1), ["folder", "file", "table", "file"]);
    assert_eq!(strings(&listed, 2), ["xyz", "csv", "md"]);
    assert!(listed.batches()[0].column(2).is_null(0));

    let limited = n.owner.sql("SHOW FILES FROM docs LIMIT 2").await.unwrap();
    assert_eq!(limited.num_rows(), 2);
    assert_eq!(strings(&limited, 0), ["spending", "blob.xyz"]);
    let inside = n
        .owner
        .sql("SHOW FILES FROM docs IN 'spending'")
        .await
        .unwrap();
    assert_eq!(strings(&inside, 0), ["2025-01.csv"]);
    assert_eq!(strings(&inside, 1), ["table"]);
    assert_eq!(strings(&inside, 2), ["csv"]);
    assert!(n.owner.sql("SHOW FILES FROM blog").await.is_err());
    assert!(n
        .owner
        .sql("SHOW FILES FROM docs LIMIT 10001")
        .await
        .is_err());
    assert!(n.owner.sql("SHOW FILES FROM docs IN '../'").await.is_err());

    let alice = n.principal("alice");
    assert_eq!(
        strings(&alice.sql("SHOW FILES FROM docs").await.unwrap(), 0),
        ["spending", "blob.xyz", "items.csv", "notes.md"]
    );
    assert!(n
        .principal("bob")
        .sql("SHOW FILES FROM docs")
        .await
        .is_err());
    n.owner
        .sql("GRANT SELECT (name) ON docs.files TO partial")
        .await
        .unwrap();
    assert!(n
        .principal("partial")
        .sql("SHOW FILES FROM docs")
        .await
        .is_err());
    n.owner
        .sql("GRANT SELECT ON docs.files TO filtered")
        .await
        .unwrap();
    n.owner
        .sql("CREATE POLICY only_items ON docs.files TO filtered USING (name = 'items.csv')")
        .await
        .unwrap();
    assert!(n
        .principal("filtered")
        .sql("SHOW FILES FROM docs")
        .await
        .is_err());
    alice.sql("SET SPACE docs").await.unwrap();
    assert_eq!(alice.sql("SHOW FILES").await.unwrap().num_rows(), 4);
}

#[tokio::test]
async fn metadata_statements_name_spaces_for_peer_authorization() {
    assert!(matches!(
        oikome_node::query_spaces("SHOW COLLECTIONS"),
        Err(SessionError::PermissionDenied(_))
    ));
    assert_eq!(
        oikome_node::query_spaces("SHOW TABLES FROM blog")
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["blog"]
    );
    assert_eq!(
        oikome_node::query_spaces("DESCRIBE DETAIL blog.posts")
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["blog"]
    );
    assert_eq!(
        oikome_node::query_spaces("SHOW FILES FROM blog")
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["blog"]
    );
    assert!(oikome_node::query_spaces("DESCRIBE posts").is_err());
    assert!(oikome_node::query_spaces("SHOW TABLES").is_err());
    assert!(oikome_node::query_spaces("SHOW FILES").is_err());
    assert!(matches!(
        oikome_node::query_spaces("SHOW IDENTITY"),
        Err(SessionError::PermissionDenied(_))
    ));
}
