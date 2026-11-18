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

//! Lance Spaces (architecture v3 §7.6): tables written, indexed and searched
//! through DuckDB's `lance` extension.
//!
//! Tests marked `#[ignore]` need the extension; they download it once into
//! the target directory (`cargo t --test node -- lance:: --ignored`).

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, Float32Array, Int64Array, StringArray};
use iceberg::io::FileIO;
use oikome_catalog::{LocationFormat, OikomeCatalog};
use oikome_engine_duckdb::DuckDbConfig;
use oikome_node::{
    EngineRegistry, NodeConfig, OikomeSession, QueryResult, SessionConfig, SessionError,
};

struct Node {
    _dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    owner: OikomeSession,
}

/// Extensions shared by the tests that need them, downloaded on first use.
fn extension_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("duckdb-extensions")
}

fn node_with(duckdb: DuckDbConfig) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let catalog =
        Arc::new(OikomeCatalog::open(FileIO::new_with_fs(), dir.path().to_str().unwrap()).unwrap());
    let config = NodeConfig {
        duckdb,
        // Models are shared by the tests that need one, downloaded once.
        models_dir: Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("models")),
        ..Default::default()
    };
    let engines = Arc::new(EngineRegistry::new(catalog.clone(), config).unwrap());
    let owner = OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
    Node {
        _dir: dir,
        catalog,
        engines,
        owner,
    }
}

/// A node with no extensions at all, which never downloads any.
fn offline_node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    node_with(DuckDbConfig {
        extension_dir: Some(dir.keep().join("extensions")),
        ..Default::default()
    })
}

/// A node that loads the `lance` extension.
fn lance_node() -> Node {
    node_with(DuckDbConfig {
        extension_dir: Some(extension_dir()),
        allow_extension_install: true,
        ..Default::default()
    })
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
}

fn ints(result: &QueryResult, column: usize) -> Vec<i64> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            let a = b
                .column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..a.len()).map(|i| a.value(i)).collect::<Vec<_>>()
        })
        .collect()
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

const ITEMS: &str =
    "CREATE TABLE notes.items (id BIGINT, title VARCHAR, body VARCHAR, embedding FLOAT[4])";

/// 300 items: every third body mentions bananas; embeddings repeat.
const SEED: &str = "INSERT INTO notes.items \
     SELECT i, 'title ' || i, \
            CASE WHEN i % 3 = 0 THEN 'apple banana' ELSE 'cherry date' END || ' note ' || i, \
            [i % 7, i % 5, i % 3, 1.0]::FLOAT[4] \
     FROM (SELECT unnest(range(300)) AS i) t";

#[tokio::test]
async fn format_lance_makes_a_lance_location() {
    let n = offline_node();
    n.sql("CREATE SPACE notes WITH FORMAT = 'lance'").await;
    let space = n.catalog.get_space("notes").unwrap();
    let location = n.catalog.get_location(&space.location).unwrap();
    assert!(
        matches!(&location.format, LocationFormat::Lance { root } if root == ".oikome/lance/notes"),
        "{location:?}"
    );
    assert!(n.catalog.lance_root(&location).unwrap().is_dir());

    for (sql, needle) in [
        (
            "CREATE SPACE t WITH KIND = 'transactional', FORMAT = 'lance'",
            "FORMAT",
        ),
        (
            "CREATE SPACE ro WITH LOCATION = 'notes', FORMAT = 'lance'",
            "FORMAT",
        ),
        ("CREATE SPACE p WITH FORMAT = 'parquet'", "unknown format"),
    ] {
        let err = n.err(sql).await.to_string();
        assert!(err.contains(needle), "{sql}: {err}");
    }
}

#[tokio::test]
async fn without_the_extension_a_lance_space_says_how_to_install_it() {
    let n = offline_node();
    n.sql("CREATE SPACE notes WITH FORMAT = 'lance'").await;
    let err = n.err(ITEMS).await.to_string();
    assert!(err.contains("oikome extensions install"), "{err}");
}

