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

//! Metadata readable through Oikome SQL and MCP, filtered as the caller's
//! queries are. Physical paths and source files are for the owner only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use oikome_catalog::{Location, LocationFormat};
use oikome_core::CatalogError;
use oikome_engine::TableSource;
use oikome_semantic::{DescribeRequest, ShowFilesRequest};
use serde::Deserialize;

use crate::error::SessionError;
use crate::result::QueryResult;
use crate::session::OikomeSession;

const DEFAULT_FILE_LIMIT: usize = 1000;
const MAX_FILE_LIMIT: usize = 10_000;

/// A collection the session may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionMetadata {
    /// Its name in SQL.
    pub name: String,
}

/// A column as the session and charts read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMetadata {
    /// Its name as the caller may read it.
    pub name: String,
    /// The table's physical type, named as DuckDB names it.
    pub data_type: String,
    /// What charts treat it as: text, integer, number, decimal, date,
    /// timestamp, boolean or other.
    pub chart_type: String,
    /// The effective DuckDB type when the owner chose another type.
    pub read_type: Option<String>,
    /// Whether the column may contain NULL.
    pub nullable: bool,
}

/// Physical facts about a table. Location and source files are absent for a
/// principal; the row count still follows that principal's row policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDetail {
    /// Where the owner reads it; never shown to a principal.
    pub location: Option<String>,
    /// Files an analysis table was made from; never shown to a principal.
    pub source_files: Option<Vec<String>>,
    /// Rows this caller may read, when the count could be obtained.
    pub rows: Option<u64>,
    /// When an analysis table was built, for the owner only.
    pub built_at_ms: Option<u64>,
}

/// A table or view the session may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableMetadata {
    /// Its collection's name.
    pub space: String,
    /// Its name within the collection.
    pub table: String,
    /// `managed`, `external`, `foreign` or `view`.
    pub table_type: String,
    /// Its data-source format, when known; a view has none.
    pub format: Option<String>,
    /// Columns the caller may read.
    pub columns: Vec<ColumnMetadata>,
    /// Physical details when `DESCRIBE DETAIL` asked for them.
    pub detail: Option<TableDetail>,
}

