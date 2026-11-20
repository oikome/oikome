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

//! Datasets in a collection's folders (PR 31): Copy to Oikome, which moves
//! a dataset into a collection as Oikome's own table, and the DuckDB
//! extensions that read datasets. Reading them is in `file_analysis` (their
//! tables) and the node's `datasets` module.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use futures::TryStreamExt;
use iceberg::arrow::arrow_schema_to_schema_auto_assign_ids;
use iceberg::{Catalog, NamespaceIdent, TableCreation, TableIdent};
use oikome_catalog::LocationFormat;
use oikome_core::{CreateSpaceRequest, SpaceFormat};
use oikome_engine::{Capabilities, QualifiedName};
use oikome_node::datasets::{self, DatasetKind as NodeKind, HashedFile};
use oikome_node::inplace;
use oikome_storage::write_data_file;
use serde::Serialize;

use crate::{
    Api, ApiError, Caller, CopiedToOikome, CopyToOikome, DatasetKind, ExtensionDownload,
    ExtensionRef,
};

/// Where a dataset copied to Oikome came from, kept beside the copy.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Provenance<'a> {
    collection: &'a str,
    path: &'a str,
    kind: &'a str,
    version: Option<&'a str>,
    written_at_ms: Option<i64>,
    copied_at_ms: u64,
    rows: u64,
    /// For an Iceberg table, the files of the version read; for a Lance
    /// dataset, every file copied.
    files: Vec<ProvenanceFile<'a>>,
}

#[derive(Debug, Serialize)]
struct ProvenanceFile<'a> {
    path: &'a str,
    size: u64,
    blake3: &'a str,
}

pub(crate) fn kind_of(kind: NodeKind) -> DatasetKind {
    match kind {
        NodeKind::Lance => DatasetKind::Lance,
        NodeKind::Iceberg => DatasetKind::Iceberg,
    }
}

/// The capability that loads the extension reading `kind`.
pub(crate) fn capability_of(kind: NodeKind) -> Capabilities {
    match kind {
        NodeKind::Lance => Capabilities::READ_LANCE,
        NodeKind::Iceberg => Capabilities::READ_ICEBERG,
    }
}

impl Api {
    /// Where an extension's download is, for `name` (`lance`, `iceberg`).
    pub(crate) fn extension_download(&self, name: &str) -> ExtensionDownload {
        crate::knowledge::extension_state(&self.session(None), name)
    }

    /// Start downloading a DuckDB extension Oikome reads datasets with,
    /// from DuckDB's repository, and installing it; DuckDB checks its
    /// signature whenever it loads it. Owner only.
    pub async fn install_extension(
        &self,
        caller: &Caller,
        req: ExtensionRef,
    ) -> Result<ExtensionDownload, ApiError> {
        caller.require_owner()?;
        let Some(kind) = NodeKind::from_name(&req.name) else {
            return Err(ApiError::invalid(format!(
                "{} is not an extension Oikome downloads",
                req.name
            )));
        };
        let session = self.session(None);
        session.install_extension_support(kind.extension())?;
        Ok(self.extension_download(kind.extension()))
    }

