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

//! A data folder backed up to a repository in a folder, restored and
//! checked; a second backup adds only what changed; a damaged repository is
//! found.

use std::path::Path;
use std::sync::Arc;

use oikome_backup::{
    repository_password, stage, BackupError, Keep, Repository, StoreBackend, MANIFEST_FILE,
};
use oikome_vfs::RemoteStore;

fn write(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn repository(at: &Path, seed: &[u8; 32]) -> Repository {
    let store = RemoteStore::folder(at, "").unwrap();
    let backend = StoreBackend::new(
        Arc::new(store),
        tokio::runtime::Handle::current(),
        at.display().to_string(),
    );
    Repository::new(backend, repository_password(seed))
}

/// Run blocking repository work off the runtime's own threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backup_is_restored_whole_and_a_damaged_one_is_found() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().to_path_buf();
    write(&root, ".oikome/spaces.json", b"{\"spaces\":[]}");
    write(&root, ".oikome/identity/node_ed25519", b"the seed");
    // Something worth deduplicating: the same megabyte in two files.
    let big: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
    write(&root, "photos/trips/data/a.parquet", &big);
    write(&root, "photos/trips/data/b.parquet", &big);
    let target = tempfile::tempdir().unwrap();
    let seed = [7u8; 32];

    let staging = tempfile::tempdir().unwrap();
    let staged = staging.path().join("copy");
    stage(&root, &staged, serde_json::json!({"collections": []}), None).unwrap();
    let at = target.path().to_path_buf();
    let first = {
        let (at, staged) = (at.clone(), staged.clone());
        let repo = repository(&at, &seed);
        blocking(move || repo.back_up(&staged, &Keep::default()))
            .await
            .unwrap()
    };
    assert_eq!(first.files, 4, "three files and the manifest");
    assert!(
        first.added < 2 * 1_048_576,
        "the second copy of the megabyte is not stored again: {}",
        first.added
    );

    // A second backup after a change adds about the change.
    write(&root, "photos/trips/data/c.parquet", b"a few new rows");
    let staged_again = staging.path().join("again");
    stage(&root, &staged_again, serde_json::Value::Null, None).unwrap();
    let second = {
        let repo = repository(&at, &seed);
        blocking(move || repo.back_up(&staged_again, &Keep::default()))
            .await
            .unwrap()
    };
    assert!(
        second.added < 64 * 1024,
        "only what changed: {}",
        second.added
    );
    // A third the same day: of one day's backups the newest is kept, and
    // the oldest, which starts the period, until a later day's comes.
    write(&root, "photos/trips/data/d.parquet", b"more rows");
    let staged_third = staging.path().join("third");
    stage(&root, &staged_third, serde_json::Value::Null, None).unwrap();
    let third = {
        let repo = repository(&at, &seed);
        blocking(move || repo.back_up(&staged_third, &Keep::default()))
            .await
            .unwrap()
    };
    let listed = {
        let repo = repository(&at, &seed);
        blocking(move || repo.snapshots()).await.unwrap()
    };
    let ids: Vec<&str> = listed.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [third.id.as_str(), first.id.as_str()], "{second:?}");

    // Restored into an empty folder, it is the data folder as it was, its
    // manifest checked; the seed was never in it.
    let restored = tempfile::tempdir().unwrap();
    let into = restored.path().join("data");
    let manifest = {
        let (repo, into) = (repository(&at, &seed), into.clone());
        blocking(move || repo.restore(None, &into)).await.unwrap()
    };
    assert_eq!(manifest.files.len(), 5, "the newest: with c and d");
    assert_eq!(
        std::fs::read(into.join("photos/trips/data/a.parquet")).unwrap(),
        big
    );
    assert!(into.join(MANIFEST_FILE).is_file());
    assert!(!into.join(".oikome/identity/node_ed25519").exists());

    // Another key does not open it.
    let other = repository(&at, &[8u8; 32]);
    assert!(blocking(move || other.snapshots()).await.is_err());

    // Whole, it checks; a damaged pack is found.
    let repo = repository(&at, &seed);
    blocking(move || repo.check()).await.unwrap();
    let pack = walk(&at.join("data"))
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).unwrap().len())
        .unwrap();
    let mut bytes = std::fs::read(&pack).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&pack, bytes).unwrap();
    let repo = repository(&at, &seed);
    let damaged = blocking(move || repo.check()).await;
    assert!(
        matches!(
            damaged,
            Err(BackupError::Damaged(_) | BackupError::Repository(_))
        ),
        "{damaged:?}"
    );
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