#[derive(Debug, Default, Deserialize)]
struct AnalysisFile {
    #[serde(default)]
    tables: BTreeMap<String, AnalysisTable>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct AnalysisTable {
    #[serde(default)]
    files: Vec<AnalysisSource>,
    #[serde(default)]
    types: BTreeMap<String, AnalysisType>,
    #[serde(default)]
    decimals: BTreeMap<String, (u8, u8)>,
    #[serde(default)]
    built_at_ms: Option<u64>,
    #[serde(default)]
    rows: Option<u64>,
    #[serde(default)]
    dataset: Option<AnalysisDataset>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnalysisSource {
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AnalysisDataset {
    kind: String,
    #[serde(default)]
    columns: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnalysisType {
    Text,
    Integer,
    Number,
    Decimal,
    Date,
    Timestamp,
    Boolean,
}

impl AnalysisType {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Decimal => "decimal",
            Self::Date => "date",
            Self::Timestamp => "timestamp",
            Self::Boolean => "boolean",
        }
    }

    fn sql(self, decimal: Option<(u8, u8)>) -> String {
        match self {
            Self::Text => "VARCHAR".into(),
            Self::Integer => "BIGINT".into(),
            Self::Number => "DOUBLE".into(),
            Self::Decimal => {
                let (precision, scale) = decimal.unwrap_or((18, 2));
                format!("DECIMAL({precision}, {scale})")
            }
            Self::Date => "DATE".into(),
            Self::Timestamp => "TIMESTAMP".into(),
            Self::Boolean => "BOOLEAN".into(),
        }
    }
}

struct DirectoryEntry {
    name: String,
    kind: String,
    format: Option<String>,
    size: u64,
    modified_ms: Option<i64>,
}

struct RelationContext {
    location: Location,
    analysis: AnalysisFile,
    sources: BTreeMap<String, TableSource>,
}

impl RelationContext {
    fn facts(&self, table: &str, view: bool) -> (String, Option<String>) {
        if view {
            return ("view".into(), None);
        }
        if let Some(analysis) = self.analysis.tables.get(table) {
            return if let Some(dataset) = &analysis.dataset {
                ("external".into(), Some(dataset.kind.clone()))
            } else {
                ("managed".into(), Some("iceberg".into()))
            };
        }
        match &self.location.format {
            LocationFormat::InPlace { .. } if table == crate::inplace::FILES => {
                ("managed".into(), Some("parquet".into()))
            }
            LocationFormat::InPlace { .. } => (
                "external".into(),
                self.sources.get(table).and_then(source_format),
            ),
            LocationFormat::Remote { .. } | LocationFormat::Replica { .. } => {
                ("foreign".into(), None)
            }
            LocationFormat::StorageCopy => ("managed".into(), None),
            other => ("managed".into(), Some(format_name(other))),
        }
    }
}

impl OikomeSession {
    /// The collections visible to this session, ordered by name.
    pub fn metadata_collections(&self) -> Vec<CollectionMetadata> {
        self.catalog
            .list_spaces()
            .into_iter()
            .filter(|space| {
                self.principal()
                    .is_none_or(|p| self.catalog.has_grants_in(p, &space.name))
            })
            .map(|space| CollectionMetadata { name: space.name })
            .collect()
    }

    /// `SHOW COLLECTIONS`.
    pub fn show_collections(&self) -> Result<QueryResult, SessionError> {
        let names: Vec<String> = self
            .metadata_collections()
            .into_iter()
            .map(|c| c.name)
            .collect();
        string_result("name", &names)
    }

    /// `SHOW TABLES [FROM <space>]`, using the default Space when omitted.
    pub async fn show_tables(&self, space: Option<&str>) -> Result<QueryResult, SessionError> {
        let space = self.metadata_space(space)?;
        let relations = self.readable_relations(&space).await?;
        let views: std::collections::BTreeSet<String> = self
            .catalog
            .list_views(&space)
            .into_iter()
            .map(|v| v.name)
            .collect();
        let context = self.relation_context(&space).await?;
        let facts: Vec<_> = relations
            .iter()
            .map(|name| context.facts(name, views.contains(name)))
            .collect();
        batch(
            vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("type", DataType::Utf8, false),
                Field::new("format", DataType::Utf8, true),
            ],
            vec![
                Arc::new(StringArray::from(relations)),
                strings(facts.iter().map(|fact| Some(fact.0.clone()))),
                strings(facts.iter().map(|fact| fact.1.clone())),
            ],
        )
    }

    /// `SHOW FILES`: one folder of a file collection, folders first.
    pub async fn show_files(
        &self,
        request: &ShowFilesRequest,
    ) -> Result<QueryResult, SessionError> {
        let space = self.metadata_space(request.space.as_deref())?;
        let relations = self.readable_relations(&space).await?;
        if !relations.iter().any(|name| name == crate::inplace::FILES) {
            return Err(SessionError::Unsupported(format!(
                "space '{space}' has no file index"
            )));
        }
        if let Some(principal) = self.principal() {
            let access =
                crate::principal_access(&self.catalog, principal, &space, crate::inplace::FILES)
                    .ok_or_else(|| CatalogError::TableNotFound(format!("{space}.files")))?;
            if access.columns.is_some()
                || self
                    .catalog
                    .policies_on(&space, crate::inplace::FILES)
                    .into_iter()
                    .any(|policy| policy.principals.iter().any(|p| p == principal))
            {
                return Err(SessionError::PermissionDenied(
                    "SHOW FILES needs an unrestricted files table".to_string(),
                ));
            }
        }
        let folder = request
            .folder
            .as_deref()
            .unwrap_or_default()
            .trim_matches('/')
            .to_string();
        if request
            .folder
            .as_deref()
            .is_some_and(|f| f.starts_with('/'))
            || folder.chars().any(char::is_control)
            || folder.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(CatalogError::InvalidProperty(
                "a files folder must be a relative path within the collection".into(),
            )
            .into());
        }
        let limit = request
            .limit
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_FILE_LIMIT);
        if limit == 0 || limit > MAX_FILE_LIMIT {
            return Err(CatalogError::InvalidProperty(format!(
                "SHOW FILES LIMIT may be 1 to {MAX_FILE_LIMIT}"
            ))
            .into());
        }
        let def = self.catalog.get_space(&space)?;
        let location = self.catalog.get_location(&def.location)?;
        let listing = crate::inplace::list(
            &self.catalog,
            &location,
            &folder,
            (crate::inplace::FileOrder::Name, false),
            0,
            limit as u64,
        )?;
        let mut entries = Vec::with_capacity(listing.folders.len() + listing.files.len());
        entries.extend(listing.folders.into_iter().map(|folder| DirectoryEntry {
            name: folder.name,
            kind: "folder".into(),
            format: None,
            size: folder.size,
            modified_ms: folder.modified_ms,
        }));
        entries.extend(listing.files.into_iter().map(|file| {
            let table = matches!(
                file.kind.as_str(),
                crate::inplace::TABLE | crate::inplace::DATASET
            );
            DirectoryEntry {
                name: file.name,
                kind: if table { "table".into() } else { "file".into() },
                format: if file.kind == crate::inplace::DATASET {
                    file.text.clone()
                } else {
                    file_format(&file.path)
                },
                size: file.size,
                modified_ms: Some(file.modified_ms),
            }
        }));
        entries.truncate(limit);
        batch(
            vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("kind", DataType::Utf8, false),
                Field::new("format", DataType::Utf8, true),
                Field::new("size", DataType::Int64, false),
                Field::new(
                    "modified",
                    DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None),
                    true,
                ),
            ],
            vec![
                strings(entries.iter().map(|entry| Some(entry.name.clone()))),
                strings(entries.iter().map(|entry| Some(entry.kind.clone()))),
                strings(entries.iter().map(|entry| entry.format.clone())),
                Arc::new(Int64Array::from(
                    entries
                        .iter()
                        .map(|entry| as_i64(entry.size))
                        .collect::<Vec<_>>(),
                )),
                Arc::new(TimestampMillisecondArray::from(
                    entries
                        .iter()
                        .map(|entry| entry.modified_ms)
                        .collect::<Vec<_>>(),
                )),
            ],
        )
    }

    /// Describe a table or view as this session may read it.
    pub async fn describe_metadata(
        &self,
        request: &DescribeRequest,
    ) -> Result<TableMetadata, SessionError> {
        let space = self.metadata_space(request.space.as_deref())?;
        let table = &request.table;
        if !self
            .readable_relations(&space)
            .await?
            .iter()
            .any(|name| name == table)
        {
            return Err(CatalogError::TableNotFound(format!("{space}.{table}")).into());
        }
        let context = self.relation_context(&space).await?;
        let analysis = context.analysis.tables.get(table).cloned();
        let qualified = format!(
            "{}.{}",
            oikome_semantic::rewrite::quote_ident(&space),
            oikome_semantic::rewrite::quote_ident(table)
        );
        let schema = match self
            .sql(&format!("SELECT * FROM {qualified} LIMIT 0"))
            .await
        {
            Ok(QueryResult::Batches { schema, .. }) => schema,
            Ok(_) => return Err(SessionError::Metadata("a table returned no schema".into())),
            Err(e) => match analysis_schema(analysis.as_ref()) {
                Some(schema) => schema,
                None => return Err(e),
            },
        };
        let schema = readable_schema(self, &space, table, schema)?;
        let columns = schema
            .fields()
            .iter()
            .map(|field| {
                let read_as = analysis.as_ref().and_then(|analysis| {
                    analysis
                        .types
                        .get(field.name())
                        .copied()
                        .filter(|kind| readable_as(field.data_type(), *kind))
                        .map(|kind| (kind, kind.sql(analysis.decimals.get(field.name()).copied())))
                });
                ColumnMetadata {
                    name: field.name().clone(),
                    data_type: duckdb_type(field.data_type()),
                    chart_type: read_as
                        .as_ref()
                        .map(|(kind, _)| kind.name())
                        .unwrap_or_else(|| chart_type(field.data_type()))
                        .to_string(),
                    read_type: read_as.map(|(_, sql)| sql),
                    nullable: field.is_nullable(),
                }
            })
            .collect();
        let (table_type, format) =
            context.facts(table, self.catalog.get_view(&space, table).is_some());
        let detail = if request.detail {
            Some(self.table_detail(&space, table, &qualified, analysis).await)
        } else {
            None
        };
        Ok(TableMetadata {
            table_type,
            format,
            space,
            table: table.clone(),
            columns,
            detail,
        })
    }

    /// `DESCRIBE [DETAIL]`, as Arrow rows for SQL and oiql.
    pub async fn describe_result(
        &self,
        request: &DescribeRequest,
    ) -> Result<QueryResult, SessionError> {
        let described = self.describe_metadata(request).await?;
        let Some(detail) = described.detail else {
            return batch(
                vec![
                    Field::new("name", DataType::Utf8, false),
                    Field::new("type", DataType::Utf8, false),
                    Field::new("chart_type", DataType::Utf8, false),
                    Field::new("read_type", DataType::Utf8, true),
                    Field::new("nullable", DataType::Boolean, false),
                ],
                vec![
                    strings(described.columns.iter().map(|c| Some(c.name.clone()))),
                    strings(described.columns.iter().map(|c| Some(c.data_type.clone()))),
                    strings(described.columns.iter().map(|c| Some(c.chart_type.clone()))),
                    strings(described.columns.iter().map(|c| c.read_type.clone())),
                    Arc::new(BooleanArray::from(
                        described
                            .columns
                            .iter()
                            .map(|c| c.nullable)
                            .collect::<Vec<_>>(),
                    )),
                ],
            );
        };
        let source_files = detail.source_files.map(|files| files.join("\n"));
        batch(
            vec![
                Field::new("collection", DataType::Utf8, false),
                Field::new("table", DataType::Utf8, false),
                Field::new("type", DataType::Utf8, false),
                Field::new("format", DataType::Utf8, true),
                Field::new("location", DataType::Utf8, true),
                Field::new("source_files", DataType::Utf8, true),
                Field::new("rows", DataType::Int64, true),
                Field::new("built_at_ms", DataType::Int64, true),
            ],
            vec![
                strings([Some(described.space)]),
                strings([Some(described.table)]),
                strings([Some(described.table_type)]),
                strings([described.format]),
                strings([detail.location]),
                strings([source_files]),
                Arc::new(Int64Array::from(vec![detail.rows.map(as_i64)])),
                Arc::new(Int64Array::from(vec![detail.built_at_ms.map(as_i64)])),
            ],
        )
    }

    fn metadata_space(&self, space: Option<&str>) -> Result<String, SessionError> {
        space
            .map(str::to_string)
            .or_else(|| self.default_space())
            .ok_or(SessionError::NoDefaultSpace)
    }

    fn analysis_file(&self, space: &str) -> Result<AnalysisFile, SessionError> {
        let path = self
            .catalog
            .oikome_dir()
            .join("analysis")
            .join(format!("{space}.json"));
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AnalysisFile::default())
            }
            Err(e) => return Err(SessionError::Metadata(e.to_string())),
        };
        serde_json::from_slice(&bytes).map_err(|e| SessionError::Metadata(e.to_string()))
    }

    async fn relation_context(&self, space: &str) -> Result<RelationContext, SessionError> {
        let def = self.catalog.get_space(space)?;
        let location = self.catalog.get_location(&def.location)?;
        let analysis = self.analysis_file(space)?;
        let sources = if matches!(location.format, LocationFormat::InPlace { .. }) {
            crate::inplace::bound(&self.catalog, self.engines(), &location)
                .await?
                .tables
                .into_iter()
                .collect()
        } else {
            BTreeMap::new()
        };
        Ok(RelationContext {
            location,
            analysis,
            sources,
        })
    }

    async fn table_detail(
        &self,
        space: &str,
        table: &str,
        qualified: &str,
        analysis: Option<AnalysisTable>,
    ) -> TableDetail {
        let def = self.catalog.get_space(space).ok();
        let location = def
            .as_ref()
            .and_then(|space| self.catalog.get_location(&space.location).ok());
        let owner = self.principal().is_none();
        let rows = match self.sql(&format!("SELECT count(*) FROM {qualified}")).await {
            Ok(QueryResult::Batches { batches, .. }) => first_count(&batches),
            _ if owner => analysis.as_ref().and_then(|analysis| analysis.rows),
            _ => None,
        };
        TableDetail {
            location: owner
                .then(|| location_path(self.catalog.root_path(), space, location.as_ref(), table)),
            source_files: if owner {
                analysis
                    .as_ref()
                    .map(|analysis| analysis.files.iter().map(|f| f.path.clone()).collect())
            } else {
                None
            },
            rows,
            built_at_ms: if owner {
                analysis.and_then(|analysis| analysis.built_at_ms)
            } else {
                None
            },
        }
    }
}

