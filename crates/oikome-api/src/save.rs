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

//! One deliberate, fixed Save To (architecture §7.8). No continuing rule is
//! kept, and no source is changed. Plain files get unique names; a saved Space
//! carries its originals and portfolio in its snapshots.

use std::collections::BTreeSet;
use std::path::Path;

use arrow_array::{BinaryArray, Int32Array, RecordBatch, StringArray};
use arrow_ipc::reader::StreamReader;
use arrow_schema::{DataType, Field};
use bytes::Bytes;
use iceberg::arrow::arrow_schema_to_schema_auto_assign_ids;
use iceberg::{Catalog, NamespaceIdent, TableCreation, TableIdent};
use oikome_catalog::StorageBinding;
use oikome_core::{CreateSpaceRequest, SpaceKind};
use oikome_node::inplace::{self, FileEntry};
use oikome_semantic::{rewrite::quote_ident, PublishRequest};
use oikome_storage::write_data_file;
use oikome_vfs::RemoteStore;

use crate::{
    Api, ApiError, Caller, ErrorCode, RunSql, SaveDestination, SaveTo, Saved, SavedFile, SqlResult,
};

const MAX_ORIGINAL_BYTES: u64 = 512 * 1024 * 1024;

struct Original {
    entry: FileEntry,
    bytes: Vec<u8>,
}

struct SpaceTarget {
    name: String,
    storage: Option<String>,
    title: String,
    cover: Option<String>,
    captions: std::collections::HashMap<String, String>,
}

enum PlainTarget {
    Local(RemoteStore),
    Remote(oikome_node::storage::Folder),
}

impl PlainTarget {
    async fn exists(&self, path: &str) -> Result<bool, ApiError> {
        match self {
            Self::Local(store) => store
                .size(path)
                .await
                .map(|size| size.is_some())
                .map_err(ApiError::internal),
            Self::Remote(folder) => folder.exists(path).await.map_err(Into::into),
        }
    }

    async fn write(&self, path: &str, bytes: Vec<u8>) -> Result<(), ApiError> {
        match self {
            Self::Local(store) => store
                .write(path, Bytes::from(bytes))
                .await
                .map_err(ApiError::internal),
            Self::Remote(folder) => folder.write(path, bytes).await.map_err(Into::into),
        }
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else {
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "file".into()
    } else {
        out
    }
}

fn render(entry: &FileEntry, folder_rule: &str, name_rule: &str) -> String {
    let taken = entry
        .taken
        .as_deref()
        .or(entry.written.as_deref())
        .unwrap_or("");
    let (year, month, day) = (
        taken.get(..4).unwrap_or(""),
        taken.get(5..7).unwrap_or(""),
        taken.get(..10).unwrap_or(""),
    );
    let replace = |rule: &str| {
        rule.replace("{taken:yyyy-MM-dd}", day)
            .replace("{taken:yyyy}", year)
            .replace("{taken:MM}", month)
            .replace("{title}", entry.title.as_deref().unwrap_or(&entry.name))
            .replace("{name}", &entry.name)
    };
    let folder = replace(folder_rule)
        .split('/')
        .filter(|part| !part.is_empty())
        .map(slug)
        .collect::<Vec<_>>()
        .join("/");
    let extension = Path::new(&entry.name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_ascii_lowercase()))
        .unwrap_or_default();
    let base = replace(if name_rule.is_empty() {
        "{name}"
    } else {
        name_rule
    });
    let base = slug(base.strip_suffix(&extension).unwrap_or(&base));
    let name = format!("{base}{extension}");
    if folder.is_empty() {
        name
    } else {
        format!("{folder}/{name}")
    }
}

