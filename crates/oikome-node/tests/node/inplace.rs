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

//! Folders viewed in place (architecture v3 §7.7): read where they are,
//! never written, with an index that is a cache.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use iceberg::io::FileIO;
use oikome_catalog::OikomeCatalog;
use oikome_engine_duckdb::DuckDbConfig;
use oikome_node::{
    inplace, EngineRegistry, MemorySecrets, NodeConfig, OikomeSession, QueryResult, SessionConfig,
};

struct Node {
    _dir: tempfile::TempDir,
    catalog: Arc<OikomeCatalog>,
    engines: Arc<EngineRegistry>,
    owner: OikomeSession,
}

fn node() -> Node {
    let dir = tempfile::Builder::new().prefix("node-").tempdir().unwrap();
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
    let owner = OikomeSession::new(catalog.clone(), engines.clone(), SessionConfig::default());
    Node {
        _dir: dir,
        catalog,
        engines,
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

    fn session(&self, principal: &str) -> OikomeSession {
        OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                default_space: None,
                principal: Some(principal.to_string()),
            },
        )
    }

    fn location(&self, space: &str) -> oikome_catalog::Location {
        self.owner.in_place_location(space).unwrap().unwrap()
    }
}

fn texts(result: &QueryResult, column: usize) -> Vec<String> {
    result
        .batches()
        .iter()
        .flat_map(|b| {
            let a = b
                .column(column)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("text");
            (0..a.len())
                .map(|i| a.value(i).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn count(result: &QueryResult) -> i64 {
    result.batches()[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, 90])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A folder as an owner might keep one: a table of each kind at the top,
/// photos with captions, notes, a subfolder, a hidden file.
fn trip() -> tempfile::TempDir {
    // Not a hidden folder, as tempdir()'s `.tmp…` would be.
    let dir = tempfile::Builder::new().prefix("trip-").tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("2025.csv"),
        "day,euros\n2025-01-01,3.5\n2025-01-02,4\n",
    )
    .unwrap();
    std::fs::write(
        root.join("items.json"),
        "{\"name\": \"bread\", \"n\": 2}\n{\"name\": \"milk\", \"n\": 1}\n",
    )
    .unwrap();
    {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let file = std::fs::File::create(root.join("rows.parquet")).unwrap();
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }
    std::fs::write(root.join("tram.png"), png(800, 400)).unwrap();
    std::fs::write(
        root.join("captions.csv"),
        "file,title,place,taken\ntram.png,Tram 28,Lisbon,2025-10-20 09:40\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("Notes")).unwrap();
    std::fs::write(
        root.join("Notes").join("1 Hello.md"),
        "---\ndate: 2025-10-21\n---\nThe trams of Lisbon climb all day.\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("Lisbon").join("Belém")).unwrap();
    std::fs::write(
        root.join("Lisbon").join("Belém").join("Pastéis.png"),
        png(64, 64),
    )
    .unwrap();
    std::fs::write(root.join(".hidden.txt"), "not listed").unwrap();
    dir
}

/// Every file under `root` with its size, time and hash: what "nothing
/// written" is checked against.
fn state(root: &Path) -> BTreeMap<PathBuf, (u64, std::time::SystemTime, String)> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                dirs.push(entry.path());
            } else {
                let bytes = std::fs::read(entry.path()).unwrap();
                out.insert(
                    entry.path(),
                    (
                        meta.len(),
                        meta.modified().unwrap(),
                        blake3::hash(&bytes).to_hex().to_string(),
                    ),
                );
            }
        }
    }
    out
}

#[tokio::test]
async fn a_folder_is_browsed_queried_and_searched_where_it_is_and_never_written() {
    let n = node();
    let folder = trip();
    let before = state(folder.path());
    n.sql(&format!(
        "CREATE SPACE trip WITH PLACE = 'local', PATH = '{}'",
        folder.path().display()
    ))
    .await;
    n.sql("REFRESH SPACE trip").await;

    // `files`: a row per file, hidden ones left out, photos and notes read.
    let files = n
        .sql("SELECT path, kind, title, place, taken, written FROM trip.files ORDER BY path")
        .await;
    assert_eq!(
        texts(&files, 0),
        [
            "2025.csv",
            "Lisbon/Belém/Pastéis.png",
            "Notes/1 Hello.md",
            "captions.csv",
            "items.json",
            "rows.parquet",
            "tram.png"
        ]
    );
    assert_eq!(
        texts(&files, 1),
        ["table", "image", "note", "table", "table", "table", "image"]
    );
    let tram = n
        .sql("SELECT title, place, taken FROM trip.files WHERE path = 'tram.png'")
        .await;
    assert_eq!(texts(&tram, 0), ["Tram 28"]);
    assert_eq!(texts(&tram, 1), ["Lisbon"]);
    assert_eq!(texts(&tram, 2), ["2025-10-20 09:40:00"]);
    let note = n
        .sql("SELECT written, text FROM trip.files WHERE kind = 'note'")
        .await;
    assert_eq!(texts(&note, 0), ["2025-10-21"]);
    assert!(texts(&note, 1)[0].contains("trams of Lisbon"));

    // A table per data file at the top, read where it is; the captions
    // name the photos and are no table.
    let tables = n.owner.readable_relations("trip").await.unwrap();
    assert_eq!(tables, ["files", "items", "rows", "t_2025"]);
    assert_eq!(count(&n.sql("SELECT count(*) FROM trip.t_2025").await), 2);
    assert_eq!(
        count(&n.sql("SELECT CAST(sum(id) AS BIGINT) FROM trip.rows").await),
        6
    );
    assert_eq!(
        count(&n.sql("SELECT CAST(sum(n) AS BIGINT) FROM trip.items").await),
        3
    );

    // Thumbnails are kept in the index, turned and made small.
    let location = n.location("trip");
    let thumb = inplace::thumb(&n.catalog, &location, "tram.png")
        .unwrap()
        .expect("a thumbnail");
    let small = image::load_from_memory(&thumb).unwrap();
    assert_eq!((small.width(), small.height()), (512, 256));

    // Browsing: folders with what they hold, files photos first.
    let top = inplace::list(&n.catalog, &location, "", Default::default(), 0, 50).unwrap();
    let folders: Vec<(String, u64)> = top
        .folders
        .iter()
        .map(|f| (f.name.clone(), f.files))
        .collect();
    assert_eq!(
        folders,
        [("Lisbon".to_string(), 1), ("Notes".to_string(), 1)]
    );
    assert_eq!(top.total, 5);
    assert_eq!(top.files[0].path, "tram.png");
    assert!(top.files[0].thumbnail);
    let deeper = inplace::list(&n.catalog, &location, "Lisbon", Default::default(), 0, 50).unwrap();
    assert_eq!(deeper.folders[0].name, "Belém");
    assert_eq!(deeper.total, 0);
    // A folder's cover, as a collection's: its newest photos from any depth,
    // else its newest note; and what it holds.
    let cover = |f: &inplace::SubFolder| f.cover.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
    assert_eq!(cover(&top.folders[0]), ["Lisbon/Belém/Pastéis.png"]);
    assert_eq!((top.folders[0].photos, top.folders[0].notes), (1, 0));
    assert_eq!(cover(&top.folders[1]), ["Notes/1 Hello.md"]);
    assert!(top.folders[1].cover[0]
        .text
        .as_deref()
        .is_some_and(|t| t.contains("trams of Lisbon")));
    assert_eq!((top.folders[1].photos, top.folders[1].notes), (0, 1));
    assert_eq!(cover(&deeper.folders[0]), ["Lisbon/Belém/Pastéis.png"]);
    // A CSV file keeps its first lines, to show some rows.
    let csv = top.files.iter().find(|f| f.path == "2025.csv").unwrap();
    assert_eq!(
        csv.text.as_deref(),
        Some("day,euros\n2025-01-01,3.5\n2025-01-02,4")
    );
    // In the order asked, either way; a folder says its bytes and newest file.
    assert!(top
        .folders
        .iter()
        .all(|f| f.size > 0 && f.modified_ms.is_some()));
    let in_order = |order, descending| {
        inplace::list(&n.catalog, &location, "", (order, descending), 0, 50).unwrap()
    };
    let sizes: Vec<u64> = in_order(inplace::FileOrder::Size, true)
        .files
        .iter()
        .map(|f| f.size)
        .collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    let names: Vec<String> = in_order(inplace::FileOrder::Name, false)
        .files
        .iter()
        .map(|f| f.name.to_lowercase())
        .collect();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(names, sorted_names);
    let (files, bytes, newest) = inplace::totals(&n.catalog, &location).unwrap();
    assert_eq!(files, 7);
    assert!(bytes > 0 && newest.is_some());

    // Search goes through the index.
    let found = n
        .owner
        .search("trip", &["trams".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].key.as_deref(),
        Some(&["Notes/1 Hello.md".to_string()][..])
    );
    let row = n
        .owner
        .row_by_key("trip", "files", &["tram.png".to_string()])
        .await
        .unwrap();
    assert_eq!(row.batches()[0].num_rows(), 1);

    // Read-only: nothing is written there, whatever is asked.
    for refused in [
        "INSERT INTO trip.t_2025 VALUES ('2025-01-03', 1)",
        "CREATE TABLE trip.more (id BIGINT)",
        "DELETE FROM trip.files",
    ] {
        assert!(n.owner.sql(refused).await.is_err(), "{refused}");
    }
    assert_eq!(state(folder.path()), before);

    // Dropping the Space forgets the index and keeps the folder.
    n.sql("DROP SPACE trip").await;
    assert!(!n.catalog.index_dir(&location).exists());
    assert_eq!(state(folder.path()), before);
}

#[tokio::test]
async fn a_file_opens_as_rows_where_it_is_until_its_place_goes_offline() {
    let n = node();
    let folder = trip();
    let path = folder.path().to_path_buf();
    n.sql(&format!(
        "CREATE SPACE trip WITH PLACE = 'local', PATH = '{}'",
        path.display()
    ))
    .await;
    n.sql("REFRESH SPACE trip").await;
    let location = n.location("trip");

    let page = inplace::file_rows(&n.catalog, &n.engines, &location, "2025.csv", 1, 1)
        .await
        .unwrap();
    assert_eq!(page.rows.iter().map(|b| b.num_rows()).sum::<usize>(), 1);
    let days = page.rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(days.value(0), "2025-01-02");
    let captions = inplace::file_rows(&n.catalog, &n.engines, &location, "captions.csv", 0, 100)
        .await
        .unwrap();
    assert_eq!(captions.rows.iter().map(|b| b.num_rows()).sum::<usize>(), 1);

    let away = path.with_extension("away");
    std::fs::rename(&path, &away).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    n.owner.refresh_in_place("trip").await.unwrap();
    let error = inplace::file_rows(&n.catalog, &n.engines, &location, "2025.csv", 0, 100)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("can't be reached"), "{error}");
}

