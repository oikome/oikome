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

//! A DuckDB of its own for one compute task (architecture v3 §12): a peer's
//! query over the input the peer sent, and nothing else.
//!
//! Each task opens a new in-memory database with no access to files or the
//! network, no extensions loaded or installed, and its configuration locked
//! once the input table `input` is in place. The task runs within a wall
//! time (it is interrupted when that passes), a memory limit and a cap on
//! the bytes it returns.

use std::time::Duration;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use duckdb::{Config, Connection};
use oikome_engine::EngineError;

use crate::types::arrow_to_duckdb_type;

/// The table the input is in.
pub const INPUT_TABLE: &str = "input";

/// Limits on one isolated run.
#[derive(Debug, Clone, Copy)]
pub struct IsolatedLimits {
    pub time: Duration,
    pub memory_bytes: u64,
    pub output_bytes: u64,
    pub threads: usize,
}

fn sql_err(e: duckdb::Error) -> EngineError {
    EngineError::Sql(e.to_string())
}

/// Open the isolated database and load the input.
fn open(
    schema: &SchemaRef,
    batches: Vec<RecordBatch>,
    limits: &IsolatedLimits,
) -> Result<Connection, EngineError> {
    let config = Config::default()
        .enable_external_access(false)
        .and_then(|c| c.enable_autoload_extension(false))
        .and_then(|c| c.with("autoinstall_known_extensions", "false"))
        .and_then(|c| c.max_memory(&format!("{}B", limits.memory_bytes.max(1 << 20))))
        .and_then(|c| c.threads(limits.threads.max(1) as i64))
        .map_err(sql_err)?;
    let conn = Connection::open_in_memory_with_flags(config).map_err(sql_err)?;
    let columns = schema
        .fields()
        .iter()
        .map(|f| {
            Ok(format!(
                "\"{}\" {}",
                f.name().replace('"', "\"\""),
                arrow_to_duckdb_type(f.data_type())?
            ))
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    conn.execute_batch(&format!(
        "CREATE TABLE {INPUT_TABLE} ({})",
        columns.join(", ")
    ))
    .map_err(sql_err)?;
    {
        let mut appender = conn.appender(INPUT_TABLE).map_err(sql_err)?;
        for batch in batches {
            appender.append_record_batch(batch).map_err(sql_err)?;
        }
        appender.flush().map_err(sql_err)?;
    }
    // From here on the task cannot change any setting.
    conn.execute_batch("SET lock_configuration = true")
        .map_err(sql_err)?;
    Ok(conn)
}

/// Run `sql` (one query) over `batches`, the table `input` of `schema`,
/// within `limits`.
pub async fn run_isolated(
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    sql: String,
    limits: IsolatedLimits,
) -> Result<(SchemaRef, Vec<RecordBatch>), EngineError> {
    let conn = {
        let schema = schema.clone();
        tokio::task::spawn_blocking(move || open(&schema, batches, &limits))
            .await
            .map_err(|e| EngineError::Sql(e.to_string()))??
    };
    let interrupt = conn.interrupt_handle();
    let run = tokio::task::spawn_blocking(move || {
        let mut stmt = conn.prepare(&sql).map_err(sql_err)?;
        let arrow = stmt.query_arrow([]).map_err(sql_err)?;
        let schema = arrow.get_schema();
        let mut out = Vec::new();
        let mut bytes = 0u64;
        for batch in arrow {
            bytes += batch.get_array_memory_size() as u64;
            if bytes > limits.output_bytes {
                return Err(EngineError::LimitExceeded(format!(
                    "the result is over the {} bytes allowed",
                    limits.output_bytes
                )));
            }
            out.push(batch);
        }
        Ok((schema, out))
    });
    match tokio::time::timeout(limits.time, run).await {
        Ok(joined) => joined.map_err(|e| EngineError::Sql(e.to_string()))?,
        Err(_) => {
            interrupt.interrupt();
            Err(EngineError::LimitExceeded(format!(
                "the task ran longer than the {} ms allowed",
                limits.time.as_millis()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::Int64Array;
    use arrow_schema::{DataType, Field, Schema};

    use super::*;

    fn input() -> (SchemaRef, Vec<RecordBatch>) {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3, 4]))],
        )
        .unwrap();
        (schema, vec![batch])
    }

    fn limits() -> IsolatedLimits {
        IsolatedLimits {
            time: Duration::from_secs(10),
            memory_bytes: 256 << 20,
            output_bytes: 1 << 20,
            threads: 1,
        }
    }

    async fn run(sql: &str, limits: IsolatedLimits) -> Result<Vec<RecordBatch>, EngineError> {
        let (schema, batches) = input();
        run_isolated(schema, batches, sql.to_string(), limits)
            .await
            .map(|(_, b)| b)
    }

    #[tokio::test]
    async fn a_task_reads_its_input_and_nothing_else() {
        let out = run(
            "SELECT CAST(sum(n) AS BIGINT) AS total FROM input",
            limits(),
        )
        .await
        .unwrap();
        let total = out[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|a| a.value(0));
        assert_eq!(total, Some(10));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secret.csv");
        std::fs::write(&file, "a\n1\n").unwrap();
        for sql in [
            format!("SELECT * FROM read_csv('{}')", file.display()),
            format!("COPY input TO '{}'", dir.path().join("out.csv").display()),
            format!("ATTACH '{}' AS other", dir.path().join("x.db").display()),
            "INSTALL httpfs".to_string(),
            "SET memory_limit = '64GB'".to_string(),
            "SET enable_external_access = true".to_string(),
        ] {
            assert!(run(&sql, limits()).await.is_err(), "{sql}");
        }
        assert!(!dir.path().join("out.csv").exists());
    }

    #[tokio::test]
    async fn time_and_output_limits_stop_a_task() {
        let slow = IsolatedLimits {
            time: Duration::from_millis(200),
            ..limits()
        };
        let started = std::time::Instant::now();
        let e = run("SELECT count(*) FROM range(100000000000) a", slow)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("longer than"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let small = IsolatedLimits {
            output_bytes: 64,
            ..limits()
        };
        let e = run("SELECT * FROM range(100000)", small).await.unwrap_err();
        assert!(e.to_string().contains("bytes allowed"), "{e}");
    }
}
