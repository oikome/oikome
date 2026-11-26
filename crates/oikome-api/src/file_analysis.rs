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
//! Parquet, JSON Lines), or each folder of them, and each sheet of a
//! spreadsheet (xlsx, xlsm, xls, xlsb, ods) becomes an analysis table of
//! the collection, when the owner chooses it. The tables are kept in
//! Iceberg under the collection's name, carry the path of the file each row
//! came from, and are made again when their files change; the originals are
//! only read. What each column is for is proposed from its name and type,
//! and the owner may correct it. The owner may remove a table: it is
//! dropped, and its files are read in place as before.
//!
//! A dataset in the collection's folders (a Lance dataset, an Iceberg
//! table; PR 31) is a table of it too, but it is not made: it is read where
//! it is, its latest version each time, and only its version, its columns
//! and its row count are kept here.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use iceberg::arrow::arrow_schema_to_schema_auto_assign_ids;
use iceberg::{Catalog, NamespaceIdent, TableCreation, TableIdent};
use oikome_catalog::Space;
use oikome_engine::{QualifiedName, TableSource};
use oikome_node::inplace::{self, DataFile, DatasetTable, SheetFile};
use oikome_node::QueryResult;
use oikome_semantic::rewrite::quote_ident;
use oikome_storage::write_data_file;
use serde::{Deserialize, Serialize};

use crate::analysis::schema_profile;
use crate::datasets::{capability_of, kind_of};
use crate::{
    AnalysisPoint, AnalysisSource, AnalysisSummary, AnalysisTable, AnalysisTableRef, AnalysisView,
    AnalysisViewRef, Api, ApiError, BuildAnalysis, Caller, ChartSpec, ColumnType, ErrorCode,
    FieldRole, FileAnalysis, SaveAnalysisView, SetAnalysisField, SetAnalysisType, SpaceRef,
};

/// The most views kept with a table, and the size of each.
const MAX_VIEWS: usize = 100;
const MAX_VIEW_BYTES: usize = 16 * 1024;
const MAX_VIEW_NAME_CHARS: usize = 80;
/// The id of the chart a table starts with. It is an ordinary chart, kept
/// with a blank name (the app names it for the reader) until renamed.
const DEFAULT_VIEW: &str = "default";

/// Whether a column of `from`'s type can be read as `to`: text as anything,
/// a number as text or another number, a date or a time as text or another
/// of them, true or false as text, itself or a whole number. A date read as
/// a decimal would only be empty.
pub(crate) fn readable_as(from: &arrow_schema::DataType, to: ColumnType) -> bool {
    use arrow_schema::DataType as T;
    use ColumnType as C;
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
        | T::Decimal256(_, _) => matches!(to, C::Text | C::Integer | C::Number | C::Decimal),
        T::Date32 | T::Date64 | T::Timestamp(_, _) => {
            matches!(to, C::Text | C::Date | C::Timestamp)
        }
        T::Boolean => matches!(to, C::Text | C::Boolean | C::Integer),
        _ => to == C::Text,
    }
}

/// A column read as another type: what it is, and that type as DuckDB names
/// it (`DECIMAL(18, 4)`), as charts read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadAs {
    pub kind: ColumnType,
    pub sql: String,
}

/// A type as DuckDB names it; a decimal with its digits and places.
fn sql_type(kind: ColumnType, decimal: Option<(u8, u8)>) -> String {
    match kind {
        ColumnType::Text => "VARCHAR".into(),
        ColumnType::Integer => "BIGINT".into(),
        ColumnType::Number => "DOUBLE".into(),
        ColumnType::Decimal => {
            let (digits, places) = decimal.unwrap_or((18, 2));
            format!("DECIMAL({digits}, {places})")
        }
        ColumnType::Date => "DATE".into(),
        ColumnType::Timestamp => "TIMESTAMP".into(),
        ColumnType::Boolean => "BOOLEAN".into(),
    }
}

/// The most digits a decimal has in DuckDB.
const MAX_DIGITS: u8 = 38;

/// The chart a table starts with, once: over its first column of time, by
/// month, as columns, when it has one; else its rows.
fn starting_view(schema: &arrow_schema::Schema) -> KeptView {
    let time = schema.fields().iter().find(|f| {
        f.name() != SOURCE_FILE
            && matches!(
                f.data_type(),
                arrow_schema::DataType::Date32
                    | arrow_schema::DataType::Date64
                    | arrow_schema::DataType::Timestamp(_, _)
            )
    });
    let config = match time {
        Some(time) => serde_json::json!({
            "chart": "column",
            "time": {"column": time.name(), "grain": "month", "range": "all"},
            "metrics": [{"aggregate": "count"}]
        }),
        None => serde_json::json!({"chart": "table"}),
    };
    KeptView {
        id: DEFAULT_VIEW.to_string(),
        name: String::new(),
        config,
        by: None,
        saved_at_ms: crate::now_ms(),
    }
}