fn readable_schema(
    session: &OikomeSession,
    space: &str,
    table: &str,
    schema: SchemaRef,
) -> Result<SchemaRef, SessionError> {
    let Some(principal) = session.principal() else {
        return Ok(schema);
    };
    let access = crate::principal_access(&session.catalog, principal, space, table)
        .ok_or_else(|| CatalogError::TableNotFound(format!("{space}.{table}")))?;
    let Some(columns) = access.columns else {
        return Ok(schema);
    };
    Ok(Arc::new(Schema::new(
        schema
            .fields()
            .iter()
            .filter(|field| columns.contains(field.name()))
            .cloned()
            .collect::<Vec<_>>(),
    )))
}

fn analysis_schema(analysis: Option<&AnalysisTable>) -> Option<SchemaRef> {
    let columns = &analysis?.dataset.as_ref()?.columns;
    Some(Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, kind)| Field::new(name, kind.parse().unwrap_or(DataType::Utf8), true))
            .collect::<Vec<_>>(),
    )))
}

fn file_format(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| extension.to_ascii_lowercase())
}

fn source_format(source: &TableSource) -> Option<String> {
    match source {
        TableSource::File { path, .. } => path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase()),
        TableSource::ParquetFiles { .. } => Some("parquet".into()),
        TableSource::SqliteFile { .. } => Some("sqlite".into()),
        TableSource::ArrowBatches { .. } => Some("arrow".into()),
        TableSource::LanceDataset { .. } => Some("lance".into()),
        TableSource::IcebergTable { .. } => Some("iceberg".into()),
        TableSource::Unavailable { .. } => None,
    }
}