#[tokio::test]
async fn offline_the_index_still_answers_and_deleted_it_is_rebuilt() {
    let n = node();
    let folder = trip();
    let path = folder.path().to_path_buf();
    n.sql(&format!(
        "CREATE SPACE trip WITH PLACE = 'local', PATH = '{}'",
        path.display()
    ))
    .await;
    n.sql("REFRESH SPACE trip").await;
    let location = n.location("trip");

    // The folder goes away (a disk unplugged): names, text, search and
    // thumbnails are still there; its tables say why they can't be read.
    let away = path.with_extension("away");
    std::fs::rename(&path, &away).unwrap();
    // Asked again after the 15 s it is remembered for.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let refreshed = n.owner.refresh_in_place("trip").await.unwrap();
    assert!(!refreshed.reachable);
    assert_eq!(refreshed.files, 7);
    assert_eq!(count(&n.sql("SELECT count(*) FROM trip.files").await), 7);
    assert_eq!(
        n.owner
            .search("trip", &["trams".to_string()], 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(inplace::thumb(&n.catalog, &location, "tram.png")
        .unwrap()
        .is_some());
    std::fs::rename(&away, &path).unwrap();

    // The index is a cache: deleted, it is built again when next read.
    std::fs::remove_dir_all(n.catalog.index_dir(&location)).unwrap();
    assert_eq!(count(&n.sql("SELECT count(*) FROM trip.files").await), 7);
    let refreshed = n.owner.refresh_in_place("trip").await.unwrap();
    assert!(refreshed.reachable);
    assert!(inplace::thumb(&n.catalog, &location, "tram.png")
        .unwrap()
        .is_some());

    // A change is seen at the next scan; a removed file goes.
    std::fs::write(path.join("Notes").join("2 More.md"), "Ferries too.").unwrap();
    std::fs::remove_file(path.join("items.json")).unwrap();
    let refreshed = n.owner.refresh_in_place("trip").await.unwrap();
    assert!(refreshed.changed);
    assert_eq!(refreshed.files, 7);
    let tables = n.owner.readable_relations("trip").await.unwrap();
    assert!(!tables.contains(&"items".to_string()));
}

#[tokio::test]
async fn damaged_json_is_isolated_from_files_even_in_an_old_index() {
    let n = node();
    let folder = trip();
    std::fs::write(
        folder.path().join("damaged.json"),
        "{\"place\": broken json}\n",
    )
    .unwrap();
    n.sql(&format!(
        "CREATE SPACE trip WITH PLACE = 'local', PATH = '{}'",
        folder.path().display()
    ))
    .await;
    n.sql("REFRESH SPACE trip").await;
    n.sql("SELECT path, kind FROM trip.files ORDER BY path")
        .await;

    // An index written before damaged JSON was marked unavailable: reading
    // it again must validate that old row rather than bind the broken view.
    let location = n.location("trip");
    let index =
        rusqlite::Connection::open(n.catalog.index_dir(&location).join("index.sqlite")).unwrap();
    index
        .execute("UPDATE files SET state = 1 WHERE path = 'damaged.json'", [])
        .unwrap();
    index
        .execute(
            "DELETE FROM facts WHERE key = 'json_table_validation_v1'",
            [],
        )
        .unwrap();
    drop(index);
    n.owner.refresh_in_place("trip").await.unwrap();
    n.sql("SELECT path, kind FROM trip.files ORDER BY path")
        .await;
    assert!(n.owner.sql("SELECT * FROM trip.damaged").await.is_err());
}

#[tokio::test]
async fn only_a_visible_folder_of_the_owners_can_be_viewed() {
    let n = node();
    let folder = trip();
    std::fs::create_dir(folder.path().join(".private")).unwrap();
    let oikome_dir = n.catalog.oikome_dir();
    std::fs::create_dir_all(oikome_dir.join("inside")).unwrap();
    for (path, why) in [
        ("relative/folder".to_string(), "whole path"),
        ("/no/such/folder".to_string(), "no folder"),
        (
            folder.path().join("2025.csv").display().to_string(),
            "not a folder",
        ),
        (
            folder.path().join(".private").display().to_string(),
            "hidden",
        ),
        (
            oikome_dir.join("inside").display().to_string(),
            "own folder",
        ),
    ] {
        let e = n
            .owner
            .sql(&format!(
                "CREATE SPACE nope WITH PLACE = 'local', PATH = '{path}'"
            ))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            e.contains(why) || e.contains("hidden") || e.contains("file"),
            "{path}: {e}"
        );
        assert!(n.catalog.get_space("nope").is_err());
    }
    let e = n
        .owner
        .sql("CREATE SPACE nope WITH PLACE = 'nas', PATH = 'photos'")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("nas"), "{e}");
    let e = n
        .owner
        .sql(&format!(
            "CREATE SPACE nope WITH PLACE = 'local', PATH = '{}', ACCESS = 'writable'",
            folder.path().display()
        ))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("read-only"), "{e}");
}

