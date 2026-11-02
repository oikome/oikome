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

//! Places on this computer that Spaces can be kept in, for the owner to
//! pick instead of typing a path: the cloud drives its desktop clients
//! sync, the drives plugged in, and a folder browser. A web page cannot ask
//! the system for a folder, so the node lists them.

use std::path::{Path, PathBuf};

use oikome_catalog::storage::{is_drive_folder, validate_folder};

use crate::error::ApiError;
use crate::types::{FolderEntry, FolderListing, FoundPlace, StorageType};

/// The folder Oikome keeps its files in, inside the folder the owner picked.
pub(crate) const DATA_FOLDER: &str = "Oikome";
/// Most folders one listing shows.
const MAX_FOLDERS: usize = 1000;

/// The home directory of the user running the node.
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Drives plugged into this computer: `/Volumes/*` on macOS (not the
/// start-up disk), `/media/<user>/*` and `/run/media/<user>/*` on Linux.
pub(crate) fn volumes() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Volumes")];
    if let Some(user) = std::env::var_os("USER") {
        roots.push(Path::new("/media").join(&user));
        roots.push(Path::new("/run/media").join(&user));
    }
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // The start-up disk shows up in /Volumes as a link to "/".
            if std::fs::canonicalize(&path).is_ok_and(|p| p == Path::new("/")) {
                continue;
            }
            if path.is_dir() && !hidden(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n.starts_with('.'))
}

fn text(path: &Path) -> Option<String> {
    path.to_str().map(str::to_string)
}

/// The cloud drives synced under `home` and the drives in `volumes`, as
/// places to keep Spaces in.
pub(crate) fn find_places(home: Option<&Path>, volumes: &[PathBuf]) -> Vec<FoundPlace> {
    let mut found: Vec<FoundPlace> = Vec::new();
    let mut add = |label: &str, kind: StorageType, path: &Path| {
        if !path.is_dir() || found.iter().any(|p| p.label == label) {
            return;
        }
        if let Some(path) = text(path) {
            found.push(FoundPlace {
                label: label.to_string(),
                kind,
                path,
            });
        }
    };
    if let Some(home) = home {
        add(
            "iCloud Drive",
            StorageType::Drive,
            &home.join("Library/Mobile Documents/com~apple~CloudDocs"),
        );
        // macOS keeps the other drives' folders together.
        if let Ok(entries) = std::fs::read_dir(home.join("Library/CloudStorage")) {
            let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for path in entries {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("Dropbox") {
                    add("Dropbox", StorageType::Drive, &path);
                } else if name.starts_with("GoogleDrive") {
                    let mine = path.join("My Drive");
                    add(
                        "Google Drive",
                        StorageType::Drive,
                        if mine.is_dir() { &mine } else { &path },
                    );
                } else if name.starts_with("OneDrive") {
                    add("OneDrive", StorageType::Drive, &path);
                }
            }
        }
        add(
            "iCloud Drive",
            StorageType::Drive,
            &home.join("iCloudDrive"),
        );
        add("Dropbox", StorageType::Drive, &home.join("Dropbox"));
        add("OneDrive", StorageType::Drive, &home.join("OneDrive"));
        add(
            "Google Drive",
            StorageType::Drive,
            &home.join("Google Drive"),
        );
    }
    for volume in volumes {
        if let Some(name) = volume.file_name().and_then(|n| n.to_str()) {
            add(name, StorageType::Folder, volume);
        }
    }
    found
}

/// The folders in `path` (the home directory when `None`), hidden ones
/// left out, for the owner to pick one.
pub(crate) fn list_folders(path: Option<&str>) -> Result<FolderListing, ApiError> {
    let dir = match path {
        Some(path) => {
            PathBuf::from(validate_folder(path).map_err(|e| ApiError::invalid(e.to_string()))?)
        }
        None => home().ok_or_else(|| ApiError::not_found("this computer has no home folder"))?,
    };
    let entries = std::fs::read_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            ApiError::not_found(format!("no folder '{}'", dir.display()))
        }
        std::io::ErrorKind::PermissionDenied => {
            ApiError::permission_denied(format!("'{}' can't be opened", dir.display()))
        }
        _ => ApiError::invalid(format!("'{}' is not a folder", dir.display())),
    })?;
    let mut folders: Vec<FolderEntry> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !hidden(p))
        .filter_map(|p| {
            Some(FolderEntry {
                name: p.file_name()?.to_str()?.to_string(),
                path: text(&p)?,
            })
        })
        .collect();
    folders.sort_by_key(|f| f.name.to_lowercase());
    folders.truncate(MAX_FOLDERS);
    Ok(FolderListing {
        path: text(&dir).ok_or_else(|| ApiError::invalid("the folder's name is not text"))?,
        parent: dir.parent().and_then(text),
        folders,
    })
}