fn format_name(format: &LocationFormat) -> String {
    match format {
        LocationFormat::IcebergParquet { .. } => "iceberg".into(),
        LocationFormat::SqliteFile { .. } => "sqlite".into(),
        other => other.name().into(),
    }
}

fn location_path(root: &str, space: &str, location: Option<&Location>, table: &str) -> String {
    let Some(location) = location else {
        return format!("{space}.{table}");
    };
    match &location.format {
        LocationFormat::IcebergParquet { namespace } => Path::new(root)
            .join(namespace)
            .join(table)
            .display()
            .to_string(),
        LocationFormat::SqliteFile { path } => Path::new(root).join(path).display().to_string(),
        LocationFormat::Lance { root: folder } => Path::new(root)
            .join(folder)
            .join(format!("{table}.lance"))
            .display()
            .to_string(),
        LocationFormat::Remote { peer, space, owner } => {
            format!("{}:{owner}/{space}", peer.as_deref().unwrap_or("peer"))
        }
        LocationFormat::Replica { space, owner, .. } => format!("replica:{owner}/{space}"),
        LocationFormat::StorageCopy => Path::new(root)
            .join(".oikome/replicas")
            .join(&location.id)
            .display()
            .to_string(),
        LocationFormat::InPlace { path, place, .. } => place
            .as_ref()
            .map(|place| format!("{place}:{}", Path::new(path).display()))
            .unwrap_or_else(|| PathBuf::from(path).display().to_string()),
    }
}