fn shown_view(view: &KeptView) -> AnalysisView {
    AnalysisView {
        id: view.id.clone(),
        name: view.name.clone(),
        config: view.config.to_string(),
        by: view.by.clone(),
        saved_at_ms: view.saved_at_ms,
    }
}

/// A view's description as it is kept: a chart (its dimensions, metrics,
/// filters and the rest), and nothing else that was sent with it.
pub(crate) fn view_config(text: &str) -> Result<serde_json::Value, ApiError> {
    if text.len() > MAX_VIEW_BYTES {
        return Err(ApiError::invalid(format!(
            "a view is at most {} KiB",
            MAX_VIEW_BYTES / 1024
        )));
    }
    let chart: ChartSpec = serde_json::from_str(text)
        .map_err(|e| ApiError::invalid(format!("a view is a chart's description: {e}")))?;
    serde_json::to_value(chart).map_err(ApiError::internal)
}

/// The column every analysis table adds: the file a row came from.
pub(crate) const SOURCE_FILE: &str = "_source_file";
/// Where a table's files are attached while it is made.
const SCRATCH: &str = "__oikome_analysis";
/// A spreadsheet larger than this is not read: its sheets are read whole.
const MAX_BOOK_BYTES: u64 = 64 * 1024 * 1024;

/// What was made of a collection's files, kept beside the catalog.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Kept {
    tables: BTreeMap<String, KeptTable>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct KeptTable {
    files: Vec<KeptFile>,
    /// The sheet it was made of, for a spreadsheet's table (empty when the
    /// spreadsheet could not be read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet: Option<String>,
    /// Roles the owner chose, by column.
    #[serde(default)]
    roles: BTreeMap<String, FieldRole>,
    /// Types the owner has columns read as, by column.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    types: BTreeMap<String, ColumnType>,
    /// For a column read as a decimal: its digits and decimal places.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    decimals: BTreeMap<String, (u8, u8)>,
    built_at_ms: u64,
    rows: u64,
    #[serde(default)]
    problem: Option<String>,
    /// For a dataset's table: what it is and what was read of it last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dataset: Option<KeptDataset>,
    /// The views saved with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    views: Vec<KeptView>,
    /// Whether it was given the chart it starts with: once, so that one
    /// removed is not made again.
    #[serde(default)]
    started: bool,
}

/// A view saved with a table: its name and the explorer's description of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeptView {
    id: String,
    name: String,
    config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    by: Option<String>,
    saved_at_ms: u64,
}

/// What was read of a dataset last: it is read where it is, not made.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct KeptDataset {
    kind: String,
    version: Option<String>,
    written_at_ms: Option<i64>,
    /// Its columns, by name and type: shown while it can't be read.
    columns: Vec<(String, String)>,
    /// The DuckDB extension it needed that this computer lacked.
    #[serde(default)]
    needs: Option<String>,
}

impl KeptDataset {
    fn schema(&self) -> SchemaRef {
        Arc::new(Schema::new(
            self.columns
                .iter()
                .map(|(name, kind)| {
                    Field::new(
                        name,
                        DataType::from_str(kind).unwrap_or(DataType::Utf8),
                        true,
                    )
                })
                .collect::<Vec<_>>(),
        ))
    }
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

impl From<&SheetFile> for KeptFile {
    fn from(file: &SheetFile) -> Self {
        KeptFile {
            path: file.path.clone(),
            size: file.size,
            modified_ms: file.modified_ms,
        }
    }
}

/// What one analysis table is made of.
enum Input {
    /// Data files the engine reads: one, or a folder's of one format.
    Files(Vec<DataFile>),
    /// A sheet of a spreadsheet, with its rows when the spreadsheet was
    /// just read, or why it could not be (the sheet is then empty).
    Sheet {
        file: SheetFile,
        sheet: String,
        read: Option<Result<RecordBatch, String>>,
    },
    /// A dataset, read where it is.
    Dataset(DatasetTable),
}

impl Input {
    fn kept_files(&self) -> Vec<KeptFile> {
        match self {
            Input::Files(files) => files.iter().map(KeptFile::from).collect(),
            Input::Sheet { file, .. } => vec![KeptFile::from(file)],
            Input::Dataset(dataset) => vec![KeptFile {
                path: dataset.path.clone(),
                size: dataset.size,
                modified_ms: dataset.modified_ms,
            }],
        }
    }

    fn paths(&self) -> Vec<String> {
        match self {
            Input::Files(files) => files.iter().map(|f| f.path.clone()).collect(),
            Input::Sheet { file, .. } => vec![file.path.clone()],
            Input::Dataset(dataset) => vec![dataset.path.clone()],
        }
    }

