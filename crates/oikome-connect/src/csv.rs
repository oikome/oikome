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

//! A CSV file with a header row, imported into one table.
//!
//! The columns and their types are read once, when the source is added
//! ([`CsvSource::inspect`]); a later file with other columns is refused.
//! Every pull reads the whole file, which lists every row (`complete`), so
//! rows missing from it are deleted. An unchanged file (same BLAKE3) is not
//! read again.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, StringArray};
use arrow_csv::reader::Format;
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;

use crate::{
    content_hash, ColumnDef, ColumnKind, ConnectError, Connector, Pull, Row, TableDef, Value,
};

pub const KIND: &str = "csv";
/// Rows read to guess column types.
const INFER_ROWS: usize = 1000;

/// Whether `name` can name a table: a letter or `_`, then letters, digits
/// or `_`.
pub fn is_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 64
}

pub struct CsvSource {
    path: PathBuf,
    table: TableDef,
}

impl CsvSource {
    pub fn new(path: PathBuf, table: TableDef) -> Self {
        Self { path, table }
    }

    /// The column names of a CSV file's header, as written.
    pub fn header(path: &Path) -> Result<Vec<String>, ConnectError> {
        let bytes = std::fs::read(path)
            .map_err(|e| ConnectError::Io(format!("{}: {e}", path.display())))?;
        let (schema, _) = Format::default()
            .with_header(true)
            .infer_schema(Cursor::new(&bytes), Some(1))
            .map_err(|e| ConnectError::Format(format!("{}: {e}", path.display())))?;
        Ok(schema
            .fields()
            .iter()
            .map(|f| f.name().trim().to_string())
            .collect())
    }

    /// The table a CSV file makes: its header's columns, their types
    /// guessed from the first rows (dates and times are text), and `key`
    /// as the columns that identify a row.
    pub fn inspect(path: &Path, table: &str, key: &[String]) -> Result<TableDef, ConnectError> {
        if !is_table_name(table) {
            return Err(ConnectError::Config(format!(
                "'{table}' is not a table name (letters, digits and '_')"
            )));
        }
        let bytes = std::fs::read(path)
            .map_err(|e| ConnectError::Io(format!("{}: {e}", path.display())))?;
        let (schema, _) = Format::default()
            .with_header(true)
            .infer_schema(Cursor::new(&bytes), Some(INFER_ROWS))
            .map_err(|e| ConnectError::Format(format!("{}: {e}", path.display())))?;
        let mut columns = Vec::with_capacity(schema.fields().len());
        for field in schema.fields() {
            let name = field.name().trim().to_string();
            if name.is_empty() || name.chars().any(char::is_control) || name.starts_with("_oikome")
            {
                return Err(ConnectError::Format(format!(
                    "{}: the header has an empty or invalid column name",
                    path.display()
                )));
            }
            if columns.iter().any(|c: &ColumnDef| c.name == name) {
                return Err(ConnectError::Format(format!(
                    "{}: column '{name}' appears twice",
                    path.display()
                )));
            }
            let kind = match field.data_type() {
                DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64 => ColumnKind::Integer,
                DataType::Float16 | DataType::Float32 | DataType::Float64 => ColumnKind::Real,
                DataType::Boolean => ColumnKind::Boolean,
                _ => ColumnKind::Text,
            };
            columns.push(ColumnDef { name, kind });
        }
        if key.is_empty() {
            return Err(ConnectError::Config(
                "name the column(s) that identify a row (--key)".to_string(),
            ));
        }
        for k in key {
            if !columns.iter().any(|c| &c.name == k) {
                return Err(ConnectError::Config(format!(
                    "{}: there is no column '{k}'",
                    path.display()
                )));
            }
        }
        Ok(TableDef {
            name: table.to_string(),
            columns,
            key: key.to_vec(),
        })
    }

    fn cell(
        &self,
        column: &ColumnDef,
        text: Option<&str>,
        row: usize,
    ) -> Result<Value, ConnectError> {
        let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
            return Ok(Value::Null);
        };
        let bad = |what: &str| {
            ConnectError::Format(format!(
                "{} row {row}, column '{}': '{text}' is not {what}",
                self.path.display(),
                column.name
            ))
        };
        Ok(match column.kind {
            ColumnKind::Integer => Value::Int64(text.parse().map_err(|_| bad("an integer"))?),
            ColumnKind::Real => Value::Float64(text.parse().map_err(|_| bad("a number"))?),
            ColumnKind::Boolean => match text.to_ascii_lowercase().as_str() {
                "true" | "1" => Value::Bool(true),
                "false" | "0" => Value::Bool(false),
                _ => return Err(bad("true or false")),
            },
            ColumnKind::Text | ColumnKind::Timestamp => Value::Text(text.to_string()),
            ColumnKind::Blob => Value::Blob(text.as_bytes().to_vec()),
        })
    }
}

