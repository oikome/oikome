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

//! Storage credentials (architecture v3 §7.5): the password or token a
//! storage asks for, and the key its files are encrypted with.
//!
//! A credential is looked up in the environment first
//! (`OIKOME_STORAGE_<NAME>_PASSWORD`, `_TOKEN`, `_KEY`), which is how a
//! server or a container is set up, then in the node's secret store: the OS
//! keychain on macOS and Windows, otherwise a file only the node's user can
//! read (`.oikome/identity/storage_access.json`, as protected as the node
//! key beside it). People hand credentials to the node (the apps, `oikome
//! storage add`), which keeps them there; they never enter the storage
//! registry, SQL or logs.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::SessionError;

/// Name of the file secret store, under `.oikome/identity/`.
pub const SECRETS_FILE: &str = "storage_access.json";
const SECRETS_VERSION: u32 = 1;

/// What a credential is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    /// A storage's password (WebDAV Basic authentication).
    Password,
    /// A storage's bearer token.
    Token,
    /// The key a storage's files are encrypted with.
    Key,
}

impl SecretKind {
    pub const ALL: [SecretKind; 3] = [SecretKind::Password, SecretKind::Token, SecretKind::Key];

    fn word(self) -> &'static str {
        match self {
            SecretKind::Password => "password",
            SecretKind::Token => "token",
            SecretKind::Key => "key",
        }
    }

    /// The environment variable that overrides the credential of `storage`.
    pub fn env_var(self, storage: &str) -> String {
        format!(
            "OIKOME_STORAGE_{}_{}",
            storage.to_ascii_uppercase(),
            self.word().to_ascii_uppercase()
        )
    }

    /// The credential's name in a secret store.
    pub fn account(self, storage: &str) -> String {
        format!("storage/{storage}/{}", self.word())
    }
}

/// Where the node keeps credentials. Implementations never log values.
pub trait SecretBackend: Send + Sync + std::fmt::Debug {
    /// `keychain`, `file` or `memory`.
    fn kind(&self) -> &'static str;
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, SessionError>;
    fn set(&self, account: &str, value: &str) -> Result<(), SessionError>;
    /// Remove `account`; a missing one is not an error.
    fn delete(&self, account: &str) -> Result<(), SessionError>;
}

fn secret_error(message: impl Into<String>) -> SessionError {
    CatalogError::Registry(format!("secret store: {}", message.into())).into()
}

/// Credentials in memory only, for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, Zeroizing<String>>>);

impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemorySecrets")
    }
}

impl SecretBackend for MemorySecrets {
    fn kind(&self) -> &'static str {
        "memory"
    }

    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, SessionError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SessionError> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(account.to_string(), Zeroizing::new(value.to_string()));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SessionError> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account);
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct SecretsFile {
    version: u32,
    items: BTreeMap<String, String>,
}

/// Credentials in a file readable by the node's user only, for machines
/// without a keychain (servers, containers).
pub struct FileSecrets {
    path: PathBuf,
    lock: Mutex<()>,
}

impl std::fmt::Debug for FileSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSecrets")
            .field("path", &self.path)
            .finish()
    }
}

impl FileSecrets {
    /// The store in `identity_dir` (`.oikome/identity`).
    pub fn new(identity_dir: &Path) -> Self {
        Self {
            path: identity_dir.join(SECRETS_FILE),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<SecretsFile, SessionError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SecretsFile::default()),
            Err(e) => return Err(CatalogError::Io(e).into()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&self.path)
                .map_err(CatalogError::Io)?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(secret_error(format!(
                    "{} is readable by other users; make it private (chmod 600)",
                    self.path.display()
                )));
            }
        }
        let file: SecretsFile = serde_json::from_slice(&bytes)
            .map_err(|e| secret_error(format!("{}: {e}", self.path.display())))?;
        if file.version != SECRETS_VERSION {
            return Err(secret_error(format!(
                "{} has version {}",
                self.path.display(),
                file.version
            )));
        }
        Ok(file)
    }

    fn save(&self, file: &SecretsFile) -> Result<(), SessionError> {
        use std::io::Write;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| secret_error("a store without a directory"))?;
        std::fs::create_dir_all(dir).map_err(CatalogError::Io)?;
        let bytes = Zeroizing::new(
            serde_json::to_vec_pretty(file).map_err(|e| secret_error(e.to_string()))?,
        );
        let tmp = self.path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        let written = (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut out = options.open(&tmp)?;
            out.write_all(&bytes)?;
            out.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.map_err(|e| CatalogError::Io(e).into())
    }

    fn update(&self, change: impl FnOnce(&mut SecretsFile)) -> Result<(), SessionError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.load()?;
        file.version = SECRETS_VERSION;
        change(&mut file);
        let saved = self.save(&file);
        // Clear the values held in memory.
        for value in file.items.values_mut() {
            zeroize::Zeroize::zeroize(value);
        }
        saved
    }
}