    fn sheet(&self) -> Option<String> {
        match self {
            Input::Files(_) | Input::Dataset(_) => None,
            Input::Sheet { sheet, .. } => Some(sheet.clone()),
        }
    }
}

/// A spreadsheet and its sheets, each with what reading it gave.
type Book = (
    SheetFile,
    Vec<(String, Option<Result<RecordBatch, String>>)>,
);

/// A sheet's name for its table, or none (`None` when it is not shown).
fn shown_sheet(sheet: Option<&String>) -> Option<String> {
    sheet.filter(|s| !s.is_empty()).cloned()
}

/// The tables a collection's data files and spreadsheets make: a top-level
/// file on its own; in a subfolder, the files of one format together,
/// named after the folder (a lone file after itself); a spreadsheet's
/// sheet, named after the file (`<file>_<sheet>` when it has several).
/// Names are made unique; data files are named first. Datasets come with
/// their names (the node's, which their tables have in SQL too).
fn sources_of(
    files: Vec<DataFile>,
    books: Vec<Book>,
    datasets: Vec<DatasetTable>,
) -> Vec<(String, Input)> {
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
    taken.extend(datasets.iter().map(|d| d.name.clone()));
    let mut unique = |base: String| {
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        taken.push(name.clone());
        name
    };
    let mut out = Vec::new();
    for ((folder, _), files) in groups {
        let base = if files.len() == 1 {
            inplace::table_name(&files[0].path)
        } else {
            inplace::table_name(folder.rsplit('/').next().unwrap_or(&folder))
        };
        out.push((unique(base), Input::Files(files)));
    }
    for (file, sheets) in books {
        let several = sheets.len() > 1;
        let stem = inplace::table_name(&file.path);
        for (sheet, read) in sheets {
            let base = if several {
                inplace::name_for_table(&format!("{stem}_{sheet}"))
            } else {
                stem.clone()
            };
            let input = Input::Sheet {
                file: file.clone(),
                sheet,
                read,
            };
            out.push((unique(base), input));
        }
    }
    for dataset in datasets {
        out.push((dataset.name.clone(), Input::Dataset(dataset)));
    }
    out
}

/// The single number a `COUNT(*)` gave.
pub(crate) fn first_count(result: &QueryResult) -> u64 {
    let QueryResult::Batches { batches, .. } = result else {
        return 0;
    };
    batches
        .iter()
        .find(|b| b.num_rows() > 0)
        .and_then(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .map(|a| a.value(0))
        })
        .and_then(|n| u64::try_from(n).ok())
        .unwrap_or(0)
}

/// `batch` with the file its rows came from as a last column.
fn with_source(batch: RecordBatch, path: &str) -> Result<(SchemaRef, RecordBatch), ApiError> {
    let mut fields: Vec<Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect();
    fields.push(Field::new(SOURCE_FILE, arrow_schema::DataType::Utf8, false));
    let schema = Arc::new(Schema::new(fields));
    let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
    columns.push(Arc::new(StringArray::from(vec![path; batch.num_rows()])));
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(ApiError::internal)?;
    Ok((schema, batch))
}

