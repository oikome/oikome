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

//! Storages (architecture v3 §7.5): places other than this node's disk
//! that hold Spaces: a NAS at home reached over WebDAV or mounted as a
//! folder, or a cloud drive's sync folder.
//!
//! A storage is registered on each node that uses it (`oikome storage
//! add`). The registry keeps its address, user name and whether its files
//! are encrypted, never a password, token or key: the node keeps those
//! apart (`oikome-node`'s secrets).

use std::path::{Component, Path};

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};

use crate::catalog::OikomeCatalog;
use crate::registry::now_ms;
use crate::space::{Location, STORAGE_DISK};

/// Longest storage name; it also names environment variables.
const MAX_STORAGE_NAME: usize = 32;
/// Longest folder of a Space on a storage.
const MAX_STORAGE_PATH: usize = 255;

/// A registered storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDef {
    pub name: String,
    #[serde(flatten)]
    pub kind: StorageKind,
    /// Files are encrypted on this node before they reach the storage.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypted: bool,
    pub created_at_ms: u64,
}

/// How a storage is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StorageKind {
    /// A WebDAV server, `https://host[:port][/path]`.
    Webdav {
        endpoint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
    /// A local folder, such as a NAS share mounted over SMB or NFS; an
    /// absolute path.
    Folder { path: String },
    /// A folder a cloud drive's desktop client syncs (iCloud Drive, Dropbox,
    /// OneDrive, Google Drive); an absolute path. Always encrypted: the
    /// drive's provider can read what is uploaded.
    Drive { path: String },
}

impl StorageKind {
    /// Whether the storage is reached over plaintext HTTP.
    pub fn is_plaintext(&self) -> bool {
        matches!(self, StorageKind::Webdav { endpoint, .. } if endpoint.starts_with("http://"))
    }
}

/// Where a Space lives on a storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageBinding {
    /// Name of the registered storage.
    pub storage: String,
    /// The Space's folder on it: plain names separated by `/`.
    pub path: String,
}

/// Check a storage name: a lowercase letter, then lowercase letters,
/// digits or `_`. Its upper-case form names the environment variables that
/// hold the storage's password or token.
pub fn validate_storage_name(name: &str) -> Result<(), CatalogError> {
    let mut chars = name.chars();
    let ok = name.len() <= MAX_STORAGE_NAME
        && name != STORAGE_DISK
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CatalogError::InvalidProperty(format!(
            "storage name '{name}': use a lowercase letter, then lowercase letters, digits or '_' \
             (at most {MAX_STORAGE_NAME}; 'disk' is reserved)"
        )))
    }
}

/// Check the folder of a Space on a storage: plain names (`[A-Za-z0-9._-]`,
/// not `.` or `..`) separated by `/`, without a leading or trailing `/`.
pub fn validate_storage_path(path: &str) -> Result<(), CatalogError> {
    let plain = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && !s.starts_with(".tmp-")
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if path.len() <= MAX_STORAGE_PATH && path.split('/').all(plain) {
        Ok(())
    } else {
        Err(CatalogError::InvalidProperty(format!(
            "PATH '{path}': use names of letters, digits, '.', '_' or '-' separated by '/'"
        )))
    }
}

/// Check a WebDAV address: `http(s)://host[:port][/path]`, without user
/// name, password, query or fragment. Returns it without a trailing `/`.
pub fn validate_webdav_endpoint(endpoint: &str) -> Result<String, CatalogError> {
    let invalid = |why: &str| CatalogError::InvalidProperty(format!("WebDAV '{endpoint}': {why}"));
    let rest = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .ok_or_else(|| invalid("expected https://host[:port][/path]"))?;
    if rest
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#' | '\\' | '%'))
    {
        return Err(invalid(
            "only a host, a port and a path are allowed; give the user name with --username",
        ));
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() || authority.starts_with(':') {
        return Err(invalid("missing host"));
    }
    let path = path.trim_end_matches('/');
    if !path.is_empty() {
        validate_storage_path(path).map_err(|_| invalid("the path must be plain names"))?;
    }
    Ok(endpoint.trim_end_matches('/').to_string())
}

/// Folders, relative to the home directory, that cloud drives' desktop
/// clients sync: a folder under one of them is on a drive.
const DRIVE_FOLDERS: [&str; 7] = [
    "Library/Mobile Documents", // iCloud Drive (macOS)
    "Library/CloudStorage",     // Dropbox, OneDrive, Google Drive, Box (macOS)
    "iCloudDrive",              // iCloud for Windows
    "Dropbox",                  // also "Dropbox (Personal)"
    "OneDrive",                 // also "OneDrive - <organization>"
    "Google Drive",
    "Box",
];

/// Whether `path` is inside a folder a cloud drive's desktop client syncs,
/// under the home directory `home`.
pub fn is_drive_folder(path: &str, home: &Path) -> bool {
    let Ok(rest) = Path::new(path).strip_prefix(home) else {
        return false;
    };
    let rest = rest.to_string_lossy().replace('\\', "/");
    DRIVE_FOLDERS.iter().any(|drive| {
        rest.strip_prefix(drive).is_some_and(|after| {
            after.is_empty()
                || after.starts_with('/')
                || (!drive.contains('/') && after.starts_with([' ', '-', '(']))
        })
    })
}

/// The home directory, for [`is_drive_folder`].
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(Into::into)
}

/// Check a mounted folder: an absolute path without `.` or `..`.
pub fn validate_folder(path: &str) -> Result<String, CatalogError> {
    // `components()` drops `.` segments, so check the text as well.
    if !Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        || path.split(['/', '\\']).any(|s| s == "." || s == "..")
    {
        return Err(CatalogError::InvalidProperty(format!(
            "folder '{path}': expected an absolute path without '.' or '..'"
        )));
    }
    Ok(path.trim_end_matches('/').to_string())
}