#[tokio::test]
async fn a_contact_granted_a_folder_queries_it_and_is_cut_off_when_revoked() {
    let n = node();
    let folder = trip();
    n.sql(&format!(
        "CREATE SPACE trip WITH PLACE = 'local', PATH = '{}'",
        folder.path().display()
    ))
    .await;
    n.sql("REFRESH SPACE trip").await;
    n.sql("GRANT SELECT ON SPACE trip TO alice").await;
    let alice = n.session("alice");
    let seen = alice
        .sql("SELECT path FROM trip.files WHERE kind = 'image' ORDER BY path")
        .await
        .unwrap();
    assert_eq!(texts(&seen, 0), ["Lisbon/Belém/Pastéis.png", "tram.png"]);
    assert_eq!(
        count(&alice.sql("SELECT count(*) FROM trip.t_2025").await.unwrap()),
        2
    );
    // Found through the index, read again as alice.
    let found = alice
        .search("trip", &["trams".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    let row = alice
        .row_by_key("trip", "files", &["Notes/1 Hello.md".to_string()])
        .await
        .unwrap();
    assert_eq!(row.batches()[0].num_rows(), 1);

    n.sql("REVOKE SELECT ON SPACE trip FROM alice").await;
    assert!(alice.sql("SELECT path FROM trip.files").await.is_err());
    assert!(alice
        .search("trip", &["trams".to_string()], 10)
        .await
        .map_or(true, |m| m.is_empty()));
}
