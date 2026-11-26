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

//! Charts of an analysis table. A chart is a description: the dimensions
//! its rows are grouped by, the dimensions compared within each group, the
//! metrics totalled, the filters, the order and how many groups to show.
//! No SQL comes from the page. The query is written here, from the
//! description: every column is one of the table's own and is quoted, every
//! value a filter gives is a bound parameter, and everything else is this
//! file's own text. DuckDB computes; the page draws the answer.

use std::collections::BTreeSet;

use arrow_array::{Array, Float64Array, Int64Array, LargeStringArray, StringArray};
use arrow_schema::{DataType, Schema};
use oikome_engine::Value;
use oikome_node::QueryResult;
use oikome_semantic::rewrite::quote_ident;

use crate::{
    AnalysisChart, AnalysisValues, Api, ApiError, Caller, ChartAggregate, ChartDimension,
    ChartFilter, ChartGroup, ChartMetric, ChartResult, ChartRows, ChartSeries, ChartSpec,
    DateGrain, FieldRole,
};

/// How much a chart may ask for.
const MAX_DIMENSIONS: usize = 4;
const MAX_COMPARE: usize = 2;
const MAX_METRICS: usize = 8;
const MAX_FILTERS: usize = 12;
const MAX_FILTER_VALUES: usize = 500;
const MAX_VALUE_BYTES: usize = 1024;
/// The most groups shown, the most cells answered with, and the most rows
/// of a chart that groups nothing.
const MAX_GROUPS: u32 = 1000;
const MAX_CELLS: usize = 20_000;
const MAX_ROWS: usize = 200;
/// The most values of a column offered to a filter.
const MAX_OFFERED: usize = 500;

/// A table's columns as a chart may use them.
struct Columns<'a> {
    schema: &'a Schema,
    roles: &'a [(String, FieldRole)],
}

impl Columns<'_> {
    /// The column, quoted, if the table has it.
    fn quoted(&self, name: &str) -> Result<String, ApiError> {
        self.schema
            .field_with_name(name)
            .map(|_| quote_ident(name))
            .map_err(|_| ApiError::invalid(format!("the table has no column {name}")))
    }

    fn is_number(&self, name: &str) -> bool {
        self.schema.field_with_name(name).is_ok_and(|f| {
            matches!(
                f.data_type(),
                DataType::Int8
                    | DataType::Int16
                    | DataType::Int32
                    | DataType::Int64
                    | DataType::UInt8
                    | DataType::UInt16
                    | DataType::UInt32
                    | DataType::UInt64
                    | DataType::Float16
                    | DataType::Float32
                    | DataType::Float64
                    | DataType::Decimal128(_, _)
                    | DataType::Decimal256(_, _)
            )
        })
    }

    /// A column of time: by its type, or because the owner said so.
    fn is_time(&self, name: &str) -> bool {
        self.schema.field_with_name(name).is_ok_and(|f| {
            matches!(
                f.data_type(),
                DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
            )
        }) || self
            .roles
            .iter()
            .any(|(of, role)| of == name && *role == FieldRole::Time)
    }
}

/// A query being written: its bound values, in the order they are named.
#[derive(Default, Clone)]
struct Bound(Vec<Value>);

impl Bound {
    /// Bind a value; the answer is the placeholder that stands for it.
    fn bind(&mut self, value: Value) -> String {
        self.0.push(value);
        format!("?{}", self.0.len())
    }
}

/// A dimension as text that sorts: a column's value, or a time at a grain.
fn dimension(columns: &Columns, dimension: &ChartDimension) -> Result<String, ApiError> {
    let column = columns.quoted(&dimension.column)?;
    let timed = columns.is_time(&dimension.column);
    let Some(grain) = dimension.grain.or(timed.then_some(DateGrain::Day)) else {
        return Ok(format!("CAST({column} AS VARCHAR)"));
    };
    if !timed {
        return Err(ApiError::invalid(format!(
            "{} is not a column of time",
            dimension.column
        )));
    }
    let at = format!("TRY_CAST({column} AS TIMESTAMP)");
    Ok(match grain {
        DateGrain::Year => format!("strftime({at}, '%Y')"),
        DateGrain::Quarter => {
            format!("strftime({at}, '%Y') || '-Q' || CAST(quarter({at}) AS VARCHAR)")
        }
        DateGrain::Month => format!("strftime({at}, '%Y-%m')"),
        DateGrain::Week => format!("strftime(date_trunc('week', {at}), '%Y-%m-%d')"),
        DateGrain::Day => format!("strftime({at}, '%Y-%m-%d')"),
    })
}

