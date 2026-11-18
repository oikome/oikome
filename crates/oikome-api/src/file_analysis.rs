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

//! Data Analysis of a folder collection: each data file (CSV, TSV,
//! Parquet, JSON Lines), or each folder of them, becomes an analysis table
//! of the collection. The tables are kept in Iceberg under the collection's
//! name, carry the path of the file each row came from, and are made again
//! when their files change; the originals are only read. What each column
//! is for is proposed from its name and type, and the owner may correct it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use arrow_array::{Array, Float64Array, RecordBatch, StringArray};
use futures::TryStreamExt;
use iceberg::arrow::arrow_schema_to_schema_auto_assign_ids;
use iceberg::{Catalog, NamespaceIdent, TableCreation, TableIdent};
use oikome_catalog::Space;
use oikome_engine::{QualifiedName, TableSource};
use oikome_node::inplace::{self, DataFile};
use oikome_node::QueryResult;
use oikome_semantic::rewrite::quote_ident;
use oikome_storage::write_data_file;
use serde::{Deserialize, Serialize};

use crate::analysis::schema_profile;
use crate::{
    AnalysisPoint, AnalysisSource, AnalysisSummary, AnalysisTable, AnalysisTableRef, Api, ApiError,
    Caller, FieldRole, FileAnalysis, SetAnalysisField, SpaceRef,
};

/// The column every analysis table adds: the file a row came from.
pub(crate) const SOURCE_FILE: &str = "_source_file";
/// Where a table's files are attached while it is made.
const SCRATCH: &str = "__oikome_analysis";

/// What was made of a collection's files, kept beside the catalog.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Kept {
    tables: BTreeMap<String, KeptTable>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct KeptTable {
    files: Vec<KeptFile>,
    /// Roles the owner chose, by column.
    #[serde(default)]
    roles: BTreeMap<String, FieldRole>,
    built_at_ms: u64,
    rows: u64,
    #[serde(default)]
    problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KeptFile {
    path: String,
    size: u64,
    modified_ms: i64,
}

impl From<&DataFile> for KeptFile {
    fn from(file: &DataFile) -> Self {
        KeptFile {
            path: file.path.clone(),
            size: file.size,
            modified_ms: file.modified_ms,
        }
    }
}

/// The tables a collection's data files make: a top-level file on its own;
/// in a subfolder, the files of one format together, named after the
/// folder (a lone file after itself). Names are made unique.
fn sources_of(files: Vec<DataFile>) -> Vec<(String, Vec<DataFile>)> {
    let mut groups: BTreeMap<(String, String), Vec<DataFile>> = BTreeMap::new();
    for file in files {
        let folder = file
            .path
            .rsplit_once('/')
            .map(|(folder, _)| folder.to_string())
            .unwrap_or_default();
        let key = if folder.is_empty() {
            (file.path.clone(), String::new())
        } else {
            (folder, format!("{:?}", file.format))
        };
        groups.entry(key).or_default().push(file);
    }
    // Top-level files first: theirs are the names their file tables have.
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|((key, format), _)| (!format.is_empty(), key.clone()));
    let mut taken: Vec<String> = vec![inplace::FILES.to_string()];
    let mut out = Vec::new();
    for ((folder, _), files) in groups {
        let base = if files.len() == 1 {
            inplace::table_name(&files[0].path)
        } else {
            inplace::table_name(folder.rsplit('/').next().unwrap_or(&folder))
        };
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        taken.push(name.clone());
        out.push((name, files));
    }
    out
}

/// A string as a DuckDB literal: single quotes doubled (DuckDB reads no
/// backslash escapes in them).
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

impl Api {
    fn kept_path(&self, space: &str) -> PathBuf {
        self.catalog
            .oikome_dir()
            .join("analysis")
            .join(format!("{space}.json"))
    }