impl OikomeCatalog {
    /// Register the storage `name`, its files encrypted on this node when
    /// `encrypted`. A cloud drive must be encrypted. Fails if the name is
    /// taken.
    pub fn add_storage(
        &self,
        name: &str,
        kind: StorageKind,
        encrypted: bool,
    ) -> Result<StorageDef, CatalogError> {
        validate_storage_name(name)?;
        let on_drive = match &kind {
            StorageKind::Drive { .. } => true,
            StorageKind::Folder { path } => {
                home_dir().is_some_and(|home| is_drive_folder(path, &home))
            }
            StorageKind::Webdav { .. } => false,
        };
        if on_drive && !encrypted {
            return Err(CatalogError::InvalidProperty(format!(
                "storage '{name}' is on a cloud drive, whose provider can read what is uploaded; \
                 add it as a drive, which is encrypted"
            )));
        }
        let kind = match kind {
            StorageKind::Webdav { endpoint, username } => {
                if username
                    .as_deref()
                    .is_some_and(|u| u.is_empty() || u.chars().any(|c| c.is_control()))
                {
                    return Err(CatalogError::InvalidProperty(
                        "the WebDAV user name must not be empty".to_string(),
                    ));
                }
                StorageKind::Webdav {
                    endpoint: validate_webdav_endpoint(&endpoint)?,
                    username,
                }
            }
            StorageKind::Folder { path } => StorageKind::Folder {
                path: validate_folder(&path)?,
            },
            StorageKind::Drive { path } => StorageKind::Drive {
                path: validate_folder(&path)?,
            },
        };
        let def = StorageDef {
            name: name.to_string(),
            kind,
            encrypted,
            created_at_ms: now_ms(),
        };
        self.mutate_registry(|reg| {
            if reg.storages.contains_key(name) {
                return Err(CatalogError::InvalidProperty(format!(
                    "storage '{name}' is already registered"
                )));
            }
            reg.storages.insert(name.to_string(), def.clone());
            Ok(())
        })?;
        Ok(def)
    }

    /// The storage `name`.
    pub fn get_storage(&self, name: &str) -> Result<StorageDef, CatalogError> {
        self.with_registry(|reg| reg.storages.get(name).cloned())
            .ok_or_else(|| CatalogError::StorageNotFound(name.to_string()))
    }

    /// All storages, ordered by name.
    pub fn list_storages(&self) -> Vec<StorageDef> {
        self.with_registry(|reg| reg.storages.values().cloned().collect())
    }

    /// Unregister the storage `name`, with the Locations that dropped
    /// Spaces left on it; refused while a Space is on it. Nothing on the
    /// storage is touched: the files of those Locations stay where they are.
    pub fn remove_storage(&self, name: &str) -> Result<(), CatalogError> {
        self.mutate_registry(|reg| {
            let on_it = |l: &Location| l.storage.as_ref().is_some_and(|b| b.storage == name);
            if let Some(space) = reg
                .spaces
                .values()
                .find(|s| reg.locations.get(&s.location).is_some_and(on_it))
            {
                return Err(CatalogError::InvalidProperty(format!(
                    "storage '{name}' holds space '{}'; drop it first",
                    space.name
                )));
            }
            reg.storages
                .remove(name)
                .ok_or_else(|| CatalogError::StorageNotFound(name.to_string()))?;
            reg.locations.retain(|_, l| !on_it(l));
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_paths_and_addresses_are_checked() {
        for good in ["nas", "home_nas2"] {
            validate_storage_name(good).unwrap();
        }
        for bad in ["", "disk", "Nas", "2nas", "n-as", "n as", &"n".repeat(33)] {
            assert!(validate_storage_name(bad).is_err(), "{bad}");
        }
        for good in ["photos", "family/photos", "a.b_c-d"] {
            validate_storage_path(good).unwrap();
        }
        for bad in ["", "/a", "a/", "a//b", "..", "a/../b", ".", "a b", ".tmp-x"] {
            assert!(validate_storage_path(bad).is_err(), "{bad}");
        }
        assert_eq!(
            validate_webdav_endpoint("https://nas.local:5006/dav/").unwrap(),
            "https://nas.local:5006/dav"
        );
        validate_webdav_endpoint("http://172.28.0.20:8080").unwrap();
        for bad in [
            "nas.local",
            "ftp://nas",
            "https://me:pw@nas",
            "https://nas/a?b",
            "https:///dav",
            "https://nas/../x",
            "https://nas/a%2Fb",
        ] {
            assert!(validate_webdav_endpoint(bad).is_err(), "{bad}");
        }
        validate_folder("/Volumes/nas/oikome").unwrap();
        for bad in ["relative", "/a/../b", "/a/./b"] {
            assert!(validate_folder(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn folders_of_cloud_drives_are_recognised() {
        let home = Path::new("/Users/me");
        for drive in [
            "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Oikome",
            "/Users/me/Library/CloudStorage/Dropbox/Oikome",
            "/Users/me/Dropbox",
            "/Users/me/Dropbox (Personal)/x",
            "/Users/me/OneDrive - Contoso/x",
            "/Users/me/Google Drive/x",
        ] {
            assert!(is_drive_folder(drive, home), "{drive}");
        }
        for not in [
            "/Volumes/nas/oikome",
            "/Users/me/Documents/x",
            "/Users/me/Dropboxes",
            "/Users/other/Dropbox",
        ] {
            assert!(!is_drive_folder(not, home), "{not}");
        }
    }
}
