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

//! A page of the rows of one data file, read by the file's own Arrow
//! reader (no SQL): how a CSV, TSV, Parquet or JSON file of a folder
//! viewed in place is shown as a table, read where the file is.
//!
//! Text tables are read with every column as text, as written in the
//! file: a cell a person typed wrong never keeps the page from opening,
//! and days and times keep the shape they had in the file.

use std::io::{BufReader, Cursor};
use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_csv::reader::Format;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use oikome_engine::FileFormat;

use crate::ConnectError;

/// At most this many bytes of one file are read for a view; a bigger
/// file's view is its first part, marked cut.
pub const MAX_VIEW_BYTES: usize = 64 << 20;
/// Rows read to guess a JSON file's schema.
const INFER_ROWS: usize = 1000;

/// One page of a file's rows.
#[derive(Debug, Clone)]
pub struct FileRows {
    pub schema: SchemaRef,
    pub rows: Vec<RecordBatch>,
    /// Whether the file itself was cut off at [`MAX_VIEW_BYTES`], so the
    /// page shows part of it.
    pub cut: bool,
}

/// Rows `offset..offset + limit` of `bytes`, parsed by `format`'s reader.
/// `tail_cut` says `bytes` already ends before the file does. Parsing
/// stops once the page is read.
pub fn read_page(
    format: FileFormat,
    name: &str,
    bytes: &[u8],
    offset: u64,
    limit: usize,
    tail_cut: bool,
) -> Result<FileRows, ConnectError> {
    let limit = limit.max(1) as u64;
    let end = offset.saturating_add(limit);
    fn bad(name: &str, e: impl std::fmt::Display) -> ConnectError {
        ConnectError::Format(format!("{name}: {e}"))
    }
    let (schema, reader): (SchemaRef, Box<dyn RecordBatchReader + Send>) = match format {
        FileFormat::Csv => {
            let tsv = name.to_ascii_lowercase().ends_with(".tsv");
            let mut format = Format::default().with_header(true);
            if tsv {
                format = format.with_delimiter(b'\t');
            }
            // Column names from the header; every cell stays the text it is.
            let (inferred, _) = format
                .infer_schema(Cursor::new(bytes), Some(1))
                .map_err(|e| bad(name, e))?;
            let fields: Vec<Field> = inferred
                .fields()
                .iter()
                .map(|f| Field::new(f.name(), DataType::Utf8, true))
                .collect();
            let schema = Arc::new(Schema::new(fields));
            let mut builder = arrow_csv::ReaderBuilder::new(schema.clone()).with_header(true);
            if tsv {
                builder = builder.with_delimiter(b'\t');
            }
            let reader = builder
                .build(Cursor::new(bytes))
                .map_err(|e| bad(name, e))?;
            (schema, Box::new(reader))
        }
        FileFormat::Parquet => {
            let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
                bytes::Bytes::copy_from_slice(bytes),
            )
            .map_err(|e| bad(name, e))?;
            let schema = builder.schema().clone();
            let reader = builder.build().map_err(|e| bad(name, e))?;
            (schema, Box::new(reader))
        }
        FileFormat::Json => {
            let (schema, _) = arrow_json::reader::infer_json_schema(
                BufReader::with_capacity(64 << 10, bytes),
                Some(INFER_ROWS),
            )
            .map_err(|e| bad(name, e))?;
            let schema = Arc::new(schema);
            let reader = arrow_json::ReaderBuilder::new(schema.clone())
                .build(BufReader::with_capacity(64 << 10, bytes))
                .map_err(|e| bad(name, e))?;
            (schema, Box::new(reader))
        }
    };
    let mut rows = Vec::new();
    let mut seen = 0u64;
    for batch in reader {
        let batch = batch.map_err(|e| bad(name, e))?;
        let first = seen;
        seen += batch.num_rows() as u64;
        if seen <= offset {
            continue;
        }
        let start = offset.saturating_sub(first) as usize;
        let stop = end.saturating_sub(first).min(batch.num_rows() as u64) as usize;
        if stop > start {
            rows.push(batch.slice(start, stop - start));
        }
        if seen >= end {
            break;
        }
    }
    Ok(FileRows {
        schema,
        rows,
        cut: tail_cut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::StringArray;

    fn texts(batch: &RecordBatch, column: usize) -> Vec<String> {
        batch
            .column(column)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .map(|v| v.unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn a_csv_is_read_as_text_with_its_header() {
        let bytes = b"day,item,amount\n2025-11-01,coffee,3.50\n2025-11-02,tram,2\n";
        let page = read_page(FileFormat::Csv, "a.csv", bytes, 0, 100, false).unwrap();
        assert!(!page.cut);
        assert_eq!(page.schema.fields().len(), 3);
        assert_eq!(
            page.schema.fields()[0].data_type(),
            &DataType::Utf8,
            "every column stays text"
        );
        assert_eq!(page.rows.len(), 1);
        assert_eq!(texts(&page.rows[0], 0), ["2025-11-01", "2025-11-02"]);
        assert_eq!(
            texts(&page.rows[0], 2),
            ["3.50", "2"],
            "the money is as written"
        );
        // A page past the first rows; a bad cell in a typed column would
        // not stop the page, there are no typed columns.
        let page = read_page(FileFormat::Csv, "a.csv", bytes, 1, 1, false).unwrap();
        assert_eq!(texts(&page.rows[0], 1), ["tram"]);
    }

    #[test]
    fn a_tsv_uses_tabs() {
        let bytes = b"day\titem\n2025-11-01\tcoffee\n";
        let page = read_page(FileFormat::Csv, "a.tsv", bytes, 0, 100, false).unwrap();
        assert_eq!(page.schema.fields().len(), 2);
        assert_eq!(texts(&page.rows[0], 1), ["coffee"]);
    }

    #[test]
    fn json_lines_are_inferred_and_read() {
        let bytes = b"{\"day\": \"2025-11-01\", \"what\": \"coffee\", \"n\": 2}\n{\"day\": \"2025-11-02\", \"what\": \"tram\", \"n\": 1}\n";
        let page = read_page(FileFormat::Json, "a.jsonl", bytes, 0, 100, false).unwrap();
        assert_eq!(page.rows.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
        let what = page
            .schema
            .fields()
            .iter()
            .position(|f| f.name() == "what")
            .unwrap();
        assert_eq!(texts(&page.rows[0], what), ["coffee", "tram"]);
        // A page of the second half.
        let page = read_page(FileFormat::Json, "a.jsonl", bytes, 1, 1, false).unwrap();
        assert_eq!(page.rows[0].num_rows(), 1);
    }

    #[test]
    fn parquet_is_read_typed() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Utf8, true),
            Field::new("amount", DataType::Float64, true),
        ]));
        let day = Arc::new(StringArray::from(vec!["2025-11-01", "2025-11-02"]));
        let amount = Arc::new(arrow_array::Float64Array::from(vec![3.5, 2.0]));
        let batch = RecordBatch::try_new(schema.clone(), vec![day, amount]).unwrap();
        let mut bytes = Vec::new();
        let mut writer = parquet::arrow::ArrowWriter::try_new(&mut bytes, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let page = read_page(FileFormat::Parquet, "a.parquet", &bytes, 0, 100, false).unwrap();
        assert_eq!(page.rows[0].num_rows(), 2);
        assert_eq!(
            page.schema.fields()[1].data_type(),
            &DataType::Float64,
            "a Parquet file keeps its own types"
        );
        let page = read_page(FileFormat::Parquet, "a.parquet", &bytes, 1, 1, false).unwrap();
        assert_eq!(page.rows[0].num_rows(), 1);
    }

    #[test]
    fn a_cut_file_says_so() {
        let bytes = b"a,b\n1,2\n";
        let page = read_page(FileFormat::Csv, "a.csv", bytes, 0, 100, true).unwrap();
        assert!(page.cut);
    }
}