    /// Copy a dataset's table into a collection as Oikome's own table: an
    /// Iceberg table's current data into an Iceberg collection, a Lance
    /// dataset whole (with its versions) into a Lance collection. Where it
    /// came from is kept beside the copy. The dataset is not changed.
    pub async fn copy_to_oikome(
        &self,
        caller: &Caller,
        req: CopyToOikome,
    ) -> Result<CopiedToOikome, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let location = self.in_place_of(&req.space)?;
        let found = inplace::dataset_tables(&self.catalog, &location)?
            .into_iter()
            .find(|d| d.name == req.table)
            .ok_or_else(|| {
                ApiError::not_found(format!("{} has no dataset {}", req.space, req.table))
            })?;
        let read = inplace::read_dataset(&self.catalog, &self.engines, &location, &found.path)
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?;
        let target = self.copy_target(&req, found.kind).await?;
        let (table, rows, files) = match found.kind {
            NodeKind::Iceberg => {
                let table = self
                    .free_table_name(&target, &req.table, found.kind)
                    .await?;
                let rows = self
                    .copy_rows(&target, &table, &read.source, &req.table)
                    .await?;
                let base = read.base.clone();
                let files = read.files.clone();
                let hashed =
                    tokio::task::spawn_blocking(move || datasets::hash_files(&base, &files))
                        .await
                        .map_err(ApiError::internal)??;
                (table, rows, hashed)
            }
            NodeKind::Lance => {
                let table = self
                    .free_table_name(&target, &req.table, found.kind)
                    .await?;
                let root = self.lance_root_of(&target)?;
                let dest = root.join(format!("{table}{}", oikome_catalog::lance::DATASET_SUFFIX));
                let place = inplace::place_of(&self.catalog, &self.engines, &location)?;
                let copied = datasets::copy_whole(&place, &found.path, &dest).await?;
                // The copy is read as Oikome reads its own: refused, and
                // removed, if it uses files of another dataset.
                if let Err(e) = oikome_catalog::lance::lance_version(&dest, None).await {
                    let _ = std::fs::remove_dir_all(&dest);
                    return Err(ApiError::invalid(format!("{}: {e}", found.path)));
                }
                let rows = self.count_rows(&target, &table).await?;
                (table, rows, copied)
            }
        };
        self.keep_provenance(&target, &table, &req.space, &found, &read, rows, &files)?;
        Ok(CopiedToOikome {
            space: target,
            table,
            rows,
            files: files.len() as u64,
        })
    }

    /// The collection a copy goes into: made when asked, else an existing
    /// collection of this computer that holds that kind of table.
    async fn copy_target(&self, req: &CopyToOikome, kind: NodeKind) -> Result<String, ApiError> {
        let format = match kind {
            NodeKind::Iceberg => SpaceFormat::Iceberg,
            NodeKind::Lance => SpaceFormat::Lance,
        };
        if req.new {
            crate::validate_space_name(&req.to)?;
            if self.catalog.get_space(&req.to).is_ok() || self.catalog.get_location(&req.to).is_ok()
            {
                return Err(ApiError::new(
                    crate::ErrorCode::Conflict,
                    format!("a collection named {} exists already", req.to),
                ));
            }
            self.session(None)
                .create_space(CreateSpaceRequest {
                    name: req.to.clone(),
                    kind: Some(oikome_core::SpaceKind::Analytical),
                    format: Some(format),
                    ..Default::default()
                })
                .await?;
            return Ok(req.to.clone());
        }
        let space = self.catalog.get_space(&req.to)?;
        let location = self.catalog.get_location(&space.location)?;
        let fits = space.kind == oikome_core::SpaceKind::Analytical
            && matches!(
                (&location.format, kind),
                (LocationFormat::IcebergParquet { .. }, NodeKind::Iceberg)
                    | (LocationFormat::Lance { .. }, NodeKind::Lance)
            );
        if !fits {
            return Err(ApiError::invalid(format!(
                "{} can't hold a copy of a {} dataset; choose another collection or a new one",
                req.to,
                kind.name()
            )));
        }
        Ok(req.to.clone())
    }

    fn lance_root_of(&self, space: &str) -> Result<PathBuf, ApiError> {
        let space = self.catalog.get_space(space)?;
        let location = self.catalog.get_location(&space.location)?;
        Ok(self.catalog.lance_root(&location)?)
    }

    /// `wanted`, or `wanted_2`, … : a table name not taken in `space`.
    async fn free_table_name(
        &self,
        space: &str,
        wanted: &str,
        kind: NodeKind,
    ) -> Result<String, ApiError> {
        let taken = |name: String| async move {
            match kind {
                NodeKind::Iceberg => self
                    .catalog
                    .table_exists(&TableIdent::new(
                        NamespaceIdent::new(space.to_string()),
                        name,
                    ))
                    .await
                    .map_err(ApiError::internal),
                NodeKind::Lance => Ok(self
                    .lance_root_of(space)?
                    .join(format!("{name}{}", oikome_catalog::lance::DATASET_SUFFIX))
                    .exists()),
            }
        };
        let mut name = wanted.to_string();
        let mut n = 2;
        while taken(name.clone()).await? {
            name = format!("{wanted}_{n}");
            n += 1;
        }
        Ok(name)
    }

    /// Read every row of the dataset `source` and keep them as the new
    /// Iceberg table `<space>.<table>`, a data file per batch read.
    async fn copy_rows(
        &self,
        space: &str,
        table: &str,
        source: &oikome_engine::TableSource,
        from: &str,
    ) -> Result<u64, ApiError> {
        let found = self.catalog.get_space(space)?;
        let engine = self.engines.engine_for(&found)?;
        let scratch = QualifiedName::new("__oikome_copy", format!("{table}_{}", crate::now_ms()));
        engine
            .attach(&scratch, source.clone())
            .await
            .map_err(|e| ApiError::invalid(format!("{from}: {e}")))?;
        let written = async {
            let (schema, mut stream) = engine
                .query(&format!("SELECT * FROM {}", scratch.quoted()), &[])
                .await
                .map_err(|e| ApiError::invalid(format!("{from}: {e}")))?;
            let namespace = NamespaceIdent::new(space.to_string());
            if !self
                .catalog
                .namespace_exists(&namespace)
                .await
                .map_err(ApiError::internal)?
            {
                self.catalog
                    .create_namespace(&namespace, HashMap::new())
                    .await
                    .map_err(ApiError::internal)?;
            }
            let iceberg_schema = arrow_schema_to_schema_auto_assign_ids(&schema)
                .map_err(|e| ApiError::invalid(format!("{from}: {e}")))?;
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
            let ident = TableIdent::new(namespace, table.to_string());
            let made = async {
                let loaded = self
                    .catalog
                    .load_table(&ident)
                    .await
                    .map_err(ApiError::internal)?;
                let temp = Path::new(self.catalog.root_path()).join("tmp");
                std::fs::create_dir_all(&temp).map_err(ApiError::internal)?;
                let mut files = Vec::new();
                let mut rows = 0u64;
                while let Some(batch) = stream
                    .try_next()
                    .await
                    .map_err(|e| ApiError::invalid(format!("{from}: {e}")))?
                {
                    if batch.num_rows() == 0 {
                        continue;
                    }
                    rows += batch.num_rows() as u64;
                    files.push(
                        write_data_file(
                            self.catalog.file_io(),
                            loaded.metadata(),
                            &schema,
                            &batch,
                            &temp,
                        )
                        .await
                        .map_err(ApiError::internal)?,
                    );
                }
                self.catalog
                    .replace_data_files(&ident, files)
                    .await
                    .map_err(ApiError::internal)?;
                Ok::<_, ApiError>(rows)
            }
            .await;
            if made.is_err() {
                let _ = self.catalog.drop_table(&ident).await;
            }
            made
        }
        .await;
        let _ = engine.detach(&scratch).await;
        written
    }

    async fn count_rows(&self, space: &str, table: &str) -> Result<u64, ApiError> {
        let sql = format!(
            "SELECT CAST(COUNT(*) AS BIGINT) FROM {}.{}",
            oikome_semantic::rewrite::quote_ident(space),
            oikome_semantic::rewrite::quote_ident(table)
        );
        let result = self.session(None).sql(&sql).await?;
        Ok(crate::file_analysis::first_count(&result))
    }

    #[allow(clippy::too_many_arguments)]
    fn keep_provenance(
        &self,
        space: &str,
        table: &str,
        from_space: &str,
        found: &inplace::DatasetTable,
        read: &datasets::DatasetRead,
        rows: u64,
        files: &[HashedFile],
    ) -> Result<(), ApiError> {
        let provenance = Provenance {
            collection: from_space,
            path: &found.path,
            kind: found.kind.name(),
            version: read.version.as_deref(),
            written_at_ms: read.written_at_ms,
            copied_at_ms: crate::now_ms(),
            rows,
            files: files
                .iter()
                .map(|f| ProvenanceFile {
                    path: &f.path,
                    size: f.size,
                    blake3: &f.blake3,
                })
                .collect(),
        };
        let dir = self.catalog.oikome_dir().join("copies").join(space);
        std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
        let file = dir.join(format!("{table}.json"));
        let tmp = file.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&provenance).map_err(ApiError::internal)?,
        )
        .map_err(ApiError::internal)?;
        std::fs::rename(&tmp, &file).map_err(ApiError::internal)
    }
}