/// Every sheet of the spreadsheet `file` worth a table, read now.
async fn read_book(file: &SheetFile) -> Result<Vec<(String, RecordBatch)>, String> {
    let Some(local) = file.local.clone() else {
        return Err(format!("{} can't be read right now", file.path));
    };
    if file.size > MAX_BOOK_BYTES {
        return Err(format!(
            "{} is larger than {} MB, the most a spreadsheet read for analysis may be",
            file.path,
            MAX_BOOK_BYTES / 1024 / 1024
        ));
    }
    tokio::task::spawn_blocking(move || oikome_connect::sheets::read(&local))
        .await
        .map_err(|e| e.to_string())?
        .map(|sheets| sheets.into_iter().map(|s| (s.name, s.batch)).collect())
        .map_err(|e| e.to_string())
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

    /// The folder collection `space`, its data files and spreadsheets and
    /// what they make. A spreadsheet is read only when it is new or changed
    /// since its tables were made; otherwise their sheets are as kept.
    async fn analysis_sources(
        &self,
        space: &str,
        kept: &Kept,
    ) -> Result<(Space, Vec<(String, Input)>), ApiError> {
        let location = self.in_place_of(space)?;
        let space = self.catalog.get_space(space)?;
        let files = inplace::data_files(&self.catalog, &self.engines, &location).await?;
        let mut books = Vec::new();
        for file in inplace::sheet_files(&self.catalog, &self.engines, &location).await? {
            let now = vec![KeptFile::from(&file)];
            let known: Vec<_> = kept
                .tables
                .values()
                .filter(|table| table.files == now)
                .filter_map(|table| table.sheet.clone())
                .map(|sheet| (sheet, None))
                .collect();
            if !known.is_empty() {
                books.push((file, known));
                continue;
            }
            match read_book(&file).await {
                // Nothing in it to analyze.
                Ok(sheets) if sheets.is_empty() => {}
                Ok(sheets) => {
                    let sheets = sheets
                        .into_iter()
                        .map(|(sheet, batch)| (sheet, Some(Ok(batch))))
                        .collect();
                    books.push((file, sheets));
                }
                Err(e) => books.push((file, vec![(String::new(), Some(Err(e)))])),
            }
        }
        let datasets = inplace::dataset_tables(&self.catalog, &location)?;
        Ok((space, sources_of(files, books, datasets)))
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
        self.write_table(space, name, &schema, &batches).await
    }

    /// A sheet of `file` as the table `name`: from the rows just read, or
    /// read again.
    async fn make_sheet_table(
        &self,
        space: &Space,
        name: &str,
        file: &SheetFile,
        sheet: &str,
        read: Option<Result<RecordBatch, String>>,
    ) -> Result<u64, ApiError> {
        let batch = match read {
            Some(read) => read.map_err(ApiError::invalid)?,
            None => read_book(file)
                .await
                .map_err(ApiError::invalid)?
                .into_iter()
                .find(|(name, _)| name == sheet)
                .map(|(_, batch)| batch)
                .ok_or_else(|| {
                    ApiError::invalid(format!(
                        "{} has no sheet {sheet} with anything in it",
                        file.path
                    ))
                })?,
        };
        let (schema, batch) = with_source(batch, &file.path)?;
        self.write_table(space, name, &schema, &[batch]).await
    }

    /// Keep `batches` in Iceberg as `<space>.<name>`, replacing what was
    /// there. The row count.
    async fn write_table(
        &self,
        space: &Space,
        name: &str,
        schema: &SchemaRef,
        batches: &[RecordBatch],
    ) -> Result<u64, ApiError> {
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
            arrow_schema_to_schema_auto_assign_ids(schema).map_err(ApiError::internal)?;
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
        let loaded = self
            .catalog
            .load_table(&ident)
            .await
            .map_err(ApiError::internal)?;
        let temp = Path::new(self.catalog.root_path()).join("tmp");
        std::fs::create_dir_all(&temp).map_err(ApiError::internal)?;
        let mut written = Vec::new();
        for batch in batches.iter().filter(|b| b.num_rows() > 0) {
            written.push(
                write_data_file(
                    self.catalog.file_io(),
                    loaded.metadata(),
                    schema,
                    batch,
                    &temp,
                )
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

    /// A dataset's table as it is now: its version, columns and rows, read
    /// where it is; or why it can't be read, its columns as last read.
    async fn read_dataset_table(
        &self,
        space: &Space,
        name: &str,
        dataset: &DatasetTable,
        was: Option<&KeptTable>,
    ) -> KeptTable {
        let mut facts = KeptDataset {
            kind: dataset.kind.name().to_string(),
            columns: was
                .and_then(|t| t.dataset.as_ref())
                .map(|d| d.columns.clone())
                .unwrap_or_default(),
            ..Default::default()
        };
        let read = self
            .read_dataset_now(space, name, dataset, &mut facts)
            .await;
        let (rows, problem) = match read {
            Ok(rows) => (rows, None),
            Err(e) => (0, Some(e.message)),
        };
        KeptTable {
            files: vec![KeptFile {
                path: dataset.path.clone(),
                size: dataset.size,
                modified_ms: dataset.modified_ms,
            }],
            sheet: None,
            roles: was.map(|t| t.roles.clone()).unwrap_or_default(),
            types: was.map(|t| t.types.clone()).unwrap_or_default(),
            decimals: was.map(|t| t.decimals.clone()).unwrap_or_default(),
            built_at_ms: crate::now_ms(),
            rows,
            problem,
            dataset: Some(facts),
            views: was.map(|t| t.views.clone()).unwrap_or_default(),
            started: was.is_some_and(|t| t.started),
        }
    }

    async fn read_dataset_now(
        &self,
        space: &Space,
        name: &str,
        dataset: &DatasetTable,
        facts: &mut KeptDataset,
    ) -> Result<u64, ApiError> {
        let engine = self.engines.engine_for(space)?;
        if engine.enable(capability_of(dataset.kind)).await.is_err() {
            facts.needs = Some(dataset.kind.extension().to_string());
            return Err(ApiError::invalid(format!(
                "reading a {} dataset needs DuckDB's {} extension, which this computer does not have yet",
                if dataset.kind == oikome_node::datasets::DatasetKind::Lance { "Lance" } else { "Iceberg" },
                dataset.kind.extension()
            )));
        }
        let location = self.in_place_of(&space.name)?;
        let read = inplace::read_dataset(&self.catalog, &self.engines, &location, &dataset.path)
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?;
        facts.version = read.version;
        facts.written_at_ms = read.written_at_ms;
        let from = format!("{}.{}", quote_ident(&space.name), quote_ident(name));
        let session = self.session(None);
        if let QueryResult::Batches { schema, .. } = session
            .sql(&format!("SELECT * FROM {from} LIMIT 0"))
            .await?
        {
            facts.columns = schema
                .fields()
                .iter()
                .map(|f| (f.name().clone(), f.data_type().to_string()))
                .collect();
        }
        let counted = session
            .sql(&format!("SELECT CAST(COUNT(*) AS BIGINT) FROM {from}"))
            .await?;
        Ok(first_count(&counted))
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
    /// changed is made again, one whose files are gone is dropped; the
    /// tables named in `make` are made too, when they are not yet.
    async fn follow_files(&self, space: &str, make: &[String]) -> Result<FileAnalysis, ApiError> {
        let mut kept = self.kept(space)?;
        let (found, sources) = self.analysis_sources(space, &kept).await?;
        if let Some(name) = make
            .iter()
            .find(|name| !sources.iter().any(|(source, _)| source == *name))
        {
            return Err(ApiError::invalid(format!(
                "{space} has no files that make a table {name}"
            )));
        }
        let mut changed = false;
        for name in kept.tables.keys().cloned().collect::<Vec<_>>() {
            if !sources.iter().any(|(source, _)| *source == name) {
                self.drop_analysis_table(space, &name).await?;
                kept.tables.remove(&name);
                changed = true;
            }
        }
        let mut listed = Vec::new();
        for (name, input) in sources {
            let now = input.kept_files();
            let sheet = input.sheet();
            let dataset = match &input {
                Input::Dataset(d) => Some(kind_of(d.kind)),
                _ => None,
            };
            listed.push((name.clone(), input.paths(), sheet.clone(), dataset));
            let entry = kept.tables.get(&name);
            let due = match entry {
                // A dataset that could not be read is tried again.
                Some(table) => {
                    table.files != now || (table.dataset.is_some() && table.problem.is_some())
                }
                // A dataset is read where it is, nothing made from it, so it
                // is an analysis table as soon as it is found; the owner
                // makes the tables of files.
                None => make.contains(&name) || matches!(input, Input::Dataset(_)),
            };
            if !due {
                continue;
            }
            let roles = entry.map(|t| t.roles.clone()).unwrap_or_default();
            let types = entry.map(|t| t.types.clone()).unwrap_or_default();
            let decimals = entry.map(|t| t.decimals.clone()).unwrap_or_default();
            let views = entry.map(|t| t.views.clone()).unwrap_or_default();
            let started = entry.is_some_and(|t| t.started);
            if let Input::Dataset(dataset) = &input {
                let table = self.read_dataset_table(&found, &name, dataset, entry).await;
                kept.tables.insert(name, table);
                changed = true;
                continue;
            }
            let made = match input {
                Input::Files(files) => self.make_table(&found, &name, &files).await,
                Input::Sheet { file, sheet, read } => {
                    self.make_sheet_table(&found, &name, &file, &sheet, read)
                        .await
                }
                Input::Dataset(_) => unreachable!("read above"),
            };
            let table = match made {
                Ok(rows) => KeptTable {
                    files: now,
                    sheet,
                    roles,
                    types: types.clone(),
                    decimals: decimals.clone(),
                    built_at_ms: crate::now_ms(),
                    rows,
                    problem: None,
                    dataset: None,
                    views,
                    started,
                },
                Err(e) => KeptTable {
                    files: now,
                    sheet,
                    roles,
                    types: types.clone(),
                    decimals: decimals.clone(),
                    built_at_ms: crate::now_ms(),
                    rows: 0,
                    problem: Some(e.to_string()),
                    dataset: None,
                    views,
                    started,
                },
            };
            kept.tables.insert(name, table);
            changed = true;
        }
        if changed {
            self.keep(space, &kept)?;
        }
        let mut schemas: BTreeMap<String, arrow_schema::SchemaRef> = self
            .catalog
            .tables_in(space)
            .await?
            .into_iter()
            .map(|(name, resolved)| (name, resolved.schema))
            .collect();
        for (name, table) in &kept.tables {
            if let Some(dataset) = &table.dataset {
                schemas.insert(name.clone(), dataset.schema());
            }
        }
        // A table starts with a chart, once. Renamed, changed or removed
        // like any other, it is not made again.
        let mut started = false;
        for (name, table) in kept.tables.iter_mut() {
            if table.started || table.problem.is_some() {
                continue;
            }
            let Some(schema) = schemas.get(name) else {
                continue;
            };
            if !table.views.iter().any(|v| v.id == DEFAULT_VIEW) {
                table.views.insert(0, starting_view(schema));
            }
            table.started = true;
            started = true;
        }
        if started {
            self.keep(space, &kept)?;
        }
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
                                field.read_as = table.types.get(&field.name).copied().filter(|t| {
                                    schema
                                        .field_with_name(&field.name)
                                        .is_ok_and(|f| readable_as(f.data_type(), *t))
                                });
                                field.read_type = field.read_as.map(|kind| {
                                    sql_type(kind, table.decimals.get(&field.name).copied())
                                });
                                field
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                AnalysisTable {
                    name: name.clone(),
                    files: table.files.iter().map(|f| f.path.clone()).collect(),
                    sheet: shown_sheet(table.sheet.as_ref()),
                    rows: table.rows,
                    fields,
                    built_at_ms: table.built_at_ms,
                    problem: table.problem.clone(),
                    dataset: table.dataset.as_ref().and_then(|d| {
                        let kind = oikome_node::datasets::DatasetKind::from_name(&d.kind)?;
                        Some(crate::DatasetFacts {
                            kind: kind_of(kind),
                            version: d.version.clone(),
                            written_at_ms: d.written_at_ms,
                            extension: d.needs.as_deref().map(|n| self.extension_download(n)),
                        })
                    }),
                    views: table.views.iter().map(shown_view).collect(),
                }
            })
            .collect();
        Ok(FileAnalysis {
            space: space.to_string(),
            sources: listed
                .into_iter()
                .filter(|(name, _, _, _)| !kept.tables.contains_key(name))
                .map(|(name, files, sheet, dataset)| AnalysisSource {
                    name,
                    files,
                    sheet: shown_sheet(sheet.as_ref()),
                    dataset,
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
        self.follow_files(&req.space, &[]).await
    }

    /// Make the analysis tables the owner chose of a folder collection's
    /// files, by the names `file_analysis` gives them.
    pub async fn build_analysis(
        &self,
        caller: &Caller,
        req: BuildAnalysis,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        if req.tables.is_empty() {
            return Err(ApiError::invalid("choose the tables to make"));
        }
        self.follow_files(&req.space, &req.tables).await
    }

    /// Remove an analysis table made of files: it is dropped with what was
    /// set for its columns, and it is not made again until the owner
    /// chooses it. Its files are not touched; SQL reads them in place again.
    /// A dataset's table is read where it is and is always analysed.
    pub async fn remove_analysis_table(
        &self,
        caller: &Caller,
        req: AnalysisTableRef,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        // A collection that views a folder: its name is checked before it
        // names the file its tables are kept in.
        self.in_place_of(&req.space)?;
        let mut kept = self.kept(&req.space)?;
        let table = kept.tables.get(&req.table).ok_or_else(|| {
            ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
        })?;
        if table.dataset.is_some() {
            return Err(ApiError::invalid(format!(
                "{} is a dataset, read where it is; it is always analysed",
                req.table
            )));
        }
        self.drop_analysis_table(&req.space, &req.table).await?;
        kept.tables.remove(&req.table);
        self.keep(&req.space, &kept)?;
        self.follow_files(&req.space, &[]).await
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
        let of_dataset = kept
            .tables
            .get(&req.table)
            .and_then(|t| t.dataset.as_ref())
            .map(KeptDataset::schema);
        let schema = match of_dataset {
            Some(schema) => Some(schema),
            None => self
                .catalog
                .tables_in(&req.space)
                .await?
                .into_iter()
                .find(|(name, _)| *name == req.table)
                .map(|(_, resolved)| resolved.schema),
        }
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
        self.follow_files(&req.space, &[]).await
    }

    /// Read a column of an analysis table as another type, or as DuckDB
    /// read it again. Only how charts read it changes: not the table, not
    /// its files.
    pub async fn set_analysis_type(
        &self,
        caller: &Caller,
        req: SetAnalysisType,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (schema, _) = self.analysis_columns(&req.space, &req.table).await?;
        let field = schema
            .field_with_name(&req.column)
            .ok()
            .filter(|_| req.column != SOURCE_FILE)
            .ok_or_else(|| {
                ApiError::invalid(format!("{} has no column {}", req.table, req.column))
            })?;
        if let Some(read_as) = req.read_as {
            if !readable_as(field.data_type(), read_as) {
                return Err(ApiError::invalid(format!(
                    "{} is {}: it cannot be read as {read_as:?}",
                    req.column,
                    field.data_type()
                )));
            }
        }
        // A decimal's digits and places, as chosen, within DuckDB's limits;
        // its digits, unless chosen, 18 or as many as its largest value needs.
        // A choice its largest value does not fit is refused: its values
        // would only be empty.
        let decimal = match req.read_as {
            Some(ColumnType::Decimal) => {
                let places = req.scale.unwrap_or(2);
                if let Some(digits) = req.precision {
                    if !(1..=MAX_DIGITS).contains(&digits) {
                        return Err(ApiError::invalid(format!(
                            "a decimal has 1 to {MAX_DIGITS} digits"
                        )));
                    }
                }
                if places > req.precision.unwrap_or(MAX_DIGITS) {
                    return Err(ApiError::invalid(
                        "a decimal has no more places than digits",
                    ));
                }
                let largest = self
                    .session(None)
                    .sql(&format!(
                        "SELECT max(abs(TRY_CAST({} AS DOUBLE))) FROM {}.{}",
                        quote_ident(&req.column),
                        quote_ident(&req.space),
                        quote_ident(&req.table)
                    ))
                    .await
                    .ok()
                    .and_then(|result| match result {
                        QueryResult::Batches { batches, .. } => {
                            batches.into_iter().find(|b| b.num_rows() > 0)
                        }
                        _ => None,
                    })
                    .and_then(|batch| {
                        batch
                            .column(0)
                            .as_any()
                            .downcast_ref::<Float64Array>()
                            .filter(|c| c.is_valid(0))
                            .map(|c| c.value(0))
                    })
                    .unwrap_or(0.0);
                let fits = |digits: u8| largest < 10f64.powi(i32::from(digits) - i32::from(places));
                let digits = match req.precision {
                    Some(digits) if fits(digits) => digits,
                    Some(digits) => {
                        return Err(ApiError::invalid(format!(
                            "{} reaches {largest}: it does not fit DECIMAL({digits}, {places})",
                            req.column
                        )))
                    }
                    None if places <= 18 && fits(18) => 18,
                    None => MAX_DIGITS,
                };
                if !fits(digits) {
                    return Err(ApiError::invalid(format!(
                        "{} reaches {largest}: it does not fit DECIMAL({digits}, {places})",
                        req.column
                    )));
                }
                Some((digits, places))
            }
            _ => None,
        };
        let mut kept = self.kept(&req.space)?;
        let table = kept.tables.get_mut(&req.table).ok_or_else(|| {
            ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
        })?;
        match req.read_as {
            Some(read_as) => table.types.insert(req.column.clone(), read_as),
            None => table.types.remove(&req.column),
        };
        match decimal {
            Some(decimal) => table.decimals.insert(req.column, decimal),
            None => table.decimals.remove(&req.column),
        };
        self.keep(&req.space, &kept)?;
        self.follow_files(&req.space, &[]).await
    }

    /// The types the owner has columns of an analysis table read as, those
    /// a column can be read as.
    pub(crate) async fn analysis_types(
        &self,
        space: &str,
        table: &str,
    ) -> Result<BTreeMap<String, ReadAs>, ApiError> {
        let (schema, _) = self.analysis_columns(space, table).await?;
        let kept = self.kept(space)?;
        let Some(entry) = kept.tables.get(table) else {
            return Ok(BTreeMap::new());
        };
        Ok(entry
            .types
            .iter()
            .filter(|(column, read_as)| {
                schema
                    .field_with_name(column)
                    .is_ok_and(|f| readable_as(f.data_type(), **read_as))
            })
            .map(|(column, kind)| {
                let sql = sql_type(*kind, entry.decimals.get(column).copied());
                (column.clone(), ReadAs { kind: *kind, sql })
            })
            .collect())
    }

    /// An analysis table's columns and what each is for (as proposed, or as
    /// the owner corrected), without looking at its files again.
    pub(crate) async fn analysis_columns(
        &self,
        space: &str,
        table: &str,
    ) -> Result<(SchemaRef, Vec<(String, FieldRole)>), ApiError> {
        self.in_place_of(space)?;
        let kept = self.kept(space)?;
        let missing = || ApiError::not_found(format!("{space} has no analysis table {table}"));
        let entry = kept.tables.get(table).ok_or_else(missing)?;
        let schema = match &entry.dataset {
            Some(dataset) => dataset.schema(),
            None => self
                .catalog
                .tables_in(space)
                .await?
                .into_iter()
                .find(|(name, _)| name == table)
                .map(|(_, resolved)| resolved.schema)
                .ok_or_else(missing)?,
        };
        let roles = schema_profile(space, table, &schema)
            .fields
            .into_iter()
            .map(|field| {
                let role = entry.roles.get(&field.name).copied().unwrap_or(field.role);
                (field.name, role)
            })
            .collect();
        Ok((schema, roles))
    }

    /// Keep a view with its analysis table: a new one, or one saved before
    /// (by its id) under a new name or with a new description.
    pub async fn save_analysis_view(
        &self,
        caller: &Caller,
        req: SaveAnalysisView,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.in_place_of(&req.space)?;
        self.keep_view(
            &req.space,
            &req.table,
            req.id.as_deref(),
            &req.name,
            &req.config,
            None,
        )?;
        self.follow_files(&req.space, &[]).await
    }

    /// A collection's analysis tables as last made: each one's name, rows
    /// and saved charts, those that could not be read left out. Nothing is
    /// made again here: what an assistant lists is what the owner has.
    pub(crate) fn analysis_kept(
        &self,
        space: &str,
    ) -> Result<Vec<(String, u64, Vec<AnalysisView>)>, ApiError> {
        Ok(self
            .kept(space)?
            .tables
            .into_iter()
            .filter(|(_, table)| table.problem.is_none())
            .map(|(name, table)| {
                (
                    name,
                    table.rows,
                    table.views.iter().map(shown_view).collect(),
                )
            })
            .collect())
    }

    /// Keep a view; `by` names the assistant that made it.
    pub(crate) fn keep_view(
        &self,
        space: &str,
        table: &str,
        id: Option<&str>,
        name: &str,
        config: &str,
        by: Option<&str>,
    ) -> Result<AnalysisView, ApiError> {
        let name = name.trim();
        let config = view_config(config)?;
        let mut kept = self.kept(space)?;
        let entry = kept
            .tables
            .get_mut(table)
            .ok_or_else(|| ApiError::not_found(format!("{space} has no analysis table {table}")))?;
        // The chart a table starts with keeps its blank name until renamed.
        let blank =
            id.is_some_and(|id| entry.views.iter().any(|v| v.id == id && v.name.is_empty()));
        if (name.is_empty() && !blank) || name.chars().count() > MAX_VIEW_NAME_CHARS {
            return Err(ApiError::invalid(format!(
                "a view's name has 1 to {MAX_VIEW_NAME_CHARS} characters"
            )));
        }
        if entry
            .views
            .iter()
            .any(|v| v.name == name && Some(v.id.as_str()) != id)
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!("{table} already has a view named {name}"),
            ));
        }
        let saved = match id {
            Some(id) if entry.views.iter().any(|v| v.id == id) => {
                let view = entry
                    .views
                    .iter_mut()
                    .find(|v| v.id == id)
                    .ok_or_else(|| ApiError::not_found(format!("{table} has no view {id}")))?;
                view.name = name.to_string();
                view.config = config;
                // Saved again by the owner, it is the owner's now.
                view.by = by.map(str::to_string);
                view.saved_at_ms = crate::now_ms();
                view.clone()
            }
            Some(id) => return Err(ApiError::not_found(format!("{table} has no view {id}"))),
            None => {
                if entry.views.len() >= MAX_VIEWS {
                    return Err(ApiError::invalid(format!(
                        "a table keeps at most {MAX_VIEWS} views"
                    )));
                }
                let mut n = entry.views.len() + 1;
                while entry.views.iter().any(|v| v.id == format!("v{n}")) {
                    n += 1;
                }
                let view = KeptView {
                    id: format!("v{n}"),
                    name: name.to_string(),
                    config,
                    by: by.map(str::to_string),
                    saved_at_ms: crate::now_ms(),
                };
                entry.views.push(view.clone());
                view
            }
        };
        self.keep(space, &kept)?;
        Ok(shown_view(&saved))
    }

    /// Remove a saved view; its table and the table's files stay.
    pub async fn remove_analysis_view(
        &self,
        caller: &Caller,
        req: AnalysisViewRef,
    ) -> Result<FileAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        self.in_place_of(&req.space)?;
        let mut kept = self.kept(&req.space)?;
        let entry = kept.tables.get_mut(&req.table).ok_or_else(|| {
            ApiError::not_found(format!("{} has no analysis table {}", req.space, req.table))
        })?;
        let before = entry.views.len();
        entry.views.retain(|v| v.id != req.id);
        if entry.views.len() == before {
            return Err(ApiError::not_found(format!(
                "{} has no view {}",
                req.table, req.id
            )));
        }
        self.keep(&req.space, &kept)?;
        self.follow_files(&req.space, &[]).await
    }

    /// An analysis table's totals by month and by category.
    pub async fn analysis_summary(
        &self,
        caller: &Caller,
        req: AnalysisTableRef,
    ) -> Result<AnalysisSummary, ApiError> {
        caller.require_owner()?;
        let analysis = self.follow_files(&req.space, &[]).await?;
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
        let files = vec![
            file("readings.csv", FileFormat::Csv),
            file("spending/2026-09.csv", FileFormat::Csv),
            file("spending/2026-10.csv", FileFormat::Csv),
            file("trips/2024/lisbon.parquet", FileFormat::Parquet),
            file("steps.jsonl", FileFormat::Json),
            file("other/readings.csv", FileFormat::Csv),
        ];
        let book = |path: &str, sheets: &[&str]| {
            let file = SheetFile {
                path: path.into(),
                size: 1,
                modified_ms: 1,
                local: None,
            };
            (file, sheets.iter().map(|s| (s.to_string(), None)).collect())
        };
        let sources = sources_of(
            files,
            vec![
                book("Budget 2026.xlsx", &["Q1 (Jan–Mar)", "Q2"]),
                book("money/steps.ods", &["Sheet1"]),
            ],
            Vec::new(),
        );
        let names: Vec<(&str, usize, Option<String>)> = sources
            .iter()
            .map(|(n, input)| (n.as_str(), input.paths().len(), input.sheet()))
            .collect();
        let sheet = |s: &str| Some(s.to_string());
        assert_eq!(
            names,
            vec![
                ("readings", 1, None),
                ("steps", 1, None),
                ("readings_2", 1, None),
                ("spending", 2, None),
                ("lisbon", 1, None),
                // A workbook of several sheets: a table each, named after
                // both; of one sheet, after the file.
                ("budget_2026_q1_jan_mar", 1, sheet("Q1 (Jan–Mar)")),
                ("budget_2026_q2", 1, sheet("Q2")),
                ("steps_2", 1, sheet("Sheet1")),
            ]
        );
    }

    #[test]
    fn literals_double_their_quotes() {
        assert_eq!(literal("it's.csv"), "'it''s.csv'");
    }
}
