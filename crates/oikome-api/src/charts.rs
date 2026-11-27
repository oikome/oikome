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

//! Charts of an analysis table. A chart is a description: its time (a
//! column of time at a grain, over a range), the dimensions its rows are
//! grouped by together with the time, the dimensions pivoted (laid across
//! each group, a series or a column for each value), the metrics totalled, the filters, the order and how many groups
//! to show. Every group is worked out from the rows, so an average or a
//! median is right at any grain.
//!
//! No SQL comes from the page. The query is written here, from the
//! description: every column is one of the table's own and is quoted, every
//! value a filter gives is a bound parameter, and everything else is this
//! file's own text. It is as plain as the chart: one GROUP BY, with a first
//! step only when pivoted values need the top groups picked by their own
//! totals. DuckDB computes; the page draws the answer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use arrow_array::{Array, ArrayRef, Float64Array, StringArray};
use arrow_schema::{DataType, Schema};
use chrono::{Datelike, Days, Months, NaiveDate, NaiveDateTime, TimeDelta, Weekday};
use oikome_engine::Value;
use oikome_node::{OikomeSession, QueryResult};
use oikome_semantic::rewrite::quote_ident;

use crate::file_analysis::{ReadAs, SOURCE_FILE};
use crate::{
    AnalysisChart, AnalysisValues, Api, ApiError, Caller, ChartAggregate, ChartColumn, ChartFilter,
    ChartGroup, ChartLevel, ChartLimit, ChartMetric, ChartQuery, ChartResult, ChartRows,
    ChartSeries, ChartSpec, ChartableTable, ColumnType, DateGrain, MetricShow, TextMatch,
    TimeRange,
};

/// How much a chart may ask for.
const MAX_DIMENSIONS: usize = 4;
const MAX_PIVOTS: usize = 2;
const MAX_METRICS: usize = 8;
const MAX_FILTERS: usize = 12;
const MAX_FILTER_VALUES: usize = 500;
const MAX_VALUE_BYTES: usize = 1024;
/// The groups shown unless a number is chosen or all of them; the cells (a
/// group and a pivoted value) answered with, likewise; and the most rows of
/// a chart that groups nothing.
const GROUPS: u32 = 1000;
const CELLS: usize = 20_000;
const MAX_ROWS: usize = 200;
/// The most values of a column offered to a filter.
const MAX_OFFERED: usize = 500;

/// A table's columns as a chart may use them: as DuckDB read them, or as
/// the owner has them read.
struct Columns<'a> {
    schema: &'a Schema,
    types: &'a BTreeMap<String, ReadAs>,
}

impl Columns<'_> {
    fn read_as(&self, name: &str) -> Option<ColumnType> {
        self.types.get(name).map(|t| t.kind)
    }

    /// A column's values as the chart reads them: as they are, or converted
    /// to the type the owner chose, a value that does not convert empty.
    fn value(&self, from: &str, name: &str) -> String {
        let column = col(from, name);
        match self.types.get(name) {
            Some(read_as) => format!("TRY_CAST({column} AS {})", read_as.sql),
            None => column,
        }
    }

    /// A column as a group is keyed by it, named as the column.
    fn key(&self, from: &str, name: &str) -> String {
        match self.read_as(name) {
            Some(_) => format!("{} AS {}", self.value(from, name), quote_ident(name)),
            None => col(from, name),
        }
    }

    /// Whether the table has the column; the answer for a chart that names
    /// one it has not.
    fn has(&self, name: &str) -> Result<(), ApiError> {
        self.schema
            .field_with_name(name)
            .map(|_| ())
            .map_err(|_| ApiError::invalid(format!("the table has no column {name}")))
    }

    fn data_type(&self, name: &str) -> Option<&DataType> {
        self.schema
            .field_with_name(name)
            .ok()
            .map(|f| f.data_type())
    }

    fn is_number(&self, name: &str) -> bool {
        if let Some(read_as) = self.read_as(name) {
            return matches!(
                read_as,
                ColumnType::Integer | ColumnType::Number | ColumnType::Decimal
            );
        }
        matches!(
            self.data_type(name),
            Some(
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
        )
    }

    /// A column of time: a date or a timestamp, as DuckDB read it.
    fn is_time(&self, name: &str) -> bool {
        if let Some(read_as) = self.read_as(name) {
            return matches!(read_as, ColumnType::Date | ColumnType::Timestamp);
        }
        matches!(
            self.data_type(name),
            Some(DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _))
        )
    }

    /// Whether a column of time has times of day: a timestamp, not a date.
    fn is_timestamp(&self, name: &str) -> bool {
        if let Some(read_as) = self.read_as(name) {
            return read_as == ColumnType::Timestamp;
        }
        matches!(self.data_type(name), Some(DataType::Timestamp(_, _)))
    }

    fn is_text(&self, name: &str) -> bool {
        if let Some(read_as) = self.read_as(name) {
            return read_as == ColumnType::Text;
        }
        matches!(
            self.data_type(name),
            Some(DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
        )
    }
}

/// A column as the query names it: quoted, and, where two tables are read,
/// with the name of the one it is read from (`t."amount"`).
fn col(from: &str, name: &str) -> String {
    format!("{from}{}", quote_ident(name))
}

/// A query being written: its bound values, in the order they are named.
/// The same value bound twice is one placeholder.
#[derive(Default)]
struct Bound(Vec<Value>);

impl Bound {
    /// Bind a value; the answer is the placeholder that stands for it.
    fn bind(&mut self, value: Value) -> String {
        let at = match self.0.iter().position(|v| *v == value) {
            Some(at) => at,
            None => {
                self.0.push(value);
                self.0.len() - 1
            }
        };
        format!("?{}", at + 1)
    }
}

/// A column of time at a grain, as the period it falls in, written in a
/// standard way: 2026, 2026Q3, 2026-10, 2026-W40 (an ISO week), 2026-10-05,
/// 2026-10-05 22, 2026-10-05 22:04, 2026-10-05 22:05:30.
fn at_grain(column: &str, grain: DateGrain) -> String {
    let format = match grain {
        DateGrain::Year => "%Y",
        DateGrain::Quarter => {
            return format!("strftime({column}, '%Y') || 'Q' || CAST(quarter({column}) AS VARCHAR)")
        }
        DateGrain::Month => "%Y-%m",
        DateGrain::Week => "%G-W%V",
        DateGrain::Day => "%Y-%m-%d",
        DateGrain::Hour => "%Y-%m-%d %H",
        DateGrain::Minute => "%Y-%m-%d %H:%M",
        DateGrain::Second => "%Y-%m-%d %H:%M:%S",
    };
    format!("strftime({column}, '{format}')")
}