/// A metric as a number: a column totalled, or the rows counted.
fn metric(columns: &Columns, metric: &ChartMetric) -> Result<String, ApiError> {
    let Some(name) = &metric.column else {
        return match metric.aggregate {
            ChartAggregate::Count => Ok("CAST(COUNT(*) AS DOUBLE)".to_string()),
            _ => Err(ApiError::invalid("a metric without a column counts rows")),
        };
    };
    let column = columns.quoted(name)?;
    let function = match metric.aggregate {
        ChartAggregate::Count => return Ok(format!("CAST(COUNT({column}) AS DOUBLE)")),
        ChartAggregate::Distinct => return Ok(format!("CAST(COUNT(DISTINCT {column}) AS DOUBLE)")),
        ChartAggregate::Sum => "SUM",
        ChartAggregate::Average => "AVG",
        ChartAggregate::Median => "MEDIAN",
        ChartAggregate::Min => "MIN",
        ChartAggregate::Max => "MAX",
    };
    if !columns.is_number(name) {
        return Err(ApiError::invalid(format!(
            "{name} is not a number: it can be counted"
        )));
    }
    Ok(format!(
        "CAST({function}(TRY_CAST({column} AS DOUBLE)) AS DOUBLE)"
    ))
}

/// A day as `YYYY-MM-DD`, and nothing else.
fn day(text: &str) -> Result<String, ApiError> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
    if shaped {
        Ok(text.to_string())
    } else {
        Err(ApiError::invalid(format!(
            "{text} is not a day (YYYY-MM-DD)"
        )))
    }
}

/// A filter as a condition, its values bound; `None` when it asks nothing.
fn filter(
    columns: &Columns,
    filter: &ChartFilter,
    bound: &mut Bound,
) -> Result<Option<String>, ApiError> {
    let name = &filter.column;
    let column = columns.quoted(name)?;
    let ranged = filter.min.is_some() || filter.max.is_some();
    let kinds = [
        !filter.values.is_empty(),
        ranged,
        filter.last_days.is_some(),
        filter.this_year,
    ];
    if kinds.iter().filter(|k| **k).count() > 1 {
        return Err(ApiError::invalid(format!(
            "the filter on {name} is of more than one kind"
        )));
    }
    if !filter.values.is_empty() {
        if filter.values.len() > MAX_FILTER_VALUES
            || filter.values.iter().any(|v| v.len() > MAX_VALUE_BYTES)
        {
            return Err(ApiError::invalid(format!(
                "the filter on {name} has too many values"
            )));
        }
        let values: Vec<String> = filter
            .values
            .iter()
            .map(|v| bound.bind(Value::Text(v.clone())))
            .collect();
        return Ok(Some(format!(
            "CAST({column} AS VARCHAR) IN ({})",
            values.join(", ")
        )));
    }
    let at = format!("TRY_CAST({column} AS TIMESTAMP)");
    let timed = || {
        if columns.is_time(name) {
            Ok(())
        } else {
            Err(ApiError::invalid(format!("{name} is not a column of time")))
        }
    };
    if let Some(days) = filter.last_days {
        timed()?;
        let days = bound.bind(Value::Int64(i64::from(days.min(36_500))));
        return Ok(Some(format!(
            "{at} >= CAST(current_date - CAST({days} AS INTEGER) AS TIMESTAMP)"
        )));
    }
    if filter.this_year {
        timed()?;
        return Ok(Some(format!("year({at}) = year(current_date)")));
    }
    if !ranged {
        return Ok(None);
    }
    let mut ends = Vec::new();
    if columns.is_number(name) {
        for (end, compare) in [(&filter.min, ">="), (&filter.max, "<=")] {
            let Some(text) = end else { continue };
            let number: f64 = text
                .trim()
                .parse()
                .ok()
                .filter(|n: &f64| n.is_finite())
                .ok_or_else(|| ApiError::invalid(format!("{text} is not a number")))?;
            let value = bound.bind(Value::Float64(number));
            ends.push(format!("TRY_CAST({column} AS DOUBLE) {compare} {value}"));
        }
    } else {
        timed()?;
        if let Some(from) = &filter.min {
            let value = bound.bind(Value::Text(day(from)?));
            ends.push(format!("{at} >= CAST({value} AS TIMESTAMP)"));
        }
        // To the end of the last day.
        if let Some(to) = &filter.max {
            let value = bound.bind(Value::Text(day(to)?));
            ends.push(format!(
                "{at} < CAST({value} AS TIMESTAMP) + INTERVAL 1 DAY"
            ));
        }
    }
    Ok(Some(ends.join(" AND ")))
}

