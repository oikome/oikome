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

//! Spreadsheets (Excel's xlsx, xlsm, xls and xlsb; OpenDocument's ods),
//! read for analysis: each visible worksheet with anything in it becomes
//! one table. Its first row with anything in it names the columns; a
//! formula gives the value last calculated and saved with the file. A
//! column of numbers, of true and false, or of dates stays one; any other
//! mix is text.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType as ArrowType, Field, Schema, TimeUnit};
use calamine::{open_workbook_auto, Data, DataType, Range, Reader, SheetType, SheetVisible};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

use crate::ConnectError;

/// The extensions of the spreadsheets read, in lower case.
pub const EXTENSIONS: [&str; 5] = ["xlsx", "xlsm", "xls", "xlsb", "ods"];

/// Whether `name` is a spreadsheet by its extension.
pub fn is_spreadsheet(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// One worksheet, as a table.
#[derive(Debug)]
pub struct SheetTable {
    pub name: String,
    pub batch: RecordBatch,
}

/// Every visible worksheet of the workbook at `path` that has a row with
/// anything in it, in the workbook's order.
pub fn read(path: &Path) -> Result<Vec<SheetTable>, ConnectError> {
    let fail = |e: &dyn std::fmt::Display| ConnectError::Format(format!("{}: {e}", path.display()));
    let mut book = open_workbook_auto(path).map_err(|e| fail(&e))?;
    let sheets: Vec<String> = book
        .sheets_metadata()
        .iter()
        .filter(|s| s.typ == SheetType::WorkSheet && s.visible == SheetVisible::Visible)
        .map(|s| s.name.clone())
        .collect();
    let mut out = Vec::new();
    for name in sheets {
        let range = book.worksheet_range(&name).map_err(|e| fail(&e))?;
        if let Some(batch) = table_of(&range).map_err(|e| fail(&e))? {
            out.push(SheetTable { name, batch });
        }
    }
    Ok(out)
}

/// What a column holds, from all its cells with something in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Number,
    Boolean,
    Date,
    Timestamp,
    Text,
}

fn kind_of_cell(cell: &Data) -> Option<Kind> {
    match cell {
        Data::Empty | Data::Error(_) => None,
        Data::Int(_) => Some(Kind::Integer),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(Kind::Integer),
        Data::Float(_) => Some(Kind::Number),
        Data::Bool(_) => Some(Kind::Boolean),
        Data::DateTime(d) if d.is_datetime() => match cell.as_datetime() {
            Some(at) if at.time() == NaiveTime::MIN => Some(Kind::Date),
            Some(_) => Some(Kind::Timestamp),
            None => Some(Kind::Text),
        },
        Data::DateTimeIso(_) => match cell.as_datetime() {
            Some(at) if at.time() == NaiveTime::MIN => Some(Kind::Date),
            Some(_) => Some(Kind::Timestamp),
            None => Some(Kind::Text),
        },
        Data::DateTime(_) | Data::DurationIso(_) | Data::String(_) => Some(Kind::Text),
    }
}

/// The kind both `a` and `b` fit in.
fn widen(a: Kind, b: Kind) -> Kind {
    use Kind::*;
    match (a, b) {
        (a, b) if a == b => a,
        (Integer, Number) | (Number, Integer) => Number,
        (Date, Timestamp) | (Timestamp, Date) => Timestamp,
        _ => Text,
    }
}

/// A cell as text, as the owner reads it in the sheet.
fn text_of(cell: &Data) -> Option<String> {
    match cell {
        Data::Empty | Data::Error(_) => None,
        Data::String(s) => Some(s.clone()),
        Data::Int(i) => Some(i.to_string()),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(format!("{}", *f as i64)),
        Data::Float(f) => Some(f.to_string()),
        Data::Bool(b) => Some(b.to_string()),
        Data::DateTime(d) if d.is_datetime() => match cell.as_datetime() {
            Some(at) if at.time() == NaiveTime::MIN => Some(at.date().to_string()),
            Some(at) => Some(at.format("%Y-%m-%d %H:%M:%S").to_string()),
            None => Some(d.as_f64().to_string()),
        },
        Data::DateTime(d) => Some(d.as_f64().to_string()),
        Data::DateTimeIso(s) | Data::DurationIso(s) => Some(s.clone()),
    }
}

fn empty(cell: &Data) -> bool {
    matches!(cell, Data::Empty) || matches!(cell, Data::String(s) if s.trim().is_empty())
}

/// The column names of a header row: as written, trimmed; a blank one is
/// `column_<n>`; one written twice gets `_2`, `_3`…
fn names_of(header: &[Data]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, cell) in header.iter().enumerate() {
        let written = text_of(cell)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| format!("column_{}", i + 1));
        let mut name = written.clone();
        let mut n = 2;
        while names.contains(&name) {
            name = format!("{written}_{n}");
            n += 1;
        }
        names.push(name);
    }
    names
}

fn days_of(date: NaiveDate) -> i32 {
    (date - NaiveDate::from_ymd_opt(1970, 1, 1).expect("a date")).num_days() as i32
}

fn micros_of(at: NaiveDateTime) -> i64 {
    at.and_utc().timestamp_micros()
}