/// Where Oikome keeps its files for a folder the owner picked: an `Oikome`
/// folder inside it, made if missing. Its kind is a cloud drive (always
/// encrypted) when it is inside one.
pub(crate) fn data_folder(picked: &str, home: Option<&Path>) -> Result<(String, bool), ApiError> {
    let picked = validate_folder(picked).map_err(|e| ApiError::invalid(e.to_string()))?;
    let picked = PathBuf::from(picked);
    if !picked.is_dir() {
        return Err(ApiError::not_found(format!(
            "no folder '{}'",
            picked.display()
        )));
    }
    let folder = if picked.file_name().is_some_and(|n| n == DATA_FOLDER) {
        picked
    } else {
        picked.join(DATA_FOLDER)
    };
    std::fs::create_dir_all(&folder)
        .map_err(|e| ApiError::invalid(format!("can't make '{}': {e}", folder.display())))?;
    let path = text(&folder).ok_or_else(|| ApiError::invalid("the folder's name is not text"))?;
    let on_drive = home.is_some_and(|h| is_drive_folder(&path, h));
    Ok((path, on_drive))
}

/// A storage name for `label` ("Home NAS" reads `home_nas`), not one of `taken`.
pub(crate) fn storage_name(label: &str, taken: &[String]) -> String {
    let mut name: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    let mut name = name.trim_matches('_').to_string();
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name = format!("storage_{name}").trim_end_matches('_').to_string();
    }
    name.truncate(28);
    let name = name.trim_end_matches('_').to_string();
    if name == "disk" || taken.contains(&name) {
        (2..)
            .map(|n| format!("{name}_{n}"))
            .find(|candidate| !taken.contains(candidate))
            .unwrap_or(name)
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_their_labels() {
        assert_eq!(storage_name("Home NAS", &[]), "home_nas");
        assert_eq!(storage_name("iCloud Drive", &[]), "icloud_drive");
        assert_eq!(storage_name("My Passport!", &[]), "my_passport");
        assert_eq!(storage_name("2024 photos", &[]), "storage_2024_photos");
        assert_eq!(storage_name("Disk", &[]), "disk_2");
        assert_eq!(storage_name("NAS", &["nas".into()]), "nas_2");
        assert_eq!(storage_name("照片", &[]), "storage");
    }

    #[test]
    fn drives_and_volumes_are_found_where_their_clients_keep_them() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        std::fs::create_dir_all(h.join("Library/Mobile Documents/com~apple~CloudDocs")).unwrap();
        std::fs::create_dir_all(h.join("Library/CloudStorage/Dropbox")).unwrap();
        std::fs::create_dir_all(h.join("Library/CloudStorage/GoogleDrive-me@example.com/My Drive"))
            .unwrap();
        let volume = tempfile::tempdir().unwrap();
        let found = find_places(Some(h), &[volume.path().to_path_buf()]);
        let labels: Vec<&str> = found.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels[..3], ["iCloud Drive", "Dropbox", "Google Drive"]);
        assert!(found[2].path.ends_with("My Drive"));
        assert_eq!(found[3].kind, StorageType::Folder);
        assert!(found[..3].iter().all(|p| p.kind == StorageType::Drive));
    }

    #[test]
    fn a_picked_folder_gets_an_oikome_folder_inside() {
        let home = tempfile::tempdir().unwrap();
        let docs = home.path().join("Documents");
        std::fs::create_dir(&docs).unwrap();
        let (path, on_drive) = data_folder(docs.to_str().unwrap(), Some(home.path())).unwrap();
        assert!(path.ends_with("Documents/Oikome"));
        assert!(Path::new(&path).is_dir());
        assert!(!on_drive);
        // Picking the Oikome folder itself keeps it.
        assert_eq!(data_folder(&path, Some(home.path())).unwrap().0, path);

        let dropbox = home.path().join("Dropbox");
        std::fs::create_dir(&dropbox).unwrap();
        assert!(
            data_folder(dropbox.to_str().unwrap(), Some(home.path()))
                .unwrap()
                .1
        );
        assert!(data_folder("relative/path", None).is_err());
        assert!(data_folder("/nowhere/../etc", None).is_err());
    }

    #[test]
    fn folders_are_listed_without_hidden_ones() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b", "A", ".hidden"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("file.txt"), "x").unwrap();
        let listing = list_folders(dir.path().to_str()).unwrap();
        let names: Vec<&str> = listing.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["A", "b"]);
        assert!(listing.parent.is_some());
        assert!(list_folders(Some("not/absolute")).is_err());
    }
}
