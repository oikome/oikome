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

//! A restored data folder in another place than the one backed up: the
//! paths Iceberg keeps absolute (in tables' metadata, manifest lists and
//! manifests) and the registries' paths within the data folder are moved to
//! the new folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use apache_avro::types::Value;
use apache_avro::{Codec, Reader, Writer};

use crate::{io, BackupError};

/// `text` moved from `old` to `new`, when it is `old` or within it.
fn moved(text: &str, old: &str, new: &str) -> Option<String> {
    let rest = text.strip_prefix(old)?;
    (rest.is_empty() || rest.starts_with('/')).then(|| format!("{new}{rest}"))
}

fn move_json(value: &mut serde_json::Value, old: &str, new: &str) -> bool {
    match value {
        serde_json::Value::String(s) => match moved(s, old, new) {
            Some(m) => {
                *s = m;
                true
            }
            None => false,
        },
        serde_json::Value::Array(items) => items
            .iter_mut()
            .fold(false, |any, v| move_json(v, old, new) || any),
        serde_json::Value::Object(map) => map
            .values_mut()
            .fold(false, |any, v| move_json(v, old, new) || any),
        _ => false,
    }
}

/// `value` with its paths moved; a manifest list's `manifest_length` is
/// set to the size the moved manifest has now.
fn move_avro(value: Value, old: &str, new: &str, sizes: &HashMap<String, i64>) -> Value {
    match value {
        Value::String(s) => Value::String(moved(&s, old, new).unwrap_or(s)),
        Value::Union(i, v) => Value::Union(i, Box::new(move_avro(*v, old, new, sizes))),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| move_avro(v, old, new, sizes))
                .collect(),
        ),
        Value::Map(map) => Value::Map(
            map.into_iter()
                .map(|(k, v)| (k, move_avro(v, old, new, sizes)))
                .collect(),
        ),
        Value::Record(fields) => {
            let mut fields: Vec<(String, Value)> = fields
                .into_iter()
                .map(|(k, v)| (k, move_avro(v, old, new, sizes)))
                .collect();
            let path = fields.iter().find_map(|(k, v)| match (k.as_str(), v) {
                ("manifest_path", Value::String(p)) => Some(p.clone()),
                _ => None,
            });
            if let Some(size) = path.and_then(|p| sizes.get(&p)) {
                for (k, v) in &mut fields {
                    if k == "manifest_length" {
                        *v = Value::Long(*size);
                    }
                }
            }
            Value::Record(fields)
        }
        other => other,
    }
}

fn rewrite(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let tmp = path.with_extension("relocating");
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// Rewrite the Avro file at `path` with its paths moved; whether it is a
/// manifest list.
fn move_avro_file(
    path: &Path,
    old: &str,
    new: &str,
    sizes: &HashMap<String, i64>,
) -> Result<(), BackupError> {
    let bytes = std::fs::read(path).map_err(io)?;
    let reader = Reader::new(bytes.as_slice()).map_err(io)?;
    let schema = reader.writer_schema().clone();
    let metadata = reader.user_metadata().clone();
    let records: Vec<Value> = reader
        .map(|r| r.map(|v| move_avro(v, old, new, sizes)))
        .collect::<Result<_, _>>()
        .map_err(io)?;
    let mut writer = Writer::with_codec(&schema, Vec::new(), Codec::Null);
    for (key, value) in metadata {
        writer.add_user_metadata(key, value).map_err(io)?;
    }
    for record in records {
        writer.append(record).map_err(io)?;
    }
    rewrite(path, &writer.into_inner().map_err(io)?)
}

fn is_manifest_list(path: &Path) -> Result<bool, BackupError> {
    let bytes = std::fs::read(path).map_err(io)?;
    let reader = Reader::new(bytes.as_slice()).map_err(io)?;
    Ok(reader
        .writer_schema()
        .canonical_form()
        .contains("\"manifest_path\""))
}

/// Every file in a `metadata` folder under `root`, outside `.oikome`.
fn metadata_files(root: &Path) -> Result<Vec<PathBuf>, BackupError> {
    let mut out = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).map_err(io)?.flatten() {
            let path = entry.path();
            let kind = entry.file_type().map_err(io)?;
            if kind.is_dir() && path != root.join(".oikome") {
                folders.push(path);
            } else if kind.is_file()
                && folder.file_name().and_then(|n| n.to_str()) == Some("metadata")
            {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Move the data folder restored at `root`, backed up from `old`, to `root`:
/// tables' metadata, manifests and manifest lists, then the registries.
/// How many files changed.
pub fn relocate(root: &Path, old: &str) -> Result<usize, BackupError> {
    let new = root
        .to_str()
        .ok_or_else(|| io("the data folder's path is not UTF-8"))?
        .trim_end_matches('/')
        .to_string();
    let old = old.trim_end_matches('/');
    if old == new {
        return Ok(0);
    }
    let files = metadata_files(root)?;
    let mut changed = 0;
    let (mut lists, mut manifests) = (Vec::new(), Vec::new());
    for file in &files {
        match file.extension().and_then(|e| e.to_str()) {
            Some("json") => {
                let mut json: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(file).map_err(io)?).map_err(io)?;
                if move_json(&mut json, old, &new) {
                    rewrite(file, &serde_json::to_vec(&json).map_err(io)?)?;
                    changed += 1;
                }
            }
            Some("avro") if is_manifest_list(file)? => lists.push(file),
            Some("avro") => manifests.push(file),
            _ => {}
        }
    }
    // Manifests first, so their lists can say their new sizes.
    let mut sizes = HashMap::new();
    for file in manifests {
        move_avro_file(file, old, &new, &HashMap::new())?;
        let size = std::fs::metadata(file).map_err(io)?.len();
        sizes.insert(
            file.to_string_lossy().into_owned(),
            i64::try_from(size).unwrap_or(i64::MAX),
        );
        changed += 1;
    }
    for file in lists {
        move_avro_file(file, old, &new, &sizes)?;
        changed += 1;
    }
    for entry in std::fs::read_dir(root.join(".oikome"))
        .map_err(io)?
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let Ok(mut json) =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).map_err(io)?)
        else {
            continue;
        };
        if move_json(&mut json, old, &new) {
            rewrite(&path, &serde_json::to_vec_pretty(&json).map_err(io)?)?;
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_within_the_old_folder_move() {
        assert_eq!(
            moved("/a/data/ns/t", "/a/data", "/b").as_deref(),
            Some("/b/ns/t")
        );
        assert_eq!(moved("/a/data", "/a/data", "/b").as_deref(), Some("/b"));
        assert_eq!(moved("/a/data2/x", "/a/data", "/b"), None);
        assert_eq!(moved("relative/x", "/a/data", "/b"), None);
    }
}