fn duckdb_type(kind: &DataType) -> String {
    use DataType as T;
    match kind {
        T::Boolean => "BOOLEAN".into(),
        T::Int8 => "TINYINT".into(),
        T::Int16 => "SMALLINT".into(),
        T::Int32 => "INTEGER".into(),
        T::Int64 => "BIGINT".into(),
        T::UInt8 => "UTINYINT".into(),
        T::UInt16 => "USMALLINT".into(),
        T::UInt32 => "UINTEGER".into(),
        T::UInt64 => "UBIGINT".into(),
        T::Float16 | T::Float32 => "FLOAT".into(),
        T::Float64 => "DOUBLE".into(),
        T::Decimal128(precision, scale) | T::Decimal256(precision, scale) => {
            format!("DECIMAL({precision}, {scale})")
        }
        T::Utf8 | T::LargeUtf8 | T::Utf8View => "VARCHAR".into(),
        T::Date32 | T::Date64 => "DATE".into(),
        T::Timestamp(_, None) => "TIMESTAMP".into(),
        T::Timestamp(_, Some(_)) => "TIMESTAMPTZ".into(),
        T::Binary | T::LargeBinary | T::BinaryView => "BLOB".into(),
        other => other.to_string(),
    }
}

fn chart_type(kind: &DataType) -> &'static str {
    use DataType as T;
    match kind {
        T::Utf8 | T::LargeUtf8 | T::Utf8View => "text",
        T::Int8 | T::Int16 | T::Int32 | T::Int64 | T::UInt8 | T::UInt16 | T::UInt32 | T::UInt64 => {
            "integer"
        }
        T::Float16 | T::Float32 | T::Float64 => "number",
        T::Decimal128(_, _) | T::Decimal256(_, _) => "decimal",
        T::Date32 | T::Date64 => "date",
        T::Timestamp(_, _) => "timestamp",
        T::Boolean => "boolean",
        _ => "other",
    }
}

