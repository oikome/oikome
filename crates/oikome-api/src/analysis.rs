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

//! Explicit derived analysis and its versioned field profile. Source tables
//! stay unchanged; a derived Space records its query and source fingerprint.

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use oikome_core::validate_space_name;
use oikome_node::QueryResult;
use oikome_semantic::rewrite::quote_ident;

use crate::{
    AnalysisProfile, AnalyzeDataset, Api, ApiError, Caller, DatasetAnalysis, DatasetProfile,
    ErrorCode, FieldProfile, FieldRole, NewSpace, SpaceKind, SpaceRef,
};

fn fingerprint(schema: &SchemaRef) -> String {
    let fields = schema
        .fields()
        .iter()
        .map(|field| format!("{}:{}", field.name(), field.data_type()))
        .collect::<Vec<_>>()
        .join(",");
    blake3::hash(fields.as_bytes()).to_string()
}

fn role(field: &Field) -> FieldRole {
    let name = field.name().to_ascii_lowercase();
    let time_name = matches!(
        name.as_str(),
        "day"
            | "date"
            | "time"
            | "timestamp"
            | "taken"
            | "written"
            | "created"
            | "created_at"
            | "updated_at"
    );
    if time_name
        || matches!(
            field.data_type(),
            DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
        )
    {
        return FieldRole::Time;
    }
    let numeric = matches!(
        field.data_type(),
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
    );
    if numeric
        && [
            "amount", "total", "value", "price", "cost", "eur", "usd", "number",
        ]
        .iter()
        .any(|word| name.contains(word))
    {
        return FieldRole::Amount;
    }
    if matches!(
        name.as_str(),
        "category" | "kind" | "type" | "group" | "status" | "place" | "room" | "vendor"
    ) {
        return FieldRole::Category;
    }
    if matches!(
        name.as_str(),
        "title" | "name" | "caption" | "memo" | "description" | "text"
    ) {
        return FieldRole::Title;
    }
    if name == "id" || name.ends_with("_id") || matches!(name.as_str(), "uuid" | "key" | "url") {
        return FieldRole::Key;
    }
    FieldRole::Other
}

fn schema_profile(space: &str, table: &str, schema: &SchemaRef) -> DatasetProfile {
    DatasetProfile {
        space: space.into(),
        table: table.into(),
        fields: schema
            .fields()
            .iter()
            .map(|field| FieldProfile {
                name: field.name().clone(),
                data_type: field.data_type().to_string(),
                role: role(field),
                inferred: true,
            })
            .collect(),
        fingerprint: fingerprint(schema),
    }
}

fn string_column<'a>(batches: &'a [RecordBatch], index: usize) -> Option<&'a StringArray> {
    batches
        .first()?
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
}

fn analysis_query(req: &AnalyzeDataset) -> String {
    format!(
        "SELECT CAST({time} AS DATE) AS analysis_day, {category} AS analysis_category, \
         SUM({amount}) AS total FROM {space}.{table} \
         GROUP BY analysis_day, analysis_category ORDER BY analysis_day, analysis_category",
        time = quote_ident(&req.time_field),
        category = quote_ident(&req.category_field),
        amount = quote_ident(&req.amount_field),
        space = quote_ident(&req.space),
        table = quote_ident(&req.table),
    )
}

impl Api {
    /// Infer field roles without reading or changing the source's rows.
    pub async fn dataset_profile(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<DatasetProfile, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let (space, table) = req.space.split_once('.').ok_or_else(|| {
            ApiError::invalid("name a source as collection.table, such as spending.entries")
        })?;
        validate_space_name(space)?;
        let session = self.session(None);
        if !session
            .readable_relations(space)
            .await?
            .contains(&table.to_string())
        {
            return Err(ApiError::not_found(format!(
                "Table not found: {space}.{table}"
            )));
        }
        let result = session
            .sql(&format!(
                "SELECT * FROM {}.{} LIMIT 0",
                quote_ident(space),
                quote_ident(table)
            ))
            .await?;
        let QueryResult::Batches { schema, .. } = result else {
            return Err(ApiError::internal("a dataset profile returned no schema"));
        };
        Ok(schema_profile(space, table, &schema))
    }