fn with_number(path: &str, n: u32) -> String {
    if n == 1 {
        return path.to_string();
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    format!(
        "{}-{n}{extension}",
        path.strip_suffix(&extension).unwrap_or(path)
    )
}

impl Api {
    async fn originals(&self, req: &SaveTo) -> Result<Vec<Original>, ApiError> {
        let Some(location) = self.session(None).in_place_location(&req.source)? else {
            return Ok(Vec::new());
        };
        let listing = inplace::list_deep(&self.catalog, &location, None, 0, 200_000)?;
        let wanted: Option<BTreeSet<String>> = req
            .paths
            .as_ref()
            .map(|paths| paths.iter().cloned().collect());
        let mut found = BTreeSet::new();
        let mut total = 0u64;
        let mut out = Vec::new();
        let place = inplace::place_of(&self.catalog, &self.engines, &location)?;
        for entry in listing.files {
            if wanted
                .as_ref()
                .is_some_and(|wanted| !wanted.contains(entry.path.as_str()))
            {
                continue;
            }
            found.insert(entry.path.clone());
            total = total.checked_add(entry.size).ok_or_else(|| {
                ApiError::invalid("the selected files are too large to save at once")
            })?;
            if total > MAX_ORIGINAL_BYTES {
                return Err(ApiError::invalid("up to 512 MiB can be saved at once"));
            }
            let bytes = place.read(&entry.path, 0..entry.size, entry.size).await?;
            out.push(Original { entry, bytes });
        }
        if let Some(wanted) = wanted {
            let missing: Vec<String> = wanted.difference(&found).cloned().collect();
            if !missing.is_empty() {
                return Err(ApiError::not_found(format!(
                    "these files are not in {}: {missing:?}",
                    req.source
                )));
            }
        }
        if let Some(paths) = &req.paths {
            out.sort_by_key(|original| {
                paths
                    .iter()
                    .position(|path| path == &original.entry.path)
                    .unwrap_or(usize::MAX)
            });
        }
        Ok(out)
    }

    async fn save_plain(
        &self,
        originals: Vec<Original>,
        place: Option<String>,
        path: String,
        folder_rule: String,
        name_rule: String,
    ) -> Result<Saved, ApiError> {
        if originals.is_empty() {
            return Err(ApiError::invalid("choose at least one file"));
        }
        let target = match place {
            Some(storage) => PlainTarget::Remote(
                self.session(None)
                    .open_folder(&StorageBinding { storage, path })?,
            ),
            None => {
                let root = Path::new(&path);
                if !root.is_absolute() {
                    return Err(ApiError::invalid(
                        "a destination folder on this computer must be an absolute path",
                    ));
                }
                std::fs::create_dir_all(root).map_err(ApiError::internal)?;
                PlainTarget::Local(RemoteStore::folder(root, "").map_err(ApiError::internal)?)
            }
        };
        let mut files = Vec::with_capacity(originals.len());
        for original in originals {
            let path = render(&original.entry, &folder_rule, &name_rule);
            let mut n = 1;
            let unique = loop {
                let candidate = with_number(&path, n);
                if !target.exists(&candidate).await? {
                    break candidate;
                }
                n += 1;
                if n > 10_000 {
                    return Err(ApiError::new(
                        ErrorCode::Conflict,
                        format!("no free name for {path}"),
                    ));
                }
            };
            target.write(&unique, original.bytes.clone()).await?;
            files.push(SavedFile {
                path: unique,
                size: original.bytes.len() as u64,
                hash: blake3::hash(&original.bytes).to_string(),
            });
        }
        Ok(Saved { files, space: None })
    }

    pub(crate) async fn write_saved_table(
        &self,
        space: &str,
        table: &str,
        schema: arrow_schema::SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), ApiError> {
        let namespace = NamespaceIdent::new(space.to_string());
        let ident = TableIdent::new(namespace.clone(), table.to_string());
        let iceberg_schema =
            arrow_schema_to_schema_auto_assign_ids(&schema).map_err(ApiError::internal)?;
        if !self
            .catalog
            .table_exists(&ident)
            .await
            .map_err(ApiError::internal)?
        {
            self.catalog
                .create_table(
                    &namespace,
                    TableCreation::builder()
                        .name(table.to_string())
                        .schema(iceberg_schema)
                        .build(),
                )
                .await
                .map_err(ApiError::internal)?;
        }
        let location = self
            .catalog
            .load_table(&ident)
            .await
            .map_err(ApiError::internal)?
            .metadata()
            .location()
            .to_string();
        let temp = Path::new(self.catalog.root_path()).join("tmp");
        std::fs::create_dir_all(&temp).map_err(ApiError::internal)?;
        let mut files = Vec::new();
        for batch in batches {
            files.push(
                write_data_file(self.catalog.file_io(), &location, &schema, &batch, &temp)
                    .await
                    .map_err(ApiError::internal)?,
            );
        }
        self.catalog
            .replace_data_files(&ident, files)
            .await
            .map_err(ApiError::internal)?;
        Ok(())
    }

    async fn save_space(
        &self,
        req: &SaveTo,
        originals: Vec<Original>,
        target: SpaceTarget,
    ) -> Result<Saved, ApiError> {
        let SpaceTarget {
            name,
            storage,
            title,
            cover,
            captions,
        } = target;
        if self.catalog.get_space(&name).is_ok() || self.catalog.get_location(&name).is_ok() {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!("{name} is taken; choose another name"),
            ));
        }
        let source = self.catalog.get_space(&req.source)?;
        let session = self.session(None);
        session
            .create_space(CreateSpaceRequest {
                name: name.clone(),
                kind: Some(SpaceKind::Analytical),
                storage: storage.clone(),
                ..Default::default()
            })
            .await?;
        match source.kind {
            SpaceKind::Transactional => {
                self.catalog.mark_published(&name, &source.name)?;
                session
                    .publish(&PublishRequest {
                        space: source.name.clone(),
                        tables: None,
                        target: Some(name.clone()),
                    })
                    .await?;
            }
            SpaceKind::Analytical => {
                for table in session.readable_relations(&source.name).await? {
                    let result = self
                        .run_sql(
                            &Caller::Owner,
                            RunSql {
                                sql: format!(
                                    "SELECT * FROM {}.{}",
                                    quote_ident(&source.name),
                                    quote_ident(&table),
                                ),
                                space: None,
                            },
                        )
                        .await?;
                    let SqlResult::Rows(rows) = result else {
                        continue;
                    };
                    if rows.truncated {
                        return Err(ApiError::invalid(
                            "this collection is too large to copy in one Save To",
                        ));
                    }
                    let reader = StreamReader::try_new(rows.ipc.as_slice(), None)
                        .map_err(ApiError::internal)?;
                    let schema = reader.schema();
                    let batches = reader
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(ApiError::internal)?;
                    self.write_saved_table(&name, &table, schema, batches)
                        .await?;
                }
            }
            _ => {
                return Err(ApiError::invalid(
                    "a saved collection must come from your own collection",
                ));
            }
        }

        let schema = arrow_schema::SchemaRef::new(arrow_schema::Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("position", DataType::Int32, false),
            Field::new("caption", DataType::Utf8, true),
            Field::new("taken", DataType::Utf8, true),
            Field::new("content", DataType::Binary, false),
            Field::new("kind", DataType::Utf8, false),
            Field::new("hash", DataType::Utf8, false),
        ]));
        let paths = originals
            .iter()
            .map(|o| o.entry.path.clone())
            .collect::<Vec<_>>();
        let names = originals
            .iter()
            .map(|o| o.entry.name.clone())
            .collect::<Vec<_>>();
        let positions = (0..originals.len() as i32).collect::<Vec<_>>();
        let texts = originals
            .iter()
            .map(|o| captions.get(&o.entry.path).cloned())
            .collect::<Vec<_>>();
        let taken = originals
            .iter()
            .map(|o| o.entry.taken.clone().or(o.entry.written.clone()))
            .collect::<Vec<_>>();
        let bytes = originals
            .iter()
            .map(|o| o.bytes.clone())
            .collect::<Vec<_>>();
        let kinds = originals
            .iter()
            .map(|o| o.entry.kind.clone())
            .collect::<Vec<_>>();
        let hashes = originals
            .iter()
            .map(|o| blake3::hash(&o.bytes).to_string())
            .collect::<Vec<_>>();
        self.write_saved_table(
            &name,
            "_oikome_files",
            schema.clone(),
            vec![RecordBatch::try_new(
                schema.clone(),
                vec![
                    std::sync::Arc::new(StringArray::from(paths)),
                    std::sync::Arc::new(StringArray::from(names)),
                    std::sync::Arc::new(Int32Array::from(positions)),
                    std::sync::Arc::new(StringArray::from(texts)),
                    std::sync::Arc::new(StringArray::from(taken)),
                    std::sync::Arc::new(BinaryArray::from(
                        bytes
                            .iter()
                            .map(|bytes| bytes.as_slice())
                            .collect::<Vec<_>>(),
                    )),
                    std::sync::Arc::new(StringArray::from(kinds)),
                    std::sync::Arc::new(StringArray::from(hashes)),
                ],
            )
            .map_err(ApiError::internal)?],
        )
        .await?;
        let portfolio_schema = arrow_schema::SchemaRef::new(arrow_schema::Schema::new(vec![
            Field::new("title", DataType::Utf8, false),
            Field::new("cover", DataType::Utf8, true),
        ]));
        self.write_saved_table(
            &name,
            "_oikome_portfolio",
            portfolio_schema.clone(),
            vec![RecordBatch::try_new(
                portfolio_schema,
                vec![
                    std::sync::Arc::new(StringArray::from(vec![title])),
                    std::sync::Arc::new(StringArray::from(vec![cover])),
                ],
            )
            .map_err(ApiError::internal)?],
        )
        .await?;
        if storage.is_some() {
            session
                .sql(&format!("REFRESH SPACE {}", quote_ident(&name)))
                .await?;
        }
        let space = self.summary_of(&Caller::Owner, &name).await?;
        let files = originals
            .into_iter()
            .map(|original| SavedFile {
                path: original.entry.path,
                size: original.bytes.len() as u64,
                hash: blake3::hash(&original.bytes).to_string(),
            })
            .collect();
        Ok(Saved {
            files,
            space: Some(space),
        })
    }

    /// Save a fixed selection once. No rule or source is changed, and the
    /// destination never silently replaces an existing file or collection.
    pub async fn save_to(&self, caller: &Caller, req: SaveTo) -> Result<Saved, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let originals = self.originals(&req).await?;
        match req.destination.clone() {
            SaveDestination::Files {
                place,
                path,
                folder_rule,
                name_rule,
            } => {
                self.save_plain(originals, place, path, folder_rule, name_rule)
                    .await
            }
            SaveDestination::Space {
                name,
                storage,
                title,
                cover,
                captions,
            } => {
                self.save_space(
                    &req,
                    originals,
                    SpaceTarget {
                        name,
                        storage,
                        title,
                        cover,
                        captions,
                    },
                )
                .await
            }
        }
    }
}