impl SecretBackend for FileSecrets {
    fn kind(&self) -> &'static str {
        "file"
    }

    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, SessionError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.load()?;
        let value = file.items.remove(account).map(Zeroizing::new);
        for other in file.items.values_mut() {
            zeroize::Zeroize::zeroize(other);
        }
        Ok(value)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SessionError> {
        self.update(|file| {
            file.items.insert(account.to_string(), value.to_string());
        })
    }

    fn delete(&self, account: &str) -> Result<(), SessionError> {
        self.update(|file| {
            file.items.remove(account);
        })
    }
}

/// Credentials in the OS keychain (macOS Keychain, Windows Credential
/// Manager), one service per node so that several nodes on one machine
/// keep apart.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct KeychainSecrets {
    store: Arc<keyring_core::CredentialStore>,
    service: String,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl std::fmt::Debug for KeychainSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainSecrets")
            .field("service", &self.service)
            .finish()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl KeychainSecrets {
    /// The keychain of this user, for the node `did`.
    pub fn new(did: &str) -> Result<Self, SessionError> {
        #[cfg(target_os = "macos")]
        let store: Arc<keyring_core::CredentialStore> =
            apple_native_keyring_store::keychain::Store::new()
                .map_err(|e| secret_error(e.to_string()))?;
        #[cfg(target_os = "windows")]
        let store: Arc<keyring_core::CredentialStore> =
            windows_native_keyring_store::Store::new().map_err(|e| secret_error(e.to_string()))?;
        Ok(Self {
            store,
            service: format!("oikome {did}"),
        })
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, SessionError> {
        self.store
            .build(&self.service, account, None)
            .map_err(|e| secret_error(e.to_string()))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl SecretBackend for KeychainSecrets {
    fn kind(&self) -> &'static str {
        "keychain"
    }

    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, SessionError> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_error(e.to_string())),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SessionError> {
        self.entry(account)?
            .set_password(value)
            .map_err(|e| secret_error(e.to_string()))
    }

    fn delete(&self, account: &str) -> Result<(), SessionError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_error(e.to_string())),
        }
    }
}

/// Set to `file`, credentials go to the private file even where there is a
/// keychain: for data centers made to develop or test with. A build made
/// again is a new program to the keychain, which then asks again for each
/// credential; and a test data center's credentials go with its folder.
pub const ENV_SECRET_STORE: &str = "OIKOME_SECRET_STORE";

/// The node's secret store when none is configured: the keychain where
/// the platform has one (unless [`ENV_SECRET_STORE`] says `file`),
/// otherwise the file in `identity_dir`.
pub fn default_backend(identity_dir: &Path, did: &str) -> Arc<dyn SecretBackend> {
    let file = std::env::var(ENV_SECRET_STORE).is_ok_and(|v| v == "file");
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if !file {
        match KeychainSecrets::new(did) {
            Ok(keychain) => return Arc::new(keychain),
            Err(e) => {
                tracing::warn!(error = %e, "no keychain; keeping credentials in a private file")
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (did, file);
    Arc::new(FileSecrets::new(identity_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_store_keeps_credentials_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecrets::new(dir.path());
        assert_eq!(store.get("storage/nas/password").unwrap(), None);
        store.set("storage/nas/password", "pw-1").unwrap();
        store.set("storage/nas/key", "k").unwrap();
        store.set("storage/nas/password", "pw-2").unwrap();
        assert_eq!(
            store
                .get("storage/nas/password")
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("pw-2")
        );
        store.delete("storage/nas/password").unwrap();
        store.delete("storage/nas/password").unwrap();
        assert_eq!(store.get("storage/nas/password").unwrap(), None);
        assert!(store.get("storage/nas/key").unwrap().is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join(SECRETS_FILE);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                store.get("storage/nas/key").is_err(),
                "a readable file is refused"
            );
        }
        assert!(!format!("{store:?}").contains("pw-2"));
    }

    #[test]
    fn names_of_credentials() {
        assert_eq!(
            SecretKind::Key.env_var("home_nas"),
            "OIKOME_STORAGE_HOME_NAS_KEY"
        );
        assert_eq!(SecretKind::Password.account("nas"), "storage/nas/password");
    }
}
