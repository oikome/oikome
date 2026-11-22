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

//! Writing one Parquet data file for an Iceberg table.

use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{Schema, SchemaRef};
use bytes::Bytes;
use iceberg::io::FileIO;
use iceberg::spec::{DataContentType, DataFile, DataFileBuilder, DataFileFormat, TableMetadata};
use oikome_core::StorageError;
use parquet::arrow::ArrowWriter;
use parquet::arrow::PARQUET_FIELD_ID_META_KEY;
use parquet::file::properties::WriterProperties;

fn failed(e: impl std::fmt::Display) -> StorageError {
    StorageError::FlushFailed(e.to_string())
}

/// `schema` with each column's Iceberg field id, found by name in the
/// table's current schema. The Iceberg spec asks it of a data file's
/// Parquet schema, and other readers (DuckDB's `iceberg_scan`, Spark)
/// find a table's columns by it: without, every value reads as null.
/// Top-level columns only; the fields inside a nested column have none.
fn with_field_ids(schema: &SchemaRef, table: &TableMetadata) -> SchemaRef {
    let ids = table.current_schema();
    let fields: Vec<_> = schema
        .fields()
        .iter()
        .map(|field| match ids.field_by_name(field.name()) {
            Some(found) => {
                let mut metadata = field.metadata().clone();
                metadata.insert(PARQUET_FIELD_ID_META_KEY.to_string(), found.id.to_string());
                Arc::new(field.as_ref().clone().with_metadata(metadata))
            }
            None => field.clone(),
        })
        .collect();
    Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

/// Write `batch` as a ZSTD-compressed Parquet file under the table's
/// `data/` and describe it as an Iceberg [`DataFile`], its columns carrying
/// the table's field ids.
///
/// The file is staged in the local `temp_dir` and then uploaded through
/// `file_io`. It is not committed: pass the result to a fast-append
/// transaction or to `OikomeCatalog::replace_data_files`.
pub async fn write_data_file(
    file_io: &FileIO,
    table: &TableMetadata,
    schema: &SchemaRef,
    batch: &RecordBatch,
    temp_dir: &Path,
) -> Result<DataFile, StorageError> {
    let table_location = table.location();
    let schema = with_field_ids(schema, table);
    // Field metadata is not part of a column's type: the columns fit.
    let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec()).map_err(failed)?;
    let temp_path = temp_dir.join(format!("{}.parquet", uuid::Uuid::new_v4()));
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).unwrap_or_default(),
        ))
        .build();
    let mut writer = ArrowWriter::try_new(
        std::fs::File::create(&temp_path)?,
        schema.clone(),
        Some(props),
    )
    .map_err(failed)?;
    writer.write(&batch).map_err(failed)?;
    writer.close().map_err(failed)?;
    let file_size = std::fs::metadata(&temp_path)?.len();

    let target_path = format!("{table_location}/data/{}.parquet", uuid::Uuid::new_v4());
    let data = tokio::fs::read(&temp_path).await?;
    file_io
        .new_output(&target_path)
        .map_err(failed)?
        .write(Bytes::from(data))
        .await
        .map_err(failed)?;
    tokio::fs::remove_file(&temp_path).await?;

    DataFileBuilder::default()
        .content(DataContentType::Data)
        .file_path(target_path)
        .file_format(DataFileFormat::Parquet)
        .record_count(batch.num_rows() as u64)
        .file_size_in_bytes(file_size)
        .build()
        .map_err(failed)
}