#[tokio::test]
async fn search_and_indexes_are_refused_outside_lance_spaces() {
    let n = offline_node();
    for sql in [
        "CREATE SPACE blog",
        "CREATE TABLE blog.posts (id BIGINT, title VARCHAR)",
        "INSERT INTO blog.posts VALUES (1, 'hello')",
        "CREATE SPACE shop WITH KIND = 'transactional'",
        "CREATE TABLE shop.orders (id INTEGER, note TEXT)",
    ] {
        n.sql(sql).await;
    }
    for (sql, needle) in [
        (
            "CREATE INDEX t ON blog.posts USING BTREE (id)",
            "indexes are for Lance spaces",
        ),
        (
            "CREATE INDEX t ON shop.orders USING BTREE (id)",
            "indexes are for Lance spaces",
        ),
        (
            "SELECT * FROM lance_fts(blog.posts, 'title', 'hello')",
            "not stored as Lance",
        ),
        (
            "SELECT * FROM lance_fts(blog.posts, 'title', 'x') a JOIN shop.orders o ON a.id = o.id",
            "several spaces",
        ),
        (
            "MERGE INTO blog.posts t USING blog.posts s ON t.id = s.id WHEN MATCHED THEN DELETE",
            "append-only",
        ),
    ] {
        let err = n.err(sql).await.to_string();
        assert!(err.contains(needle), "{sql}: {err}");
    }

    // A read-only binding never writes to its Location.
    n.sql("CREATE SPACE blog_ro WITH LOCATION = 'blog', ACCESS = 'readonly'")
        .await;
    assert_eq!(
        strings(&n.sql("SELECT title FROM blog_ro.posts").await, 0),
        vec!["hello"]
    );
    for sql in [
        "INSERT INTO blog_ro.posts VALUES (2, 'x')",
        "CREATE TABLE blog_ro.more (id BIGINT)",
    ] {
        assert!(
            matches!(n.err(sql).await, SessionError::ReadOnlySpace(ref s) if s == "blog_ro"),
            "{sql}"
        );
    }

    // Peers never call search functions: they would read past grants.
    let peer = OikomeSession::new(
        n.catalog.clone(),
        n.engines.clone(),
        SessionConfig {
            principal: Some("did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw".into()),
            default_space: None,
        },
    );
    let err = peer
        .sql("SELECT * FROM lance_fts(blog.posts, 'title', 'hello')")
        .await
        .unwrap_err();
    assert!(
        matches!(err, SessionError::PermissionDenied(ref m) if m.contains("subscribe")),
        "{err}"
    );
}