/// Whether a grain is finer than a day: for a timestamp only.
fn within_a_day(grain: DateGrain) -> bool {
    matches!(
        grain,
        DateGrain::Hour | DateGrain::Minute | DateGrain::Second
    )
}

/// When a period written as [`at_grain`] writes it begins. A quarter written
/// 2026-Q3 and a week written as the Monday it starts on, as they were
/// written before, are read too.
fn period_start(grain: DateGrain, key: &str) -> Option<NaiveDateTime> {
    let midnight = |day: NaiveDate| day.and_hms_opt(0, 0, 0);
    let year: i32 = key.get(..4)?.parse().ok()?;
    let rest = key.get(4..)?;
    match grain {
        DateGrain::Year => rest
            .is_empty()
            .then(|| NaiveDate::from_ymd_opt(year, 1, 1))
            .flatten()
            .and_then(midnight),
        DateGrain::Quarter => {
            let quarter: u32 = rest
                .strip_prefix('-')
                .unwrap_or(rest)
                .strip_prefix('Q')?
                .parse()
                .ok()?;
            if !(1..=4).contains(&quarter) {
                return None;
            }
            NaiveDate::from_ymd_opt(year, quarter * 3 - 2, 1).and_then(midnight)
        }
        DateGrain::Month => (key.len() == 7)
            .then(|| NaiveDate::from_ymd_opt(year, key.get(5..7)?.parse().ok()?, 1))
            .flatten()
            .and_then(midnight),
        DateGrain::Week => match rest.strip_prefix("-W") {
            Some(week) => NaiveDate::from_isoywd_opt(year, week.parse().ok()?, Weekday::Mon),
            None => NaiveDate::parse_from_str(key, "%Y-%m-%d")
                .ok()
                .map(|day| day - Days::new(u64::from(day.weekday().num_days_from_monday()))),
        }
        .and_then(midnight),
        DateGrain::Day => NaiveDate::parse_from_str(key, "%Y-%m-%d")
            .ok()
            .and_then(midnight),
        DateGrain::Hour => {
            NaiveDateTime::parse_from_str(&format!("{key}:00:00"), "%Y-%m-%d %H:%M:%S").ok()
        }
        DateGrain::Minute => {
            NaiveDateTime::parse_from_str(&format!("{key}:00"), "%Y-%m-%d %H:%M:%S").ok()
        }
        DateGrain::Second => NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M:%S").ok(),
    }
}

/// The period after the one beginning `at`.
fn period_after(grain: DateGrain, at: NaiveDateTime) -> Option<NaiveDateTime> {
    match grain {
        DateGrain::Year => at.checked_add_months(Months::new(12)),
        DateGrain::Quarter => at.checked_add_months(Months::new(3)),
        DateGrain::Month => at.checked_add_months(Months::new(1)),
        DateGrain::Week => at.checked_add_days(Days::new(7)),
        DateGrain::Day => at.checked_add_days(Days::new(1)),
        DateGrain::Hour => at.checked_add_signed(TimeDelta::try_hours(1)?),
        DateGrain::Minute => at.checked_add_signed(TimeDelta::try_minutes(1)?),
        DateGrain::Second => at.checked_add_signed(TimeDelta::try_seconds(1)?),
    }
}