#[async_trait]
impl Connector for CsvSource {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn tables(&self) -> Vec<TableDef> {
        vec![self.table.clone()]
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<Pull, ConnectError> {
        let bytes = tokio::fs::read(&self.path)
            .await
            .map_err(|e| ConnectError::Io(format!("{}: {e}", self.path.display())))?;
        let hash = content_hash(&bytes);
        if cursor == Some(hash.as_str()) {
            return Ok(Pull::unchanged(Some(hash)));
        }
        let (header, _) = Format::default()
            .with_header(true)
            .infer_schema(Cursor::new(&bytes), Some(1))
            .map_err(|e| ConnectError::Format(format!("{}: {e}", self.path.display())))?;
        let names: Vec<String> = header
            .fields()
            .iter()
            .map(|f| f.name().trim().to_string())
            .collect();
        let expected: Vec<&str> = self.table.columns.iter().map(|c| c.name.as_str()).collect();
        if names != expected {
            return Err(ConnectError::Format(format!(
                "{} now has the columns {names:?}, not {expected:?}; add the source again",
                self.path.display()
            )));
        }
        // Read every cell as text, then convert by the declared types.
        let schema = Arc::new(Schema::new(
            names
                .iter()
                .map(|n| Field::new(n, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        let reader = arrow_csv::ReaderBuilder::new(schema)
            .with_header(true)
            .build(Cursor::new(&bytes))
            .map_err(|e| ConnectError::Format(format!("{}: {e}", self.path.display())))?;
        let keys = self.table.key_positions();
        let mut rows: Vec<Row> = Vec::new();
        for batch in reader {
            let batch =
                batch.map_err(|e| ConnectError::Format(format!("{}: {e}", self.path.display())))?;
            let columns: Vec<&StringArray> = batch
                .columns()
                .iter()
                .map(|c| c.as_any().downcast_ref::<StringArray>())
                .collect::<Option<_>>()
                .ok_or_else(|| ConnectError::Format("a column was not read as text".into()))?;
            for i in 0..batch.num_rows() {
                let line = rows.len() + 2;
                let row = self
                    .table
                    .columns
                    .iter()
                    .zip(&columns)
                    .map(|(column, values)| {
                        let text = (!values.is_null(i)).then(|| values.value(i));
                        self.cell(column, text, line)
                    })
                    .collect::<Result<Row, _>>()?;
                if keys.iter().any(|&k| row[k] == Value::Null) {
                    return Err(ConnectError::Format(format!(
                        "{} row {line}: a key column is empty",
                        self.path.display()
                    )));
                }
                rows.push(row);
            }
        }
        Ok(Pull {
            upserts: vec![(self.table.name.clone(), rows)],
            deletes: Vec::new(),
            cursor: Some(hash),
            complete: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_is_typed_once_and_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bank.csv");
        std::fs::write(
            &path,
            "txn_id,amount,booked,note\n1,12.5,2026-01-02,coffee\n2,40,2026-01-03,\n",
        )
        .unwrap();
        let table = CsvSource::inspect(&path, "transactions", &["txn_id".into()]).unwrap();
        let kinds: Vec<_> = table.columns.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ColumnKind::Integer,
                ColumnKind::Real,
                ColumnKind::Text,
                ColumnKind::Text
            ]
        );
        let source = CsvSource::new(path.clone(), table.clone());
        let pull = source.pull(None).await.unwrap();
        assert!(pull.complete);
        let rows = &pull.upserts[0].1;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][1], Value::Float64(40.0));
        assert_eq!(rows[1][3], Value::Null);
        // Unchanged: nothing to apply.
        let again = source.pull(pull.cursor.as_deref()).await.unwrap();
        assert!(again.upserts.is_empty() && !again.complete);

        // Other columns, a bad number, a missing key.
        std::fs::write(&path, "txn_id,amount\n1,2\n").unwrap();
        assert!(source.pull(None).await.is_err());
        std::fs::write(&path, "txn_id,amount,booked,note\n1,lots,x,y\n").unwrap();
        assert!(source
            .pull(None)
            .await
            .unwrap_err()
            .to_string()
            .contains("lots"));
        std::fs::write(&path, "txn_id,amount,booked,note\n,1,x,y\n").unwrap();
        assert!(source.pull(None).await.is_err());
        assert!(CsvSource::inspect(&path, "t", &["nope".into()]).is_err());
        assert!(CsvSource::inspect(&path, "bad name", &["txn_id".into()]).is_err());
    }
}