#[tokio::test]
#[ignore = "downloads the DuckDB lance extension"]
async fn lance_tables_are_written_indexed_and_searched() {
    let n = lance_node();
    n.sql("CREATE SPACE notes WITH FORMAT = 'lance'").await;
    n.sql(ITEMS).await;
    assert_eq!(n.sql(SEED).await.rows_affected(), Some(300));
    assert_eq!(
        n.sql("UPDATE notes.items SET title = 'first' WHERE id = 0")
            .await
            .rows_affected(),
        Some(1)
    );
    n.sql("DELETE FROM notes.items WHERE id = 1").await;
    n.sql(
        "MERGE INTO notes.items t USING (SELECT 2 AS id, 'merged' AS title) s ON t.id = s.id \
         WHEN MATCHED THEN UPDATE SET title = s.title",
    )
    .await;
    let rows = n
        .sql("SELECT id, title FROM notes.items WHERE id < 4 ORDER BY id")
        .await;
    assert_eq!(ints(&rows, 0), vec![0, 2, 3]);
    assert_eq!(strings(&rows, 1), vec!["first", "merged", "title 3"]);

    n.sql(
        "CREATE INDEX emb ON notes.items USING IVF_FLAT (embedding) \
         WITH (num_partitions = 2, metric_type = 'l2')",
    )
    .await;
    n.sql("CREATE INDEX body ON notes.items USING INVERTED (body)")
        .await;
    let dataset = n
        .catalog
        .lance_root(
            &n.catalog
                .get_location(&n.catalog.get_space("notes").unwrap().location)
                .unwrap(),
        )
        .unwrap()
        .join("items.lance");
    assert_eq!(
        std::fs::read_dir(dataset.join("_indices")).unwrap().count(),
        2
    );

    // Vector search: the closest embeddings are exact matches.
    let nearest = n
        .sql(
            "SELECT id, _distance FROM lance_vector_search(notes.items, 'embedding', \
             [0, 0, 0, 1]::FLOAT[4], k => 3) ORDER BY _distance, id",
        )
        .await;
    assert_eq!(ints(&nearest, 0), vec![0, 105, 210]);
    let distance = nearest.batches()[0]
        .column(1)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap()
        .value(0);
    assert_eq!(distance, 0.0);

    // Full-text and hybrid search, with SET SPACE resolving the table.
    n.sql("SET SPACE notes").await;
    let found = n
        .sql("SELECT id FROM lance_fts(items, 'body', 'banana', k => 5) ORDER BY id")
        .await;
    assert_eq!(found.num_rows(), 5);
    assert!(ints(&found, 0).iter().all(|id| id % 3 == 0));
    let hybrid = n
        .sql(
            "SELECT count(*) FROM lance_hybrid_search(items, 'embedding', [0, 0, 0, 1]::FLOAT[4], \
             'body', 'banana', k => 4)",
        )
        .await;
    assert_eq!(ints(&hybrid, 0), vec![4]);

    // A Lance table joins tables of other Spaces.
    for sql in [
        "CREATE SPACE tags",
        "CREATE TABLE tags.t (id BIGINT, tag VARCHAR)",
        "INSERT INTO tags.t VALUES (0, 'zero'), (3, 'three')",
    ] {
        n.sql(sql).await;
    }
    let joined = n
        .sql("SELECT i.id, t.tag FROM notes.items i JOIN tags.t t ON t.id = i.id ORDER BY i.id")
        .await;
    assert_eq!(strings(&joined, 1), vec!["zero", "three"]);

    // A read-only binding of the same Location reads it but never writes.
    n.sql("CREATE SPACE notes_ro WITH LOCATION = 'notes', ACCESS = 'readonly'")
        .await;
    assert_eq!(
        ints(&n.sql("SELECT count(*) FROM notes_ro.items").await, 0),
        vec![299]
    );
    assert!(matches!(
        n.err("DELETE FROM notes_ro.items").await,
        SessionError::ReadOnlySpace(_)
    ));
    assert!(matches!(
        n.err("DROP INDEX emb ON notes_ro.items").await,
        SessionError::ReadOnlySpace(_)
    ));

    n.sql("DROP INDEX emb ON notes.items").await;
    n.sql("DROP TABLE notes.items").await;
    let err = n.err("SELECT * FROM notes.items").await.to_string();
    assert!(err.contains("items"), "{err}");
    n.sql("DROP SPACE notes").await;
    assert!(n
        .err("SELECT * FROM notes.items")
        .await
        .to_string()
        .contains("notes"));
}