/// The period a moment falls in, written as [`at_grain`] writes it.
fn period_of(grain: DateGrain, at: NaiveDateTime) -> String {
    match grain {
        DateGrain::Year => at.format("%Y").to_string(),
        DateGrain::Quarter => format!("{}Q{}", at.year(), (at.month() - 1) / 3 + 1),
        DateGrain::Month => at.format("%Y-%m").to_string(),
        DateGrain::Week => {
            let week = at.date().iso_week();
            format!("{:04}-W{:02}", week.year(), week.week())
        }
        DateGrain::Day => at.format("%Y-%m-%d").to_string(),
        DateGrain::Hour => at.format("%Y-%m-%d %H").to_string(),
        DateGrain::Minute => at.format("%Y-%m-%d %H:%M").to_string(),
        DateGrain::Second => at.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

/// Every period at `grain` from the one `from` is in to the one `to` is in;
/// none when they are more than `most`.
fn periods(
    grain: DateGrain,
    from: NaiveDateTime,
    to: NaiveDateTime,
    most: usize,
) -> Option<Vec<String>> {
    let last = period_start(grain, &period_of(grain, to))?;
    let mut next = period_start(grain, &period_of(grain, from));
    let mut out = Vec::new();
    while let Some(at) = next {
        if at > last {
            break;
        }
        if out.len() >= most {
            return None;
        }
        out.push(period_of(grain, at));
        next = period_after(grain, at);
    }
    Some(out)
}

/// The answer to a chart's query as cells: each row's group keys (the first
/// `along` columns), its pivoted values (the next `pivots`) and its totals.
async fn cells_of(
    session: &OikomeSession,
    sql: &str,
    bound: &Bound,
    along: usize,
    pivots: usize,
) -> Result<Vec<Cell>, ApiError> {
    let mut cells = Vec::new();
    let QueryResult::Batches { batches, .. } = session.query_params(sql, &bound.0).await? else {
        return Err(ApiError::internal("a chart's query returned no rows"));
    };
    for batch in batches {
        let shown: Vec<Vec<Option<String>>> = batch.columns()[..along + pivots]
            .iter()
            .map(texts)
            .collect();
        let totals: Vec<Vec<Option<f64>>> = batch.columns()[along + pivots..]
            .iter()
            .map(numbers)
            .collect();
        for row in 0..batch.num_rows() {
            cells.push((
                shown[..along].iter().map(|c| c[row].clone()).collect(),
                shown[along..].iter().map(|c| c[row].clone()).collect(),
                totals.iter().map(|c| c[row]).collect(),
            ));
        }
    }
    Ok(cells)
}

/// A metric as SQL writes it, and as it is named: `SUM(amount)`,
/// `COUNT(DISTINCT member)`, `COUNT(*)`.
/// A metric's name as the query shows it: its total as SQL writes it, and
/// how it is shown, as the app names it (a period's change by the chart's
/// grain: MoM for a month).
fn metric_name(metric: &ChartMetric, grain: Option<DateGrain>) -> String {
    let total = match &metric.column {
        None => "COUNT(*)".to_string(),
        Some(name) => match metric.aggregate {
            ChartAggregate::Distinct => format!("COUNT(DISTINCT {name})"),
            other => format!("{}({name})", function(other)),
        },
    };
    match metric.show {
        None => total,
        Some(show) => format!("{total} ({})", shown_as(show, grain)),
    }
}

/// How a metric is shown, in the words BI tools and finance use: % of
/// Total, YoY %, Running Total. A change from the period before is named
/// by the grain (YoY, QoQ, MoM, WoW, DoD; PoP finer than a day).
fn shown_as(show: MetricShow, grain: Option<DateGrain>) -> String {
    let over = match grain {
        Some(DateGrain::Year) => "YoY",
        Some(DateGrain::Quarter) => "QoQ",
        Some(DateGrain::Month) => "MoM",
        Some(DateGrain::Week) => "WoW",
        Some(DateGrain::Day) => "DoD",
        _ => "PoP",
    };
    match show {
        MetricShow::ShareOfAll => "% of Total".to_string(),
        MetricShow::ShareOfRow => "% of Row Total".to_string(),
        MetricShow::ShareOfColumn => "% of Column Total".to_string(),
        MetricShow::ChangeFromPrevious => format!("{over} Change"),
        MetricShow::GrowthFromPrevious => format!("{over} %"),
        MetricShow::ChangeFromYearBefore => "YoY Change".to_string(),
        MetricShow::GrowthFromYearBefore => "YoY %".to_string(),
        MetricShow::RunningTotal => "Running Total".to_string(),
        MetricShow::MovingAverage => "Moving Average".to_string(),
        MetricShow::Rank => "Rank".to_string(),
    }
}

/// Whether a metric is shown against an earlier period: the period before,
/// or the same one a year before.
fn over_time(show: Option<MetricShow>) -> Option<bool> {
    match show {
        Some(MetricShow::ChangeFromPrevious | MetricShow::GrowthFromPrevious) => Some(false),
        Some(MetricShow::ChangeFromYearBefore | MetricShow::GrowthFromYearBefore) => Some(true),
        _ => None,
    }
}

/// A period's unit for `date_trunc`, how far back the period before is,
/// and how far back the same period a year before is.
fn steps_of(grain: DateGrain) -> (&'static str, &'static str, &'static str) {
    match grain {
        DateGrain::Year => ("year", "1 YEAR", "1 YEAR"),
        DateGrain::Quarter => ("quarter", "3 MONTH", "1 YEAR"),
        DateGrain::Month => ("month", "1 MONTH", "1 YEAR"),
        DateGrain::Week => ("week", "7 DAY", "364 DAY"),
        DateGrain::Day => ("day", "1 DAY", "1 YEAR"),
        DateGrain::Hour => ("hour", "1 HOUR", "1 YEAR"),
        DateGrain::Minute => ("minute", "1 MINUTE", "1 YEAR"),
        DateGrain::Second => ("second", "1 SECOND", "1 YEAR"),
    }
}

fn function(aggregate: ChartAggregate) -> &'static str {
    match aggregate {
        ChartAggregate::Sum => "SUM",
        ChartAggregate::Average => "AVG",
        ChartAggregate::Median => "MEDIAN",
        ChartAggregate::Min => "MIN",
        ChartAggregate::Max => "MAX",
        ChartAggregate::Count | ChartAggregate::Distinct => "COUNT",
    }
}

/// A metric as the query computes it, its columns read from `from`.
fn metric(columns: &Columns, metric: &ChartMetric, from: &str) -> Result<String, ApiError> {
    let Some(name) = &metric.column else {
        return match metric.aggregate {
            ChartAggregate::Count => Ok("COUNT(*)".to_string()),
            _ => Err(ApiError::invalid("a metric without a column counts rows")),
        };
    };
    columns.has(name)?;
    let column = columns.value(from, name);
    match metric.aggregate {
        ChartAggregate::Count => Ok(format!("COUNT({column})")),
        ChartAggregate::Distinct => Ok(format!("COUNT(DISTINCT {column})")),
        aggregate if columns.is_number(name) => Ok(format!("{}({column})", function(aggregate))),
        _ => Err(ApiError::invalid(format!(
            "{name} is not a number: it can be counted"
        ))),
    }
}

/// The days a range is from and to: only a custom range has them, whole
/// days written 2026-08-01, in order.
fn days_of(
    range: TimeRange,
    from: &Option<String>,
    to: &Option<String>,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>), ApiError> {
    let day = |text: &Option<String>| -> Result<Option<NaiveDate>, ApiError> {
        text.as_deref()
            .map(|t| {
                NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d")
                    .map_err(|_| ApiError::invalid(format!("{t} is not a day (2026-08-01)")))
            })
            .transpose()
    };
    let ends = (day(from)?, day(to)?);
    if range != TimeRange::Custom && (ends.0.is_some() || ends.1.is_some()) {
        return Err(ApiError::invalid(
            "from and to are the ends of a custom range",
        ));
    }
    if let (Some(from), Some(to)) = ends {
        if from > to {
            return Err(ApiError::invalid("a range ends after it starts"));
        }
    }
    Ok(ends)
}

/// The rows whose time `at` is within a range: recent days, this year, or
/// whole days from one to another (the last one up to its end, for a
/// timestamp too).
fn within_range(
    at: &str,
    range: TimeRange,
    ends: (Option<NaiveDate>, Option<NaiveDate>),
    bound: &mut Bound,
) -> Vec<String> {
    let since = |days: u32| format!("{at} >= current_date - INTERVAL {days} DAY");
    let mut out = Vec::new();
    match range {
        TimeRange::All => {}
        TimeRange::Last7Days => out.push(since(7)),
        TimeRange::Last30Days => out.push(since(30)),
        TimeRange::Last90Days => out.push(since(90)),
        TimeRange::Last365Days => out.push(since(365)),
        TimeRange::ThisYear => out.push(format!("year({at}) = year(current_date)")),
        TimeRange::Custom => {
            if let Some(first) = ends.0 {
                let day = bound.bind(Value::Text(first.format("%Y-%m-%d").to_string()));
                out.push(format!("{at} >= CAST({day} AS DATE)"));
            }
            if let Some(last) = ends.1 {
                let day = bound.bind(Value::Text(last.format("%Y-%m-%d").to_string()));
                out.push(format!("{at} < CAST({day} AS DATE) + INTERVAL 1 DAY"));
            }
        }
    }
    out
}

/// A number a filter keeps from or up to.
fn number_of(text: &str) -> Result<f64, ApiError> {
    text.trim()
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite())
        .ok_or_else(|| ApiError::invalid(format!("{text} is not a number")))
}