/// The text at a row of a column of text.
fn text_at(column: &dyn Array, row: usize) -> Option<String> {
    if column.is_null(row) {
        return None;
    }
    if let Some(texts) = column.as_any().downcast_ref::<StringArray>() {
        return Some(texts.value(row).to_string());
    }
    column
        .as_any()
        .downcast_ref::<LargeStringArray>()
        .map(|texts| texts.value(row).to_string())
}

fn number_at(column: &dyn Array, row: usize) -> Option<f64> {
    if column.is_null(row) {
        return None;
    }
    if let Some(numbers) = column.as_any().downcast_ref::<Float64Array>() {
        return Some(numbers.value(row)).filter(|n| n.is_finite());
    }
    column
        .as_any()
        .downcast_ref::<Int64Array>()
        .map(|numbers| numbers.value(row) as f64)
}

impl Api {
    /// What a chart of an analysis table shows: its groups and series,
    /// computed by the node from the chart's description.
    pub async fn analysis_chart(
        &self,
        caller: &Caller,
        req: AnalysisChart,
    ) -> Result<ChartResult, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (schema, roles) = self.analysis_columns(&req.space, &req.table).await?;
        let columns = Columns {
            schema: &schema,
            roles: &roles,
        };
        let spec: &ChartSpec = &req.spec;
        if spec.dimensions.len() > MAX_DIMENSIONS
            || spec.compare.len() > MAX_COMPARE
            || spec.metrics.len() > MAX_METRICS
            || spec.filters.len() > MAX_FILTERS
        {
            return Err(ApiError::invalid("the chart asks for too much at once"));
        }
        let from = format!("{}.{}", quote_ident(&req.space), quote_ident(&req.table));
        let mut bound = Bound::default();
        let mut conditions = Vec::new();
        for each in &spec.filters {
            if let Some(condition) = filter(&columns, each, &mut bound)? {
                conditions.push(condition);
            }
        }
        // The rows the filters keep. Written out wherever they are read, not
        // as a CTE read twice: DuckDB copies such a CTE's plan, which a scan
        // of an Iceberg table read in place cannot be.
        let kept = if conditions.is_empty() {
            format!("SELECT * FROM {from}")
        } else {
            format!("SELECT * FROM {from} WHERE {}", conditions.join(" AND "))
        };
        let session = self.session(None);
        let batches_of = |result: QueryResult| match result {
            QueryResult::Batches { batches, .. } => Ok(batches),
            _ => Err(ApiError::internal("a chart's query returned no rows")),
        };

        // How many rows the filters keep, whatever is made of them.
        let counted = session
            .query_params(&format!("SELECT COUNT(*) FROM ({kept}) AS f"), &bound.0)
            .await?;
        let matched = batches_of(counted)?
            .iter()
            .find(|b| b.num_rows() > 0)
            .and_then(|b| number_at(b.column(0).as_ref(), 0))
            .unwrap_or(0.0) as u64;

