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

//! A restic repository's files on one of Oikome's places, in restic's
//! layout: `config`, `keys/<id>`, `snapshots/<id>`, `index/<id>` and
//! `data/<first two hex digits>/<id>`.

use std::sync::Arc;

use bytes::Bytes;
use oikome_vfs::RemoteStore;
use rustic_core::{
    BytesList, ErrorKind, FileType, Id, ReadBackend, RusticError, RusticResult, WriteBackend,
};
use tokio::runtime::Handle;

/// The largest repository file read whole. restic's packs are far smaller.
const MAX_FILE: u64 = 1 << 30;

/// A repository on a place. rustic calls it from its own threads; each call
/// runs the place's asynchronous operation on `handle`'s runtime.
pub struct StoreBackend {
    store: Arc<RemoteStore>,
    handle: Handle,
    /// What a person is told the repository is.
    label: String,
}

impl StoreBackend {
    pub fn new(store: Arc<RemoteStore>, handle: Handle, label: impl Into<String>) -> Self {
        Self {
            store,
            handle,
            label: label.into(),
        }
    }

    fn block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.handle.block_on(f)
    }
}

fn path_of(tpe: FileType, id: &Id) -> String {
    let hex = id.to_hex();
    let hex = hex.as_str();
    match tpe {
        FileType::Config => "config".to_string(),
        FileType::Pack => format!("data/{}/{hex}", &hex[..2]),
        other => format!("{}/{hex}", other.dirname()),
    }
}

fn failed(what: &str, e: impl std::fmt::Display) -> Box<RusticError> {
    RusticError::new(ErrorKind::Backend, format!("{what}: {e}"))
}

impl ReadBackend for StoreBackend {
    fn location(&self) -> String {
        self.label.clone()
    }

    fn list_with_size(&self, tpe: FileType) -> RusticResult<Vec<(Id, u32)>> {
        if tpe == FileType::Config {
            return match self.block(self.store.size("config")) {
                Ok(Some(size)) => Ok(vec![(Id::default(), size_of(size)?)]),
                Ok(None) => Ok(Vec::new()),
                Err(e) => Err(failed("reading the repository's config", e)),
            };
        }
        let folders = if tpe == FileType::Pack {
            self.block(self.store.list_owned("data"))
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_dir)
                .map(|e| format!("data/{}", e.name))
                .collect()
        } else {
            vec![tpe.dirname().to_string()]
        };
        let mut out = Vec::new();
        for folder in folders {
            // A folder not made yet holds nothing.
            let Ok(entries) = self.block(self.store.list_owned(&folder)) else {
                continue;
            };
            for entry in entries.into_iter().filter(|e| !e.is_dir) {
                if let Ok(id) = entry.name.parse::<Id>() {
                    out.push((id, size_of(entry.size)?));
                }
            }
        }
        Ok(out)
    }

    fn read_full(&self, tpe: FileType, id: &Id) -> RusticResult<Bytes> {
        let path = path_of(tpe, id);
        self.block(self.store.read_owned(&path, 0..u64::MAX, MAX_FILE))
            .map_err(|e| failed(&format!("reading {path}"), e))
    }

    fn read_partial(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        offset: u32,
        length: u32,
    ) -> RusticResult<Bytes> {
        let path = path_of(tpe, id);
        let (start, length) = (u64::from(offset), u64::from(length));
        self.block(self.store.read_owned(&path, start..start + length, length))
            .map_err(|e| failed(&format!("reading {path}"), e))
    }

    fn warmup_path(&self, tpe: FileType, id: &Id) -> String {
        path_of(tpe, id)
    }
}

impl WriteBackend for StoreBackend {
    fn write_bytes(
        &self,
        tpe: FileType,
        id: &Id,
        _cacheable: bool,
        content: BytesList,
    ) -> RusticResult<()> {
        let path = path_of(tpe, id);
        let parts = content.into_vec();
        let bytes = if parts.len() == 1 {
            parts.into_iter().next().unwrap_or_default()
        } else {
            Bytes::from(parts.concat())
        };
        self.block(self.store.write(&path, bytes))
            .map_err(|e| failed(&format!("writing {path}"), e))
    }

    fn remove(&self, tpe: FileType, id: &Id, _cacheable: bool) -> RusticResult<()> {
        let path = path_of(tpe, id);
        self.block(self.store.delete(&path))
            .map_err(|e| failed(&format!("removing {path}"), e))
    }
}

fn size_of(size: u64) -> RusticResult<u32> {
    u32::try_from(size)
        .map_err(|_| RusticError::new(ErrorKind::Backend, "a repository file is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_where_restic_keeps_them() {
        let id: Id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .unwrap();
        assert_eq!(path_of(FileType::Config, &id), "config");
        assert!(path_of(FileType::Pack, &id).starts_with("data/01/0123"));
        assert!(path_of(FileType::Snapshot, &id).starts_with("snapshots/0123"));
        assert!(path_of(FileType::Key, &id).starts_with("keys/"));
        assert!(path_of(FileType::Index, &id).starts_with("index/"));
    }
}