/// A pattern for text that matches, its own `%`, `_` and `\` taken as
/// they are.
fn like_pattern(how: TextMatch, text: &str) -> String {
    let plain = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    match how {
        TextMatch::Contains | TextMatch::NotContains => format!("%{plain}%"),
        TextMatch::StartsWith => format!("{plain}%"),
        TextMatch::EndsWith => format!("%{plain}"),
    }
}

/// A filter as a condition, its columns read from `from` and its values
/// bound; `None` when it asks nothing.
fn filter(
    columns: &Columns,
    filter: &ChartFilter,
    from: &str,
    bound: &mut Bound,
) -> Result<Option<String>, ApiError> {
    let name = &filter.column;
    columns.has(name)?;
    let column = columns.value(from, name);
    let ranged = filter.min.is_some() || filter.max.is_some();
    let kinds = [
        !filter.values.is_empty(),
        ranged,
        filter.period.is_some(),
        filter.text.is_some(),
        filter.empty.is_some(),
        filter.dates.is_some(),
    ];
    if kinds.iter().filter(|k| **k).count() > 1 {
        return Err(ApiError::invalid(format!(
            "the filter on {name} is of more than one kind"
        )));
    }
    let exclude = filter.exclude == Some(true);
    if exclude && filter.values.is_empty() {
        return Err(ApiError::invalid(format!(
            "the filter on {name} excludes values it does not name"
        )));
    }
    // Empty or not: for text, empty text is empty too.
    if let Some(empty) = filter.empty {
        let text = columns.is_text(name);
        return Ok(Some(match (empty, text) {
            (true, true) => format!("({column} IS NULL OR {column} = '')"),
            (true, false) => format!("{column} IS NULL"),
            (false, true) => format!("({column} IS NOT NULL AND {column} <> '')"),
            (false, false) => format!("{column} IS NOT NULL"),
        }));
    }
    // Text that matches, whatever its case, bound as a pattern.
    if let Some(text) = &filter.text {
        if !columns.is_text(name) {
            return Err(ApiError::invalid(format!(
                "{name} is not text: choose the values to keep"
            )));
        }
        if text.value.is_empty() || text.value.len() > MAX_VALUE_BYTES {
            return Err(ApiError::invalid(format!(
                "the filter on {name} needs some text, not too long"
            )));
        }
        let pattern = bound.bind(Value::Text(like_pattern(text.how, &text.value)));
        return Ok(Some(match text.how {
            TextMatch::NotContains => {
                format!("({column} IS NULL OR {column} NOT ILIKE {pattern} ESCAPE '\\')")
            }
            _ => format!("{column} ILIKE {pattern} ESCAPE '\\'"),
        }));
    }
    // Days of a column of time.
    if let Some(dates) = &filter.dates {
        if !columns.is_time(name) {
            return Err(ApiError::invalid(format!("{name} is not a column of time")));
        }
        let ends = days_of(dates.range, &dates.from, &dates.to)?;
        let within = within_range(&column, dates.range, ends, bound);
        return Ok((!within.is_empty()).then(|| within.join(" AND ")));
    }
    // A period of a column of time: the rows whose time falls in it.
    if let Some(period) = &filter.period {
        if !columns.is_time(name) {
            return Err(ApiError::invalid(format!(
                "{name} is not a column of time: it has no periods"
            )));
        }
        if within_a_day(period.grain) && !columns.is_timestamp(name) {
            return Err(ApiError::invalid(format!(
                "{name} is a date: it has no hours, minutes or seconds"
            )));
        }
        let Some(start) = period_start(period.grain, &period.key) else {
            return Err(ApiError::invalid(format!(
                "{} is not a period of {name}",
                period.key
            )));
        };
        let key = bound.bind(Value::Text(period_of(period.grain, start)));
        return Ok(Some(format!("{} = {key}", at_grain(&column, period.grain))));
    }
    if !filter.values.is_empty() {
        if filter.values.len() > MAX_FILTER_VALUES
            || filter.values.iter().any(|v| v.len() > MAX_VALUE_BYTES)
        {
            return Err(ApiError::invalid(format!(
                "the filter on {name} has too many values"
            )));
        }
        // Text is compared as it is; another column as the text it is shown as.
        let shown = if columns.is_text(name) {
            column
        } else {
            format!("CAST({column} AS VARCHAR)")
        };
        let values: Vec<String> = filter
            .values
            .iter()
            .map(|v| bound.bind(Value::Text(v.clone())))
            .collect();
        let any = match values.as_slice() {
            [one] => format!("{shown} = {one}"),
            _ => format!("{shown} IN ({})", values.join(", ")),
        };
        // Excluded: none of them, the empty values kept.
        return Ok(Some(if exclude {
            format!("({shown} IS NULL OR NOT ({any}))")
        } else {
            any
        }));
    }
    if !ranged {
        return Ok(None);
    }
    if !columns.is_number(name) {
        return Err(ApiError::invalid(format!(
            "{name} is not a number: choose the values to keep"
        )));
    }
    let mut ends = Vec::new();
    for (end, compare) in [(&filter.min, ">="), (&filter.max, "<=")] {
        let Some(text) = end else { continue };
        let value = bound.bind(Value::Float64(number_of(text)?));
        ends.push(format!("{column} {compare} {value}"));
    }
    Ok(Some(ends.join(" AND ")))
}

/// The query as it runs, for the owner to read: its text, and the value
/// each placeholder (`?1`, `?2`, …) is bound to.
fn query_of(sql: &str, bound: &Bound) -> ChartQuery {
    ChartQuery {
        sql: sql.to_string(),
        params: bound
            .0
            .iter()
            .map(|value| match value {
                Value::Null => "NULL".to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Int64(n) => n.to_string(),
                Value::Float64(n) => n.to_string(),
                Value::Text(text) => format!("'{}'", text.replace('\'', "''")),
                Value::Blob(bytes) => format!("<{} bytes>", bytes.len()),
            })
            .collect(),
    }
}

/// A column of any type as the text it is shown as: a timestamp as DuckDB
/// writes one (`2026-08-03 10:00:00`), so that a value shown is a value a
/// filter keeps.
fn texts(column: &ArrayRef) -> Vec<Option<String>> {
    let stamp = matches!(column.data_type(), DataType::Timestamp(_, _));
    let Ok(cast) = arrow_cast::cast(column, &DataType::Utf8) else {
        return vec![None; column.len()];
    };
    let Some(cast) = cast.as_any().downcast_ref::<StringArray>() else {
        return vec![None; column.len()];
    };
    (0..cast.len())
        .map(|row| {
            (!cast.is_null(row)).then(|| {
                let text = cast.value(row);
                if stamp {
                    text.replacen('T', " ", 1)
                } else {
                    text.to_string()
                }
            })
        })
        .collect()
}