/// A worksheet's rows as one batch: `None` when nothing is in it.
fn table_of(range: &Range<Data>) -> Result<Option<RecordBatch>, arrow_schema::ArrowError> {
    let mut rows = range.rows().filter(|row| !row.iter().all(empty));
    let Some(header) = rows.next() else {
        return Ok(None);
    };
    let rows: Vec<&[Data]> = rows.collect();
    // As wide as the header, or a row that goes further.
    let width = rows
        .iter()
        .map(|r| r.len())
        .chain([header.len()])
        .max()
        .unwrap_or(0);
    let mut header = header.to_vec();
    header.resize(width, Data::Empty);
    let names = names_of(&header);
    let cell = |row: &[Data], i: usize| row.get(i).cloned().unwrap_or(Data::Empty);

    let mut fields = Vec::with_capacity(width);
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(width);
    for (i, name) in names.iter().enumerate() {
        let cells: Vec<Data> = rows.iter().map(|row| cell(row, i)).collect();
        let kind = cells
            .iter()
            .filter_map(kind_of_cell)
            .reduce(widen)
            .unwrap_or(Kind::Text);
        let (data_type, column): (ArrowType, ArrayRef) = match kind {
            Kind::Integer => (
                ArrowType::Int64,
                Arc::new(Int64Array::from_iter(cells.iter().map(|c| match c {
                    Data::Int(i) => Some(*i),
                    Data::Float(f) => Some(*f as i64),
                    _ => None,
                }))),
            ),
            Kind::Number => (
                ArrowType::Float64,
                Arc::new(Float64Array::from_iter(cells.iter().map(|c| match c {
                    Data::Int(i) => Some(*i as f64),
                    Data::Float(f) => Some(*f),
                    _ => None,
                }))),
            ),
            Kind::Boolean => (
                ArrowType::Boolean,
                Arc::new(BooleanArray::from_iter(cells.iter().map(|c| match c {
                    Data::Bool(b) => Some(*b),
                    _ => None,
                }))),
            ),
            Kind::Date => (
                ArrowType::Date32,
                Arc::new(Date32Array::from_iter(
                    cells
                        .iter()
                        .map(|c| c.as_datetime().map(|at| days_of(at.date()))),
                )),
            ),
            Kind::Timestamp => (
                ArrowType::Timestamp(TimeUnit::Microsecond, None),
                Arc::new(TimestampMicrosecondArray::from_iter(
                    cells.iter().map(|c| c.as_datetime().map(micros_of)),
                )),
            ),
            Kind::Text => (
                ArrowType::Utf8,
                Arc::new(StringArray::from_iter(cells.iter().map(text_of))),
            ),
        };
        fields.push(Field::new(name, data_type, true));
        columns.push(column);
    }
    let schema = Arc::new(Schema::new(fields));
    if rows.is_empty() {
        return Ok(Some(RecordBatch::new_empty(schema)));
    }
    RecordBatch::try_new(schema, columns).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Array;
    use calamine::{ExcelDateTime, ExcelDateTimeType};

    fn range(rows: Vec<Vec<Data>>) -> Range<Data> {
        let height = rows.len() as u32;
        let width = rows.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let mut range = Range::new((0, 0), (height - 1, width - 1));
        for (r, row) in rows.into_iter().enumerate() {
            for (c, value) in row.into_iter().enumerate() {
                range.set_value((r as u32, c as u32), value);
            }
        }
        range
    }

    fn date(serial: f64) -> Data {
        Data::DateTime(ExcelDateTime::new(
            serial,
            ExcelDateTimeType::DateTime,
            false,
        ))
    }

    #[test]
    fn a_sheet_keeps_numbers_dates_and_text_apart() {
        let s = |t: &str| Data::String(t.into());
        let sheet = range(vec![
            vec![Data::Empty, Data::Empty, Data::Empty, Data::Empty],
            vec![s("Date"), s("Amount"), s("Paid"), s("Note")],
            // 2026-01-01 and 2026-01-02 as Excel serials.
            vec![
                date(46023.0),
                Data::Float(12.5),
                Data::Bool(true),
                s("Market"),
            ],
            vec![Data::Empty, Data::Empty, Data::Empty, Data::Empty],
            vec![
                date(46024.0),
                Data::Int(30),
                Data::Bool(false),
                Data::Float(7.0),
            ],
        ]);
        let batch = table_of(&sheet).unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2, "blank rows are not rows");
        let schema = batch.schema();
        let types: Vec<_> = schema
            .fields()
            .iter()
            .map(|f| (f.name().as_str(), f.data_type().clone()))
            .collect();
        assert_eq!(
            types,
            [
                ("Date", ArrowType::Date32),
                ("Amount", ArrowType::Float64),
                ("Paid", ArrowType::Boolean),
                ("Note", ArrowType::Utf8),
            ]
        );
        let dates = batch
            .column(0)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert_eq!(dates.value_as_date(0).unwrap().to_string(), "2026-01-01");
        let notes = batch
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(
            notes.value(1),
            "7",
            "a whole number in text reads as written"
        );
    }

    #[test]
    fn names_are_made_unique_and_blank_ones_numbered() {
        let s = |t: &str| Data::String(t.into());
        let sheet = range(vec![
            vec![s(" City "), s("City"), Data::Empty],
            vec![s("Lisbon"), s("Porto"), Data::Int(3)],
        ]);
        let batch = table_of(&sheet).unwrap().unwrap();
        let schema = batch.schema();
        let names: Vec<_> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, ["City", "City_2", "column_3"]);
        let counts = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(counts.value(0), 3);
        assert!(batch.column(0).is_valid(0));
    }

    #[test]
    fn an_empty_sheet_is_no_table() {
        let sheet = range(vec![vec![Data::Empty, Data::String("  ".into())]]);
        assert!(table_of(&sheet).unwrap().is_none());
    }

    #[test]
    fn spreadsheets_are_told_by_their_extension() {
        assert!(is_spreadsheet("Budget 2026.XLSX"));
        assert!(is_spreadsheet("trips.ods"));
        assert!(!is_spreadsheet("trips.csv"));
    }
}