        // Nothing grouped, compared or totalled: the rows themselves.
        if spec.dimensions.is_empty() && spec.compare.is_empty() && spec.metrics.is_empty() {
            let names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
            let shown: Vec<String> = names
                .iter()
                .map(|n| format!("CAST({} AS VARCHAR)", quote_ident(n)))
                .collect();
            let sql = format!(
                "SELECT {} FROM ({kept}) AS f LIMIT {}",
                shown.join(", "),
                MAX_ROWS + 1
            );
            let mut rows = Vec::new();
            for batch in batches_of(session.query_params(&sql, &bound.0).await?)? {
                for row in 0..batch.num_rows() {
                    rows.push(
                        (0..batch.num_columns())
                            .map(|c| text_at(batch.column(c).as_ref(), row))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            let truncated = rows.len() > MAX_ROWS;
            rows.truncate(MAX_ROWS);
            return Ok(ChartResult {
                series: Vec::new(),
                groups: Vec::new(),
                all_groups: 0,
                matched,
                rows: Some(ChartRows {
                    columns: names,
                    rows,
                }),
                truncated,
            });
        }

        let counting = [ChartMetric {
            column: None,
            aggregate: ChartAggregate::Count,
        }];
        let metrics: &[ChartMetric] = if spec.metrics.is_empty() {
            &counting
        } else {
            &spec.metrics
        };
        let dims = spec
            .dimensions
            .iter()
            .map(|d| dimension(&columns, d))
            .collect::<Result<Vec<_>, _>>()?;
        let compared = spec
            .compare
            .iter()
            .map(|d| dimension(&columns, d))
            .collect::<Result<Vec<_>, _>>()?;
        let totals = metrics
            .iter()
            .map(|m| metric(&columns, m))
            .collect::<Result<Vec<_>, _>>()?;
        let named = |list: &[String], prefix: &str| -> Vec<String> {
            list.iter()
                .enumerate()
                .map(|(i, e)| format!("{e} AS oik_{prefix}{i}"))
                .collect()
        };
        let by: Vec<String> = dims.iter().chain(&compared).cloned().collect();
        let mut picked = named(&dims, "d");
        picked.extend(named(&compared, "c"));
        picked.extend(named(&totals, "m"));
        let grouped = format!(
            "SELECT {} FROM ({kept}) AS f{}",
            picked.join(", "),
            if by.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {}", by.join(", "))
            }
        );
        let compare_order: Vec<String> = (0..compared.len()).map(|i| format!("oik_c{i}")).collect();
        let sql = if dims.is_empty() {
            // One group: everything the filters keep.
            format!(
                "WITH g AS ({grouped}) \
                 SELECT g.*, 1 AS oik_n, 1 AS oik_all FROM g{} LIMIT {}",
                if compare_order.is_empty() {
                    String::new()
                } else {
                    format!(" ORDER BY {}", compare_order.join(", "))
                },
                MAX_CELLS + 1
            )
        } else {
            // The groups in order: by a metric over the whole group (whatever
            // is compared within it), else by their own values; then as many
            // as asked for.
            let sorted = match &spec.sort {
                Some(sort) => {
                    let total = totals.get(sort.metric as usize).ok_or_else(|| {
                        ApiError::invalid("the chart is ordered by a metric it has not")
                    })?;
                    Some((total.clone(), sort.descending))
                }
                None => None,
            };
            let keys: Vec<String> = (0..dims.len()).map(|i| format!("oik_d{i}")).collect();
            let mut order = Vec::new();
            if let Some((_, descending)) = &sorted {
                order.push(format!(
                    "oik_s {} NULLS LAST",
                    if *descending { "DESC" } else { "ASC" }
                ));
            }
            order.extend(keys.iter().cloned());
            let limit = bound.bind(Value::Int64(i64::from(
                spec.limit.unwrap_or(MAX_GROUPS).clamp(1, MAX_GROUPS),
            )));
            let on: Vec<String> = keys
                .iter()
                .map(|k| format!("g.{k} IS NOT DISTINCT FROM r.{k}"))
                .collect();
            let mut last = vec!["r.oik_n".to_string()];
            last.extend(compare_order.iter().map(|c| format!("g.{c}")));
            format!(
                "WITH g AS ({grouped}), \
                 k AS (SELECT {}, {} AS oik_s FROM ({kept}) AS f GROUP BY {}), \
                 r AS (SELECT *, row_number() OVER (ORDER BY {}) AS oik_n, \
                 COUNT(*) OVER () AS oik_all FROM k) \
                 SELECT g.*, r.oik_n, r.oik_all FROM g JOIN r ON {} \
                 WHERE r.oik_n <= {limit} ORDER BY {} LIMIT {}",
                named(&dims, "d").join(", "),
                sorted
                    .as_ref()
                    .map(|(total, _)| total.as_str())
                    .unwrap_or("0"),
                dims.join(", "),
                order.join(", "),
                on.join(" AND "),
                last.join(", "),
                MAX_CELLS + 1
            )
        };

        // One row for each group and compared value: laid out as groups,
        // each with a total for every series.
        let (nd, nc, nm) = (dims.len(), compared.len(), totals.len());
        let mut cells = Vec::new();
        let mut all_groups = 0u64;
        for batch in batches_of(session.query_params(&sql, &bound.0).await?)? {
            for row in 0..batch.num_rows() {
                let text = |c: usize| text_at(batch.column(c).as_ref(), row);
                let keys: Vec<Option<String>> = (0..nd).map(text).collect();
                let compare: Vec<Option<String>> = (nd..nd + nc).map(text).collect();
                let values: Vec<Option<f64>> = (nd + nc..nd + nc + nm)
                    .map(|c| number_at(batch.column(c).as_ref(), row))
                    .collect();
                let n = number_at(batch.column(nd + nc + nm).as_ref(), row).unwrap_or(0.0) as u64;
                all_groups =
                    number_at(batch.column(nd + nc + nm + 1).as_ref(), row).unwrap_or(0.0) as u64;
                cells.push((n, keys, compare, values));
            }
        }
        let truncated = cells.len() > MAX_CELLS;
        cells.truncate(MAX_CELLS);
        let compares: BTreeSet<Vec<Option<String>>> =
            cells.iter().map(|(_, _, c, _)| c.clone()).collect();
        let compares: Vec<Vec<Option<String>>> = compares.into_iter().collect();
        let series: Vec<ChartSeries> = compares
            .iter()
            .flat_map(|compare| {
                (0..nm).map(|m| ChartSeries {
                    compare: compare.clone(),
                    metric: m as u32,
                })
            })
            .collect();
        let mut groups: Vec<(u64, ChartGroup)> = Vec::new();
        for (n, keys, compare, values) in cells {
            if groups.last().is_none_or(|(last, _)| *last != n) {
                groups.push((
                    n,
                    ChartGroup {
                        keys,
                        values: vec![None; series.len()],
                    },
                ));
            }
            let at = compares.iter().position(|c| *c == compare).unwrap_or(0) * nm;
            if let Some((_, group)) = groups.last_mut() {
                for (m, value) in values.into_iter().enumerate() {
                    group.values[at + m] = value;
                }
            }
        }
        Ok(ChartResult {
            series,
            all_groups: all_groups.max(groups.len() as u64),
            groups: groups.into_iter().map(|(_, group)| group).collect(),
            matched,
            rows: None,
            truncated,
        })
    }

    /// The values a column of an analysis table holds, in order, for a
    /// filter to choose from.
    pub async fn analysis_values(
        &self,
        caller: &Caller,
        req: AnalysisValues,
    ) -> Result<Vec<String>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (schema, roles) = self.analysis_columns(&req.space, &req.table).await?;
        let column = Columns {
            schema: &schema,
            roles: &roles,
        }
        .quoted(&req.column)?;
        let sql = format!(
            "SELECT DISTINCT CAST({column} AS VARCHAR) AS value FROM {}.{} \
             WHERE {column} IS NOT NULL ORDER BY 1 LIMIT {MAX_OFFERED}",
            quote_ident(&req.space),
            quote_ident(&req.table)
        );
        let QueryResult::Batches { batches, .. } = self.session(None).sql(&sql).await? else {
            return Ok(Vec::new());
        };
        Ok(batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows()).filter_map(|row| text_at(batch.column(0).as_ref(), row))
            })
            .collect())
    }
}