/// A column of numbers of any width (a count, a sum of integers, a
/// decimal) as numbers.
fn numbers(column: &ArrayRef) -> Vec<Option<f64>> {
    let Ok(cast) = arrow_cast::cast(column, &DataType::Float64) else {
        return vec![None; column.len()];
    };
    let Some(cast) = cast.as_any().downcast_ref::<Float64Array>() else {
        return vec![None; column.len()];
    };
    (0..cast.len())
        .map(|row| {
            (!cast.is_null(row))
                .then(|| cast.value(row))
                .filter(|n| n.is_finite())
        })
        .collect()
}

/// One row of a chart's answer: its group's keys, the pivoted values, and
/// a total for each metric.
type Cell = (Vec<Option<String>>, Vec<Option<String>>, Vec<Option<f64>>);

/// Lines of a query, each set in by `pad`.
fn indent(text: &str, pad: &str) -> String {
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `1, 2, 3`: the first `n` columns of a query's answer, by their places.
fn places(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|i| (i + 1).to_string()).collect()
}

impl Api {
    /// The analysis tables of `space` that can be charted, with what each
    /// column holds.
    pub(crate) async fn chartable(&self, space: &str) -> Result<Vec<ChartableTable>, ApiError> {
        let mut out = Vec::new();
        for (table, rows, charts) in self.analysis_kept(space)? {
            let Ok((schema, _)) = self.analysis_columns(space, &table).await else {
                continue;
            };
            let types = self.analysis_types(space, &table).await?;
            let columns = Columns {
                schema: &schema,
                types: &types,
            };
            out.push(ChartableTable {
                collection: space.to_string(),
                rows,
                columns: schema
                    .fields()
                    .iter()
                    .map(|f| f.name())
                    .filter(|name| *name != SOURCE_FILE)
                    .map(|name| ChartColumn {
                        kind: if columns.is_number(name) {
                            "number"
                        } else if columns.is_time(name) {
                            "time"
                        } else {
                            "text"
                        }
                        .to_string(),
                        name: name.clone(),
                    })
                    .collect(),
                charts,
                table,
            });
        }
        Ok(out)
    }