#[tokio::test]
#[ignore = "downloads the DuckDB lance extension"]
async fn writes_by_another_process_are_seen() {
    let n = lance_node();
    n.sql("CREATE SPACE notes WITH FORMAT = 'lance'").await;
    n.sql(ITEMS).await;
    n.sql(SEED).await;
    // A second DuckDB on the same data directory, as `oikome serve` has
    // while `oikome sql` writes.
    let other = OikomeSession::new(
        n.catalog.clone(),
        Arc::new(
            EngineRegistry::new(
                n.catalog.clone(),
                NodeConfig {
                    duckdb: DuckDbConfig {
                        extension_dir: Some(extension_dir()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap(),
        ),
        SessionConfig::default(),
    );
    let count = || async {
        ints(
            &other.sql("SELECT count(*) FROM notes.items").await.unwrap(),
            0,
        )
    };
    assert_eq!(count().await, vec![300]);
    n.sql("DELETE FROM notes.items WHERE id < 10").await;
    assert_eq!(count().await, vec![290]);
}

/// Words found in `docs`: each hit as `path:first line`, best first.
async fn found(n: &Node, query: &str) -> Vec<String> {
    n.owner
        .search_knowledge("docs", query, 5)
        .await
        .unwrap()
        .hits
        .iter()
        .map(|c| format!("{}:{}", c.path, c.line_start))
        .collect()
}

/// Knowledge (docs/knowledge-schema.md, K1): a folder's Markdown and text
/// split into chunks that keep their places, found in Chinese and English
/// through the `ngram` full-text index, read back by chunk and by lines,
/// turned off and on, and replaced or removed as the originals change.
#[tokio::test]
#[ignore = "downloads the DuckDB lance extension"]
async fn knowledge_is_prepared_found_read_and_kept_up_to_date() {
    use oikome_node::knowledge::{KnowledgeRange, Stage};
    let n = lance_node();
    let folder = tempfile::Builder::new().prefix("docs-").tempdir().unwrap();
    let root = folder.path();
    std::fs::write(
        root.join("architecture.md"),
        "# 架构\n\n## 联邦层\n\nDataFusion 是协调者，从不直接读取文件。\n\n## 多模态\n\n向量检索走 Lance，全文检索也走 Lance。\n",
    )
    .unwrap();
    std::fs::write(
        root.join("sharing.txt"),
        "A public share is the owner's explicit grant to anyone.\n",
    )
    .unwrap();
    std::fs::write(root.join("spending.csv"), "day,euros\n2025-01-01,3.5\n").unwrap();
    n.sql(&format!(
        "CREATE SPACE docs WITH PLACE = 'local', PATH = '{}'",
        root.display()
    ))
    .await;
    n.owner.refresh_in_place("docs").await.unwrap();

    // Before Prepare there is nothing, and a preview writes nothing.
    assert!(n
        .owner
        .knowledge_status("docs")
        .unwrap()
        .prepared_ms
        .is_none());
    let preview = n.owner.preview_knowledge("docs", 5).await.unwrap();
    assert!(!preview.is_empty());
    assert!(n
        .owner
        .knowledge_status("docs")
        .unwrap()
        .prepared_ms
        .is_none());

    // Prepare queues the Markdown and the text; a table is not a document.
    let status = n.owner.prepare_knowledge("docs").await.unwrap();
    assert!(status.prepared_ms.is_some());
    let mut paths: Vec<_> = status.assets.iter().map(|a| a.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, vec!["architecture.md", "sharing.txt"]);
    assert!(status.assets.iter().all(|a| a.stage == Stage::Queued));

    let done = n
        .owner
        .work_knowledge("docs", std::time::Duration::from_secs(120))
        .await
        .unwrap();
    assert_eq!(
        done,
        2,
        "{:?}",
        n.owner.knowledge_status("docs").unwrap().assets
    );
    let status = n.owner.knowledge_status("docs").unwrap();
    assert!(
        status.assets.iter().all(|a| a.stage == Stage::Ready),
        "{:?}",
        status.assets
    );

    // Two-character Chinese words, English words, each with its place.
    assert_eq!(found(&n, "协调者").await, vec!["architecture.md:3"]);
    assert_eq!(found(&n, "向量").await, vec!["architecture.md:7"]);
    assert_eq!(found(&n, "DataFusion").await, vec!["architecture.md:3"]);
    assert_eq!(found(&n, "grant").await, vec!["sharing.txt:1"]);
    assert_eq!(found(&n, "public GRANT").await, vec!["sharing.txt:1"]);
    assert!(found(&n, "public 协调者").await.is_empty());
    assert!(found(&n, "euros").await.is_empty());

    let hit = n
        .owner
        .search_knowledge("docs", "协调者", 1)
        .await
        .unwrap()
        .hits
        .remove(0);
    assert_eq!(hit.heading, vec!["架构", "联邦层"]);
    assert!(hit.score.is_some());

    // Read back: the chunk itself, and the original by lines.
    let read = n
        .owner
        .read_knowledge(
            "docs",
            "architecture.md",
            KnowledgeRange::Chunk {
                chunk_id: hit.chunk_id.clone(),
                around: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(read.text, hit.text);
    let line = n
        .owner
        .read_knowledge(
            "docs",
            "architecture.md",
            KnowledgeRange::Lines {
                page: None,
                start: 5,
                end: 5,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        line.text.trim_end(),
        "DataFusion 是协调者，从不直接读取文件。"
    );

    // A chunk turned off is kept but never returned, nor read by its id.
    n.owner
        .set_knowledge_chunk("docs", &hit.chunk_id, false)
        .unwrap();
    assert!(found(&n, "协调者").await.is_empty());
    let chunks = n
        .owner
        .knowledge_chunks("docs", "architecture.md")
        .await
        .unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks.iter().filter(|(_, on)| !on).count(), 1);
    assert!(n
        .owner
        .read_knowledge(
            "docs",
            "architecture.md",
            KnowledgeRange::Chunk {
                chunk_id: hit.chunk_id.clone(),
                around: 0,
            },
        )
        .await
        .is_err());
    n.owner
        .set_knowledge_chunk("docs", &hit.chunk_id, true)
        .unwrap();
    assert_eq!(found(&n, "协调者").await, vec!["architecture.md:3"]);

    // An original changed: its old chunks go, its new ones are found.
    // One removed: its chunks go with it.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    std::fs::write(
        root.join("architecture.md"),
        "# 架构\n\n协调者只合并 Arrow 流。\n",
    )
    .unwrap();
    std::fs::remove_file(root.join("sharing.txt")).unwrap();
    n.owner.refresh_in_place("docs").await.unwrap();
    n.owner
        .refresh_knowledge(std::time::Duration::from_secs(120))
        .await;
    assert_eq!(found(&n, "协调者").await, vec!["architecture.md:1"]);
    assert!(found(&n, "向量").await.is_empty());
    assert!(found(&n, "grant").await.is_empty());
    let status = n.owner.knowledge_status("docs").unwrap();
    assert_eq!(status.assets.len(), 1);

    // Removing the collection forgets its knowledge; the folder stays.
    n.sql("DROP SPACE docs").await;
    assert!(root.join("architecture.md").exists());
    assert!(!n
        .catalog
        .oikome_dir()
        .join("knowledge")
        .join("docs")
        .exists());
}

/// Knowledge searched by meaning (docs/knowledge-schema.md, K2): the owner
/// downloads Qwen3-Embedding-0.6B, turns it on for a prepared collection,
/// its originals are converted again with vectors while the old chunks keep
/// serving, and a query finds passages by meaning, in any language, alone
/// or fused with words. A chunk turned off stays off.
#[tokio::test]
#[ignore = "downloads the DuckDB lance extension and a 1.2 GB embedding model"]
async fn knowledge_is_found_by_meaning() {
    use oikome_node::knowledge::{model, FoundBy, SearchMode, Stage};
    use std::time::Duration;
    let n = lance_node();
    let folder = tempfile::Builder::new().prefix("notes-").tempdir().unwrap();
    let root = folder.path();
    std::fs::write(
        root.join("cat.md"),
        "# 猫\n\n家里的猫喜欢在午后的阳光下睡觉。\n",
    )
    .unwrap();
    std::fs::write(
        root.join("budget.md"),
        "# Budget\n\nIn March we paid 300 euros for groceries and 80 for the train.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("engines.md"),
        "# Engines\n\nDuckDB runs the analytical queries over Parquet files.\n",
    )
    .unwrap();
    n.sql(&format!(
        "CREATE SPACE docs WITH PLACE = 'local', PATH = '{}'",
        root.display()
    ))
    .await;
    n.owner.refresh_in_place("docs").await.unwrap();
    n.owner.prepare_knowledge("docs").await.unwrap();
    n.owner
        .work_knowledge("docs", Duration::from_secs(120))
        .await
        .unwrap();
    // A chunk turned off before meaning is turned on.
    let engines = n
        .owner
        .search_knowledge("docs", "DuckDB", 1)
        .await
        .unwrap()
        .hits
        .remove(0);
    n.owner
        .set_knowledge_chunk("docs", &engines.chunk_id, false)
        .unwrap();

    // Meaning needs the model here.
    let id = model::MODELS[0].id;
    let here = n
        .owner
        .embedding_models()
        .iter()
        .any(|(m, state)| m.id == id && *state == model::ModelState::Ready);
    if !here {
        assert!(n.owner.set_knowledge_model("docs", Some(id)).await.is_err());
    }
    let started = std::time::Instant::now();
    loop {
        match n.owner.download_embedding_model(id).unwrap() {
            model::ModelState::Ready => break,
            model::ModelState::Failed(e) => panic!("download failed: {e}"),
            _ => {}
        }
        assert!(
            started.elapsed() < Duration::from_secs(1800),
            "download took too long"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let status = n.owner.set_knowledge_model("docs", Some(id)).await.unwrap();
    assert_eq!(status.model.as_deref(), Some(id));
    assert!(status.assets.iter().all(|a| a.stage == Stage::Queued));
    // Until the originals are converted again, words still find them, and
    // meaning finds nothing (they have no vectors yet).
    let words = n
        .owner
        .search_knowledge_by("docs", "groceries", 5, Some(SearchMode::Words))
        .await
        .unwrap();
    assert_eq!(words.hits.len(), 1);
    let meaning = n
        .owner
        .search_knowledge_by("docs", "pets napping", 5, Some(SearchMode::Meaning))
        .await
        .unwrap();
    assert!(meaning.hits.is_empty());

    let done = n
        .owner
        .work_knowledge("docs", Duration::from_secs(600))
        .await
        .unwrap();
    assert_eq!(done, 3);
    let status = n.owner.knowledge_status("docs").unwrap();
    assert!(
        status.assets.iter().all(|a| a.stage == Stage::Ready),
        "{:?}",
        status.assets
    );

    let first = |hits: &[oikome_node::knowledge::StoredChunk]| {
        hits.first().map(|c| c.path.clone()).unwrap_or_default()
    };
    // No word in common, in another language: found by meaning.
    let pets = n
        .owner
        .search_knowledge_by(
            "docs",
            "where does the cat like to sleep",
            3,
            Some(SearchMode::Meaning),
        )
        .await
        .unwrap();
    assert_eq!(first(&pets.hits), "cat.md", "{:?}", pets.hits);
    assert!(
        pets.hits[0].score.unwrap() > 0.3,
        "{:?}",
        pets.hits[0].score
    );
    let money = n
        .owner
        .search_knowledge_by("docs", "how much did food cost", 3, None)
        .await
        .unwrap();
    assert_eq!(money.mode, SearchMode::Hybrid);
    assert_eq!(first(&money.hits), "budget.md", "{:?}", money.hits);
    assert_eq!(money.hits[0].found_by, Some(FoundBy::Meaning));
    // Words and meaning together.
    let both = n
        .owner
        .search_knowledge("docs", "groceries", 3)
        .await
        .unwrap();
    assert_eq!(first(&both.hits), "budget.md");
    assert_eq!(both.hits[0].found_by, Some(FoundBy::Both));
    // The chunk turned off kept its id through the new conversion.
    let engines_now = n
        .owner
        .search_knowledge_by(
            "docs",
            "query engine for Parquet",
            3,
            Some(SearchMode::Meaning),
        )
        .await
        .unwrap();
    assert!(engines_now.hits.iter().all(|c| c.path != "engines.md"));

    // Back to words only: meaning is refused, words still work.
    n.owner.set_knowledge_model("docs", None).await.unwrap();
    assert!(n
        .owner
        .search_knowledge_by("docs", "cat", 3, Some(SearchMode::Meaning))
        .await
        .is_err());
    assert_eq!(
        n.owner
            .search_knowledge("docs", "groceries", 3)
            .await
            .unwrap()
            .mode,
        SearchMode::Words
    );
}