    fn kept(&self, space: &str) -> Result<Kept, ApiError> {
        match std::fs::read(self.kept_path(space)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(ApiError::internal),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Kept::default()),
            Err(e) => Err(ApiError::internal(e)),
        }
    }

    fn keep(&self, space: &str, kept: &Kept) -> Result<(), ApiError> {
        let path = self.kept_path(space);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(ApiError::internal)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(kept).map_err(ApiError::internal)?,
        )
        .map_err(ApiError::internal)?;
        std::fs::rename(&tmp, &path).map_err(ApiError::internal)
    }

    /// The folder collection `space`, its data files and what they make.
    async fn analysis_sources(
        &self,
        space: &str,
    ) -> Result<(Space, Vec<(String, Vec<DataFile>)>), ApiError> {
        let location = self.in_place_of(space)?;
        let space = self.catalog.get_space(space)?;
        let files = inplace::data_files(&self.catalog, &self.engines, &location).await?;
        Ok((space, sources_of(files)))
    }

    /// Read `files` into one table and keep it in Iceberg as
    /// `<space>.<name>`, replacing what was there. The row count.
    async fn make_table(
        &self,
        space: &Space,
        name: &str,
        files: &[DataFile],
    ) -> Result<u64, ApiError> {
        let engine = self.engines.engine_for(space)?;
        let mut selects = Vec::new();
        let mut attached = Vec::new();
        for (i, file) in files.iter().enumerate() {
            let local = file.local.clone().ok_or_else(|| {
                ApiError::invalid(format!("{} can't be read right now", file.path))
            })?;
            if file.path.chars().any(char::is_control) {
                return Err(ApiError::invalid(format!(
                    "{} has a name that can't be read",
                    file.path
                )));
            }
            let scratch = QualifiedName::new(SCRATCH, format!("{name}_{i}"));
            engine
                .attach(
                    &scratch,
                    TableSource::File {
                        path: local,
                        format: file.format,
                    },
                )
                .await
                .map_err(|e| ApiError::invalid(format!("{}: {e}", file.path)))?;
            selects.push(format!(
                "SELECT *, {} AS {} FROM {}",
                literal(&file.path),
                quote_ident(SOURCE_FILE),
                scratch.quoted()
            ));
            attached.push(scratch);
        }
        let sql = selects.join(" UNION ALL BY NAME ");
        let read = async {
            let (schema, stream) = engine.query(&sql, &[]).await?;
            let batches: Vec<RecordBatch> = stream.try_collect().await?;
            Ok::<_, oikome_engine::EngineError>((schema, batches))
        }
        .await;
        for scratch in &attached {
            let _ = engine.detach(scratch).await;
        }
        let (schema, batches) = read.map_err(|e| ApiError::invalid(e.to_string()))?;
        let rows = batches.iter().map(|b| b.num_rows() as u64).sum();

        let namespace = NamespaceIdent::new(space.name.clone());
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
        let ident = TableIdent::new(namespace.clone(), name.to_string());
        // Made again whole: its columns may have changed with its files.
        if self
            .catalog
            .table_exists(&ident)
            .await
            .map_err(ApiError::internal)?
        {
            self.catalog
                .drop_table(&ident)
                .await
                .map_err(ApiError::internal)?;
        }
        let iceberg_schema =
            arrow_schema_to_schema_auto_assign_ids(&schema).map_err(ApiError::internal)?;
        self.catalog
            .create_table(
                &namespace,
                TableCreation::builder()
                    .name(name.to_string())
                    .schema(iceberg_schema)
                    .build(),
            )
            .await
            .map_err(ApiError::internal)?;
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
        let mut written = Vec::new();
        for batch in batches.iter().filter(|b| b.num_rows() > 0) {
            written.push(
                write_data_file(self.catalog.file_io(), &location, &schema, batch, &temp)
                    .await
                    .map_err(ApiError::internal)?,
            );
        }
        self.catalog
            .replace_data_files(&ident, written)
            .await
            .map_err(ApiError::internal)?;
        Ok(rows)
    }

    async fn drop_analysis_table(&self, space: &str, name: &str) -> Result<(), ApiError> {
        let ident = TableIdent::new(NamespaceIdent::new(space.to_string()), name.to_string());
        if self
            .catalog
            .table_exists(&ident)
            .await
            .map_err(ApiError::internal)?
        {
            self.catalog
                .drop_table(&ident)
                .await
                .map_err(ApiError::internal)?;
        }
        Ok(())
    }

    /// Bring the tables made in line with the files: a table whose files
    /// changed is made again, one whose files are gone is dropped; with
    /// `make_new`, tables not made yet are made too.
    async fn follow_files(&self, space: &str, make_new: bool) -> Result<FileAnalysis, ApiError> {
        let (found, sources) = self.analysis_sources(space).await?;
        let mut kept = self.kept(space)?;
        let mut changed = false;
        for name in kept.tables.keys().cloned().collect::<Vec<_>>() {
            if !sources.iter().any(|(source, _)| *source == name) {
                self.drop_analysis_table(space, &name).await?;
                kept.tables.remove(&name);
                changed = true;
            }
        }
        for (name, files) in &sources {
            let now: Vec<KeptFile> = files.iter().map(KeptFile::from).collect();
            let entry = kept.tables.get(name);
            let due = match entry {
                Some(table) => table.files != now,
                None => make_new,
            };
            if !due {
                continue;
            }
            let roles = entry.map(|t| t.roles.clone()).unwrap_or_default();
            let made = self.make_table(&found, name, files).await;
            let table = match made {
                Ok(rows) => KeptTable {
                    files: now,
                    roles,
                    built_at_ms: crate::now_ms(),
                    rows,
                    problem: None,
                },
                Err(e) => KeptTable {
                    files: now,
                    roles,
                    built_at_ms: crate::now_ms(),
                    rows: 0,
                    problem: Some(e.to_string()),
                },
            };
            kept.tables.insert(name.clone(), table);
            changed = true;
        }
        if changed {
            self.keep(space, &kept)?;
        }
        let schemas: BTreeMap<String, arrow_schema::SchemaRef> = self
            .catalog
            .tables_in(space)
            .await?
            .into_iter()
            .map(|(name, resolved)| (name, resolved.schema))
            .collect();
        let tables = kept
            .tables
            .iter()
            .map(|(name, table)| {
                let fields = schemas
                    .get(name)
                    .map(|schema| {
                        schema_profile(space, name, schema)
                            .fields
                            .into_iter()
                            .map(|mut field| {
                                if field.name == SOURCE_FILE {
                                    field.role = FieldRole::Other;
                                } else if let Some(role) = table.roles.get(&field.name) {
                                    field.role = *role;
                                    field.inferred = false;
                                }
                                field
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                AnalysisTable {
                    name: name.clone(),
                    files: table.files.iter().map(|f| f.path.clone()).collect(),
                    rows: table.rows,
                    fields,
                    built_at_ms: table.built_at_ms,
                    problem: table.problem.clone(),
                }
            })
            .collect();
        Ok(FileAnalysis {
            space: space.to_string(),
            sources: sources
                .into_iter()
                .filter(|(name, _)| !kept.tables.contains_key(name))
                .map(|(name, files)| AnalysisSource {
                    name,
                    files: files.into_iter().map(|f| f.path).collect(),
                })
                .collect(),
            tables,
        })
    }

    /// What a folder collection's files can make for analysis, and the
    /// tables made, made again first where their files changed.
    pub async fn file_analysis(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.follow_files(&req.space, false).await
    }

    /// Make every analysis table a folder collection's files can make.
    pub async fn build_analysis(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.follow_files(&req.space, true).await
    }

    /// Correct what a column of an analysis table is for.
    pub async fn set_analysis_field(
        &self,
        caller: &Caller,
        req: SetAnalysisField,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let mut kept = self.kept(&req.space)?;
        let schema = self
            .catalog
            .tables_in(&req.space)
            .await?
            .into_iter()
            .find(|(name, _)| *name == req.table)
            .map(|(_, resolved)| resolved.schema)
            .ok_or_else(|| {
                ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
            })?;
        if req.field == SOURCE_FILE || schema.field_with_name(&req.field).is_err() {
            return Err(ApiError::invalid(format!(
                "{} has no column {}",
                req.table, req.field
            )));
        }
        let table = kept.tables.get_mut(&req.table).ok_or_else(|| {
            ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
        })?;
        table.roles.insert(req.field, req.role);
        self.keep(&req.space, &kept)?;
        self.follow_files(&req.space, false).await
    }

    /// An analysis table's totals by month and by category.
    pub async fn analysis_summary(
        &self,
        caller: &Caller,
        req: AnalysisTableRef,
    ) -> Result<AnalysisSummary, ApiError> {
        caller.require_owner()?;
        let analysis = self.follow_files(&req.space, false).await?;
        let table = analysis
            .tables
            .into_iter()
            .find(|t| t.name == req.table)
            .ok_or_else(|| {
                ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
            })?;
        let of = |role: FieldRole| {
            table
                .fields
                .iter()
                .find(|f| f.role == role)
                .map(|f| f.name.clone())
        };
        let (time, amount, category) = (
            of(FieldRole::Time),
            of(FieldRole::Amount),
            of(FieldRole::Category),
        );
        let from = format!("{}.{}", quote_ident(&req.space), quote_ident(&req.table));
        let value = amount
            .as_ref()
            .map(|a| format!("SUM(TRY_CAST({} AS DOUBLE))", quote_ident(a)))
            .unwrap_or_else(|| "CAST(COUNT(*) AS DOUBLE)".to_string());
        let session = self.session(None);
        let points = |result: QueryResult| -> Vec<AnalysisPoint> {
            let QueryResult::Batches { batches, .. } = result else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for batch in batches {
                let (Some(labels), Some(values)) = (
                    batch.column(0).as_any().downcast_ref::<StringArray>(),
                    batch.column(1).as_any().downcast_ref::<Float64Array>(),
                ) else {
                    continue;
                };
                for i in 0..batch.num_rows() {
                    if labels.is_valid(i) {
                        out.push(AnalysisPoint {
                            label: labels.value(i).to_string(),
                            value: if values.is_valid(i) {
                                values.value(i)
                            } else {
                                0.0
                            },
                        });
                    }
                }
            }
            out
        };
        let by_month = match &time {
            Some(t) => {
                let t = quote_ident(t);
                points(
                    session
                        .sql(&format!(
                            "SELECT strftime(date_trunc('month', TRY_CAST({t} AS TIMESTAMP)), '%Y-%m') AS label, \
                             {value} AS value FROM {from} WHERE TRY_CAST({t} AS TIMESTAMP) IS NOT NULL \
                             GROUP BY 1 ORDER BY 1 LIMIT 240"
                        ))
                        .await?,
                )
            }
            None => Vec::new(),
        };
        let by_category = match &category {
            Some(c) => {
                let c = quote_ident(c);
                points(
                    session
                        .sql(&format!(
                            "SELECT CAST({c} AS VARCHAR) AS label, {value} AS value FROM {from} \
                             WHERE {c} IS NOT NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 12"
                        ))
                        .await?,
                )
            }
            None => Vec::new(),
        };
        Ok(AnalysisSummary {
            table: table.name,
            time,
            amount,
            category,
            by_month,
            by_category,
        })
    }

    /// Forget what was made of a collection's files when it is removed.
    pub(crate) async fn forget_file_analysis(&self, space: &str) -> Result<(), ApiError> {
        let kept = self.kept(space)?;
        for name in kept.tables.keys() {
            self.drop_analysis_table(space, name).await?;
        }
        let namespace = NamespaceIdent::new(space.to_string());
        if self
            .catalog
            .namespace_exists(&namespace)
            .await
            .map_err(ApiError::internal)?
            && self
                .catalog
                .list_tables(&namespace)
                .await
                .map_err(ApiError::internal)?
                .is_empty()
        {
            let _ = self.catalog.drop_namespace(&namespace).await;
        }
        match std::fs::remove_file(self.kept_path(space)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(ApiError::internal(e)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oikome_engine::FileFormat;

    fn file(path: &str, format: FileFormat) -> DataFile {
        DataFile {
            path: path.into(),
            size: 1,
            modified_ms: 1,
            format,
            local: None,
        }
    }

    #[test]
    fn a_folder_of_files_is_one_table_and_a_top_file_its_own() {
        let sources = sources_of(vec![
            file("readings.csv", FileFormat::Csv),
            file("spending/2026-09.csv", FileFormat::Csv),
            file("spending/2026-10.csv", FileFormat::Csv),
            file("trips/2024/lisbon.parquet", FileFormat::Parquet),
            file("steps.jsonl", FileFormat::Json),
            file("other/readings.csv", FileFormat::Csv),
        ]);
        let names: Vec<(&str, usize)> =
            sources.iter().map(|(n, f)| (n.as_str(), f.len())).collect();
        assert_eq!(
            names,
            vec![
                ("readings", 1),
                ("steps", 1),
                ("readings_2", 1),
                ("spending", 2),
                ("lisbon", 1),
            ]
        );
    }

    #[test]
    fn literals_double_their_quotes() {
        assert_eq!(literal("it's.csv"), "'it''s.csv'");
    }
}