    /// What a chart of an analysis table shows: its groups and series,
    /// computed by the node from the chart's description.
    pub async fn analysis_chart(
        &self,
        caller: &Caller,
        req: AnalysisChart,
    ) -> Result<ChartResult, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let owner = self.session(None);
        self.compute_chart(&owner, &req.space, &req.table, &req.spec)
            .await
    }

    /// A chart of an analysis table, its query run in `session`: the
    /// owner's, or an assistant's, whose grants then decide what it reads.
    pub(crate) async fn compute_chart(
        &self,
        session: &OikomeSession,
        space: &str,
        table: &str,
        spec: &ChartSpec,
    ) -> Result<ChartResult, ApiError> {
        let (schema, _) = self.analysis_columns(space, table).await?;
        let types = self.analysis_types(space, table).await?;
        let columns = Columns {
            schema: &schema,
            types: &types,
        };
        if spec.dimensions.len() > MAX_DIMENSIONS
            || spec.pivots.len() > MAX_PIVOTS
            || spec.metrics.len() > MAX_METRICS
            || spec.filters.len() > MAX_FILTERS
            || spec.metric_filters.len() > MAX_FILTERS
        {
            return Err(ApiError::invalid("the chart asks for too much at once"));
        }
        // The time is a column of time, seen at a grain; any other column
        // is a dimension or a pivot, once.
        // A custom range's ends are days, in order; other ranges have none.
        let mut ends: (Option<NaiveDate>, Option<NaiveDate>) = (None, None);
        if let Some(time) = &spec.time {
            columns.has(&time.column)?;
            if !columns.is_time(&time.column) {
                return Err(ApiError::invalid(format!(
                    "{} is not a column of time",
                    time.column
                )));
            }
            if within_a_day(time.grain) && !columns.is_timestamp(&time.column) {
                return Err(ApiError::invalid(format!(
                    "{} is a date: it has no hours, minutes or seconds",
                    time.column
                )));
            }
            ends = days_of(time.range, &time.from, &time.to)?;
        }
        let mut seen: BTreeSet<&str> = spec.time.iter().map(|t| t.column.as_str()).collect();
        for d in spec.dimensions.iter().chain(&spec.pivots) {
            columns.has(&d.column)?;
            if !seen.insert(d.column.as_str()) {
                return Err(ApiError::invalid(format!(
                    "{} is in the chart already: a column is grouped once",
                    d.column
                )));
            }
        }
        let grain = spec.time.as_ref().map(|t| t.grain);
        let mut named = BTreeSet::new();
        for m in &spec.metrics {
            if !named.insert(metric_name(m, grain)) {
                return Err(ApiError::invalid(format!(
                    "the chart has {} already",
                    metric_name(m, grain)
                )));
            }
        }
        let table_name = format!("{}.{}", quote_ident(space), quote_ident(table));
        let mut bound = Bound::default();
        // The rows kept, their columns read from `from`.
        let conditions = |from: &str, bound: &mut Bound| -> Result<Vec<String>, ApiError> {
            let mut out = Vec::new();
            for each in &spec.filters {
                if let Some(condition) = filter(&columns, each, from, bound)? {
                    out.push(condition);
                }
            }
            if let Some(time) = &spec.time {
                let at = columns.value(from, &time.column);
                out.extend(within_range(&at, time.range, ends, bound));
            }
            Ok(out)
        };
        let kept = conditions("", &mut bound)?;
        let where_of = |list: &[String]| {
            if list.is_empty() {
                String::new()
            } else {
                format!("\nWHERE {}", list.join("\n  AND "))
            }
        };
        let batches_of = |result: QueryResult| match result {
            QueryResult::Batches { batches, .. } => Ok(batches),
            _ => Err(ApiError::internal("a chart's query returned no rows")),
        };

        // How many rows the filters keep, whatever is made of them.
        let counted = session
            .query_params(
                &format!("SELECT COUNT(*) FROM {table_name}{}", where_of(&kept)),
                &bound.0,
            )
            .await?;
        let matched = batches_of(counted)?
            .iter()
            .find(|b| b.num_rows() > 0)
            .and_then(|b| numbers(b.column(0)).first().copied().flatten())
            .unwrap_or(0.0) as u64;

        // Nothing grouped, pivoted or totalled: the rows themselves.
        if spec.time.is_none()
            && spec.dimensions.is_empty()
            && spec.pivots.is_empty()
            && spec.metrics.is_empty()
        {
            if !spec.metric_filters.is_empty() {
                return Err(ApiError::invalid(
                    "a filter on a metric keeps groups: the chart groups nothing",
                ));
            }
            // Every column as the chart reads it: converted where the owner chose.
            let shown = if types.is_empty() {
                "*".to_string()
            } else {
                schema
                    .fields()
                    .iter()
                    .map(|f| columns.key("", f.name()))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let sql = format!(
                "SELECT {shown}\nFROM {table_name}{}\nLIMIT {MAX_ROWS}",
                where_of(&kept)
            );
            let query = query_of(&sql, &bound);
            let names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
            let mut rows = Vec::new();
            for batch in batches_of(session.query_params(&sql, &bound.0).await?)? {
                let cells: Vec<Vec<Option<String>>> = batch.columns().iter().map(texts).collect();
                for row in 0..batch.num_rows() {
                    rows.push(cells.iter().map(|c| c[row].clone()).collect::<Vec<_>>());
                }
            }
            let truncated = matched > rows.len() as u64;
            return Ok(ChartResult {
                query,
                columns: Vec::new(),
                series: Vec::new(),
                groups: Vec::new(),
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
            show: None,
        }];
        let metrics: &[ChartMetric] = if spec.metrics.is_empty() {
            &counting
        } else {
            &spec.metrics
        };
        // Groups kept by a metric's total (HAVING), read from `from`.
        let having = |from: &str, bound: &mut Bound| -> Result<String, ApiError> {
            let mut out = Vec::new();
            for each in &spec.metric_filters {
                let m = metrics
                    .get(each.metric as usize)
                    .ok_or_else(|| ApiError::invalid("the chart filters by a metric it has not"))?;
                let total = metric(&columns, m, from)?;
                for (end, compare) in [(&each.min, ">="), (&each.max, "<=")] {
                    let Some(text) = end else { continue };
                    let value = bound.bind(Value::Float64(number_of(text)?));
                    out.push(format!("{total} {compare} {value}"));
                }
            }
            Ok(if out.is_empty() {
                String::new()
            } else {
                format!("\nHAVING {}", out.join(" AND "))
            })
        };
        let having_plain = having("", &mut bound)?;
        let having_ranked = having("t.", &mut bound)?;
        // What each group is keyed by: the time at its grain, then the
        // dimensions; then the pivots within it.
        let mut keyed: Vec<ChartLevel> = spec
            .time
            .iter()
            .map(|t| ChartLevel {
                column: t.column.clone(),
                grain: Some(t.grain),
            })
            .collect();
        keyed.extend(spec.dimensions.iter().map(|d| ChartLevel {
            column: d.column.clone(),
            grain: None,
        }));
        let keys = |from: &str| -> Vec<String> {
            keyed
                .iter()
                .map(|level| match level.grain {
                    Some(grain) => format!(
                        "{} AS {}",
                        at_grain(&columns.value(from, &level.column), grain),
                        quote_ident(&level.column)
                    ),
                    None => columns.key(from, &level.column),
                })
                .chain(spec.pivots.iter().map(|d| columns.key(from, &d.column)))
                .collect()
        };
        // Shown beside the others (a share, a change, a running total): the
        // grouped rows are a first step `g`, each total in it by its place,
        // and the query shows them from it. Otherwise one GROUP BY, as before.
        let shown = metrics.iter().any(|m| m.show.is_some());
        let raw = |i: usize| format!("__m{i}");
        let totals = |from: &str| -> Result<Vec<String>, ApiError> {
            metrics
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let name = if shown { raw(i) } else { metric_name(m, grain) };
                    metric(&columns, m, from).map(|sql| format!("{sql} AS {}", quote_ident(&name)))
                })
                .collect()
        };
        // A change over time compares a period with an earlier one: the time
        // must be along, and the first step keeps when each period starts.
        let compared = metrics.iter().any(|m| over_time(m.show).is_some());
        if compared && spec.time.is_none() {
            return Err(ApiError::invalid(
                "a change from an earlier period needs the time along the chart",
            ));
        }
        let start = |from: &str| -> Option<String> {
            let time = spec.time.as_ref().filter(|_| compared)?;
            let (unit, _, _) = steps_of(time.grain);
            Some(format!(
                "MIN(CAST(date_trunc('{unit}', {}) AS TIMESTAMP)) AS \"__start\"",
                columns.value(from, &time.column)
            ))
        };
        let (nk, nc, nd) = (keyed.len(), spec.pivots.len(), spec.dimensions.len());
        let timed = spec.time.is_some();
        // What the query shows from the first step: each key as it is, each
        // metric as it is shown.
        let named = |name: &str| format!("g.{}", quote_ident(name));
        let along: Vec<String> = keyed.iter().map(|l| named(&l.column)).collect();
        let pivoted: Vec<String> = spec.pivots.iter().map(|d| named(&d.column)).collect();
        // Along the time, the others are what a running total or a change
        // keeps apart; without it, the groups' own order runs instead.
        let (sequence, apart): (Vec<String>, Vec<String>) = if timed {
            (
                along[..1].to_vec(),
                along[1..].iter().chain(&pivoted).cloned().collect(),
            )
        } else {
            (along.clone(), pivoted.clone())
        };
        let partition = |by: &[String]| {
            if by.is_empty() {
                String::new()
            } else {
                format!("PARTITION BY {} ", by.join(", "))
            }
        };
        let outer = |base: &str, top: Option<&str>| -> String {
            let mut list: Vec<String> = along.iter().chain(&pivoted).cloned().collect();
            for (i, m) in metrics.iter().enumerate() {
                let v = format!("g.{}", quote_ident(&raw(i)));
                let earlier = |at: &str| format!("{at}.{}", quote_ident(&raw(i)));
                let running = |frame: &str, f: &str| {
                    if sequence.is_empty() {
                        v.clone()
                    } else {
                        format!(
                            "{f}({v}) OVER ({}ORDER BY {} ROWS BETWEEN {frame} AND CURRENT ROW)",
                            partition(&apart),
                            sequence.join(", ")
                        )
                    }
                };
                let value = match m.show {
                    None => v.clone(),
                    Some(MetricShow::ShareOfAll) => format!("{v} / NULLIF(SUM({v}) OVER (), 0)"),
                    Some(MetricShow::ShareOfRow) => {
                        format!(
                            "{v} / NULLIF(SUM({v}) OVER ({}), 0)",
                            partition(&along).trim_end()
                        )
                    }
                    Some(MetricShow::ShareOfColumn) => {
                        format!(
                            "{v} / NULLIF(SUM({v}) OVER ({}), 0)",
                            partition(&pivoted).trim_end()
                        )
                    }
                    Some(MetricShow::RunningTotal) => running("UNBOUNDED PRECEDING", "SUM"),
                    Some(MetricShow::MovingAverage) => running("2 PRECEDING", "AVG"),
                    Some(MetricShow::Rank) => format!(
                        "RANK() OVER ({}ORDER BY {v} DESC NULLS LAST)",
                        partition(&pivoted)
                    ),
                    Some(MetricShow::ChangeFromPrevious) => format!("{v} - {}", earlier("prev")),
                    Some(MetricShow::GrowthFromPrevious) => {
                        format!("({v} - {p}) / NULLIF(ABS({p}), 0)", p = earlier("prev"))
                    }
                    Some(MetricShow::ChangeFromYearBefore) => format!("{v} - {}", earlier("ago")),
                    Some(MetricShow::GrowthFromYearBefore) => {
                        format!("({v} - {p}) / NULLIF(ABS({p}), 0)", p = earlier("ago"))
                    }
                };
                list.push(format!(
                    "{value} AS {}",
                    quote_ident(&metric_name(m, grain))
                ));
            }
            // The period before, and the same one a year before: the same
            // group's, joined by when it starts.
            let mut joins = String::new();
            if let Some(time) = spec.time.as_ref().filter(|_| compared) {
                let (_, before, year) = steps_of(time.grain);
                for (at, back, used) in [("prev", before, false), ("ago", year, true)] {
                    if !metrics.iter().any(|m| over_time(m.show) == Some(used)) {
                        continue;
                    }
                    let same: Vec<String> = apart
                        .iter()
                        .map(|k| {
                            format!(
                                "{at}.{} IS NOT DISTINCT FROM {k}",
                                k.trim_start_matches("g.")
                            )
                        })
                        .collect();
                    joins.push_str(&format!(
                        "\nLEFT JOIN g AS {at} ON {at}.\"__start\" = g.\"__start\" - INTERVAL {back}{}",
                        same.iter().map(|c| format!(" AND {c}")).collect::<String>()
                    ));
                }
            }
            let mut steps = Vec::new();
            if let Some(top) = top {
                steps.push(format!("top AS (\n{}\n)", indent(top, "  ")));
            }
            steps.push(format!("g AS (\n{}\n)", indent(base, "  ")));
            format!(
                "WITH {}\nSELECT {}\nFROM g{joins}",
                steps.join(",\n"),
                list.join(",\n       ")
            )
        };
        // An order and a number of groups are for what is along the chart,
        // its time and dimensions, as BI tools sort an axis: by a metric over
        // each whole group; else the time's own order, then the values.
        let sort = if nk > 0 { spec.sort.as_ref() } else { None };
        if let Some(sort) = sort {
            if sort.metric as usize >= metrics.len() {
                return Err(ApiError::invalid(
                    "the chart is ordered by a metric it has not",
                ));
            }
        }
        let direction = |s: &crate::ChartSort| if s.descending { "DESC" } else { "ASC" };
        // How many groups: so many as chosen, all of them, or 1000.
        let limit = if nk > 0 { spec.limit } else { None };
        if limit == Some(ChartLimit::Count(0)) {
            return Err(ApiError::invalid("a chart shows at least one group"));
        }
        let chosen = match limit {
            Some(ChartLimit::Count(n)) => Some(n),
            _ => None,
        };
        let all = matches!(limit, Some(ChartLimit::All(_)));
        let group_by = places(0..nk + nc);
        // Pivots with an order or a number of groups: the groups along are
        // picked and ordered by their own totals first (an average or a
        // median over the whole group, not of a pivoted value within it).
        let ranked = nc > 0 && nk > 0 && (sort.is_some() || chosen.is_some());
        // One GROUP BY. Its order is the metric's, or the keys' own; or, when
        // a time has more periods than can be shown, the newest first, so
        // that the newest are the ones kept.
        let plain = |newest: bool| -> Result<(String, Option<usize>), ApiError> {
            let mut order = Vec::new();
            if let Some(sort) = sort {
                order.push(format!(
                    "{} {}",
                    quote_ident(&metric_name(&metrics[sort.metric as usize], grain)),
                    direction(sort)
                ));
                order.extend(group_by.clone());
            } else if newest {
                order.push("1 DESC".to_string());
                order.extend(places(1..nk + nc));
            } else {
                order.extend(group_by.clone());
            }
            let cap = match (chosen, all, nc) {
                (Some(n), _, _) => Some(n as usize),
                (None, true, _) => None,
                (None, false, 0) => Some(GROUPS as usize),
                (None, false, _) => Some(CELLS),
            };
            let mut sql = format!(
                "SELECT {}\nFROM {table_name}{}",
                keys("")
                    .into_iter()
                    .chain(totals("")?)
                    .chain(start(""))
                    .collect::<Vec<_>>()
                    .join(",\n       "),
                where_of(&kept)
            );
            if !group_by.is_empty() {
                sql.push_str(&format!("\nGROUP BY {}", group_by.join(", ")));
            }
            sql.push_str(&having_plain);
            if shown {
                sql = outer(&sql, None);
            }
            if !order.is_empty() {
                sql.push_str(&format!("\nORDER BY {}", order.join(", ")));
            }
            if let Some(cap) = cap {
                sql.push_str(&format!("\nLIMIT {cap}"));
            }
            Ok((sql, cap))
        };
        // The groups the first step picks: so many as chosen, all, or 1000.
        let picked_most = if all {
            None
        } else {
            Some(chosen.unwrap_or(GROUPS))
        };
        let (mut sql, mut cap) = if !ranked {
            plain(false)?
        } else {
            // What is along, as the first step picks it and as the rows are
            // joined back to it: the time at its grain, the dimensions.
            let along = |from: &str, level: &ChartLevel| match level.grain {
                Some(grain) => at_grain(&columns.value(from, &level.column), grain),
                None => columns.value(from, &level.column),
            };
            let picked_keys: Vec<String> = keyed
                .iter()
                .map(|level| match level.grain {
                    Some(_) => format!("{} AS {}", along("", level), quote_ident(&level.column)),
                    None => columns.key("", &level.column),
                })
                .collect();
            let cut = picked_most
                .map(|n| format!("\nLIMIT {n}"))
                .unwrap_or_default();
            let picked = match sort {
                Some(sort) => format!(
                    "SELECT {}, {} AS total\nFROM {table_name}{}\nGROUP BY {}\nORDER BY total {}{cut}",
                    picked_keys.join(", "),
                    metric(&columns, &metrics[sort.metric as usize], "")?,
                    where_of(&kept),
                    places(0..nk).join(", "),
                    direction(sort),
                ),
                None => format!(
                    "SELECT {}\nFROM {table_name}{}\nGROUP BY {}\nORDER BY {}{cut}",
                    picked_keys.join(", "),
                    where_of(&kept),
                    places(0..nk).join(", "),
                    places(0..nk).join(", "),
                ),
            };
            let joined: Vec<String> = keyed
                .iter()
                .map(|level| {
                    format!(
                        "{} IS NOT DISTINCT FROM {}",
                        along("t.", level),
                        col("top.", &level.column)
                    )
                })
                .collect();
            let mut by = group_by.clone();
            let mut order = Vec::new();
            // The groups' own totals, kept through the first step when shown.
            let total = if shown { "g.\"__total\"" } else { "top.total" };
            if let Some(sort) = sort {
                by.push("top.total".to_string());
                order.push(format!("{total} {}", direction(sort)));
            }
            order.extend(group_by.clone());
            let inner = conditions("t.", &mut bound)?;
            // Every cell of the groups picked: they are as many as were chosen.
            let selected: Vec<String> = keys("t.")
                .into_iter()
                .chain(totals("t.")?)
                .chain((shown && sort.is_some()).then(|| "top.total AS \"__total\"".to_string()))
                .chain(start("t."))
                .collect();
            let cells = format!(
                "SELECT {}\nFROM {table_name} AS t\nJOIN top ON {}{}\nGROUP BY {}{having_ranked}",
                selected.join(",\n       "),
                joined.join("\n  AND "),
                where_of(&inner),
                by.join(", "),
            );
            let sql = if shown {
                format!(
                    "{}\nORDER BY {}",
                    outer(&cells, Some(&picked)),
                    order.join(", ")
                )
            } else {
                format!(
                    "WITH top AS (\n{}\n)\n{cells}\nORDER BY {}",
                    indent(&picked, "  "),
                    order.join(", ")
                )
            };
            (sql, None)
        };

        // One row for each group and pivoted value: laid out as groups,
        // each with a total for every series.
        let mut cells = cells_of(session, &sql, &bound, nk, nc).await?;
        let mut truncated = match (cap, ranked.then_some(picked_most).flatten()) {
            (Some(cap), _) => cells.len() >= cap,
            (None, Some(most)) => {
                cells.iter().map(|c| &c.0).collect::<HashSet<_>>().len() >= most as usize
            }
            (None, None) => false,
        };
        // A time with more periods than are shown, in its own order: the
        // newest are shown (asked for newest first, then put back in order).
        if truncated && timed && sort.is_none() && limit.is_none() && !ranked {
            (sql, cap) = plain(true)?;
            cells = cells_of(session, &sql, &bound, nk, nc).await?;
            cells.sort_by(|a, b| (a.0[0].is_none(), &a.0[0]).cmp(&(b.0[0].is_none(), &b.0[0])));
            truncated = cap.is_some_and(|cap| cells.len() >= cap);
        }
        let query = query_of(&sql, &bound);
        let pivoted: Vec<Vec<Option<String>>> = cells
            .iter()
            .map(|(_, c, _)| c.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let nm = metrics.len();
        let series: Vec<ChartSeries> = pivoted
            .iter()
            .flat_map(|pivot| {
                (0..nm).map(|m| ChartSeries {
                    pivot: pivot.clone(),
                    metric: m as u32,
                })
            })
            .collect();
        let mut groups: Vec<ChartGroup> = Vec::new();
        let mut at: HashMap<Vec<Option<String>>, usize> = HashMap::new();
        for (keys, pivot, values) in cells {
            let i = *at.entry(keys.clone()).or_insert_with(|| {
                groups.push(ChartGroup {
                    keys,
                    values: vec![None; series.len()],
                });
                groups.len() - 1
            });
            let first = pivoted.iter().position(|c| *c == pivot).unwrap_or(0) * nm;
            for (m, value) in values.into_iter().enumerate() {
                groups[i].values[first + m] = value;
            }
        }
        // The time alone, in its own order and all of it: its periods with
        // no rows keep their place, within the period a filter keeps, or the
        // custom range, or from the first period found to the last (unless
        // that is more periods than can be shown).
        // (Not when groups are kept by a metric: an empty period is not one.)
        if let (Some(time), 0, false, None, None, true) = (
            &spec.time,
            nd,
            truncated,
            sort,
            chosen,
            spec.metric_filters.is_empty(),
        ) {
            let within = spec.filters.iter().find_map(|f| {
                f.period
                    .as_ref()
                    .filter(|_| f.column == time.column)
                    .and_then(|p| {
                        let start = period_start(p.grain, &p.key)?;
                        let end = period_after(p.grain, start)?
                            .checked_sub_signed(TimeDelta::try_seconds(1)?)?;
                        Some((start, end))
                    })
            });
            let found: Vec<&String> = groups.iter().filter_map(|g| g.keys[0].as_ref()).collect();
            let ranged = match ends {
                (Some(from), Some(to)) => from.and_hms_opt(0, 0, 0).zip(to.and_hms_opt(23, 59, 59)),
                _ => None,
            };
            let bounds = within.or(ranged).or_else(|| {
                found
                    .first()
                    .and_then(|k| period_start(time.grain, k))
                    .zip(found.last().and_then(|k| period_start(time.grain, k)))
            });
            let most = (GROUPS as usize).max(groups.len());
            if let Some(all) = bounds.and_then(|(from, to)| periods(time.grain, from, to, most)) {
                let mut by_key: HashMap<String, ChartGroup> = HashMap::new();
                let mut unreadable = Vec::new();
                for group in groups {
                    match group.keys[0].clone() {
                        Some(key) => {
                            by_key.insert(key, group);
                        }
                        None => unreadable.push(group),
                    }
                }
                groups = all
                    .into_iter()
                    .map(|key| {
                        by_key.remove(&key).unwrap_or(ChartGroup {
                            keys: vec![Some(key)],
                            values: vec![None; series.len()],
                        })
                    })
                    .collect();
                groups.extend(unreadable);
            }
        }
        Ok(ChartResult {
            query,
            columns: keyed,
            series,
            groups,
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
        let (schema, _) = self.analysis_columns(&req.space, &req.table).await?;
        let types = self.analysis_types(&req.space, &req.table).await?;
        let columns = Columns {
            schema: &schema,
            types: &types,
        };
        columns.has(&req.column)?;
        let column = columns.value("", &req.column);
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
            .flat_map(|batch| texts(batch.column(0)))
            .flatten()
            .collect())
    }
}