fn readable_as(from: &DataType, to: AnalysisType) -> bool {
    use AnalysisType as A;
    use DataType as T;
    match from {
        T::Utf8 | T::LargeUtf8 | T::Utf8View => true,
        T::Int8
        | T::Int16
        | T::Int32
        | T::Int64
        | T::UInt8
        | T::UInt16
        | T::UInt32
        | T::UInt64
        | T::Float16
        | T::Float32
        | T::Float64
        | T::Decimal128(_, _)
        | T::Decimal256(_, _) => matches!(to, A::Text | A::Integer | A::Number | A::Decimal),
        T::Date32 | T::Date64 | T::Timestamp(_, _) => {
            matches!(to, A::Text | A::Date | A::Timestamp)
        }
        T::Boolean => matches!(to, A::Text | A::Boolean | A::Integer),
        _ => to == A::Text,
    }
}

fn first_count(batches: &[RecordBatch]) -> Option<u64> {
    let batch = batches.iter().find(|batch| batch.num_rows() > 0)?;
    let column = batch.column(0);
    if let Some(values) = column.as_any().downcast_ref::<Int64Array>() {
        return u64::try_from(values.value(0)).ok();
    }
    column
        .as_any()
        .downcast_ref::<arrow_array::UInt64Array>()
        .map(|values| values.value(0))
}

fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn strings(values: impl IntoIterator<Item = Option<String>>) -> ArrayRef {
    Arc::new(StringArray::from(values.into_iter().collect::<Vec<_>>()))
}

fn string_result(name: &str, values: &[String]) -> Result<QueryResult, SessionError> {
    batch(
        vec![Field::new(name, DataType::Utf8, false)],
        vec![Arc::new(StringArray::from(values.to_vec()))],
    )
}

fn batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> Result<QueryResult, SessionError> {
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    Ok(QueryResult::Batches {
        schema,
        batches: vec![batch],
    })
}