    /// Create a derived analysis explicitly; the source is never rewritten.
    pub async fn analyze_dataset(
        &self,
        caller: &Caller,
        req: AnalyzeDataset,
    ) -> Result<DatasetAnalysis, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        validate_space_name(&req.name)?;
        if self.catalog.get_space(&req.name).is_ok() || self.catalog.get_location(&req.name).is_ok()
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!("{} is taken; choose another analysis name", req.name),
            ));
        }
        let source = self
            .dataset_profile(
                caller,
                SpaceRef {
                    space: format!("{}.{}", req.space, req.table),
                },
            )
            .await?;
        for field in [&req.time_field, &req.amount_field, &req.category_field] {
            if !source
                .fields
                .iter()
                .any(|candidate| &candidate.name == field)
            {
                return Err(ApiError::invalid(format!(
                    "{}.{} has no field {field}",
                    req.space, req.table
                )));
            }
        }
        self.create_space(
            caller,
            NewSpace {
                name: req.name.clone(),
                kind: Some(SpaceKind::Analytical),
                storage: None,
            },
        )
        .await?;
        let query = analysis_query(&req);
        let result = self.session(None).sql(&query).await?;
        let QueryResult::Batches { schema, batches } = result else {
            return Err(ApiError::invalid("the analysis did not return rows"));
        };
        self.write_saved_table(&req.name, "_analysis_daily", schema, batches)
            .await?;
        let created_at_ms = crate::now_ms();
        let metadata = SchemaRef::new(Schema::new(vec![
            Field::new("source_space", DataType::Utf8, false),
            Field::new("source_table", DataType::Utf8, false),
            Field::new("time_field", DataType::Utf8, false),
            Field::new("amount_field", DataType::Utf8, false),
            Field::new("category_field", DataType::Utf8, false),
            Field::new("query", DataType::Utf8, false),
            Field::new("source_fingerprint", DataType::Utf8, false),
            Field::new("created_at_ms", DataType::Int64, false),
        ]));
        self.write_saved_table(
            &req.name,
            "_oikome_analysis",
            metadata.clone(),
            vec![RecordBatch::try_new(
                metadata,
                vec![
                    std::sync::Arc::new(StringArray::from(vec![req.space.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![req.table.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![req.time_field.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![req.amount_field.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![req.category_field.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![query.clone()])),
                    std::sync::Arc::new(StringArray::from(vec![source.fingerprint.clone()])),
                    std::sync::Arc::new(Int64Array::from(vec![created_at_ms as i64])),
                ],
            )
            .map_err(ApiError::internal)?],
        )
        .await?;
        Ok(DatasetAnalysis {
            space: self.summary_of(caller, &req.name).await?,
            profile: AnalysisProfile {
                name: req.name,
                source_space: req.space,
                source_table: req.table,
                time_field: req.time_field,
                amount_field: req.amount_field,
                category_field: req.category_field,
                query,
                source_fingerprint: source.fingerprint,
                source_changed: false,
                created_at_ms,
            },
        })
    }

    /// Read a derived analysis and compare its source's current shape.
    pub async fn analysis_profile(
        &self,
        caller: &Caller,
        req: SpaceRef,
    ) -> Result<AnalysisProfile, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let result = self
            .session(None)
            .sql(&format!(
                "SELECT * FROM {}._oikome_analysis LIMIT 1",
                quote_ident(&req.space)
            ))
            .await?;
        let QueryResult::Batches { batches, .. } = result else {
            return Err(ApiError::not_found("this is not a derived analysis"));
        };
        let values = |index| {
            string_column(&batches, index)
                .filter(|column| column.len() == 1)
                .map(|column| column.value(0).to_string())
                .ok_or_else(|| ApiError::not_found("this is not a derived analysis"))
        };
        let source_space = values(0)?;
        let source_table = values(1)?;
        let current = self
            .dataset_profile(
                caller,
                SpaceRef {
                    space: format!("{source_space}.{source_table}"),
                },
            )
            .await?;
        Ok(AnalysisProfile {
            name: req.space,
            source_space,
            source_table,
            time_field: values(2)?,
            amount_field: values(3)?,
            category_field: values(4)?,
            query: values(5)?,
            source_changed: values(6)? != current.fingerprint,
            source_fingerprint: current.fingerprint,
            created_at_ms: batches
                .first()
                .and_then(|batch| batch.column(7).as_any().downcast_ref::<Int64Array>())
                .map(|column| column.value(0) as u64)
                .unwrap_or(0),
        })
    }
}
