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

//! Peers' compute tasks over Arrow Flight (architecture v3 §12).
//!
//! `DoExchange` carries one task: its first message's descriptor holds the
//! signed request (method and task), the input batches follow, and the
//! result streams back. `oikome.compute_methods` lists what the caller may
//! run here. Both need a `compute/<method>` grant for a method this node
//! offers; a data grant is not enough, and a compute grant reads no Space.

use std::sync::atomic::{AtomicU64, Ordering};

use arrow_flight::decode::FlightRecordBatchStream;
use oikome_identity::ucan::Limits;
use oikome_network::protocol::{
    ComputeMethod, ComputeRequest, METHOD_COMPUTE, METHOD_COMPUTE_METHODS,
};
use oikome_node::compute::{effective_limits, quota_for, ComputeTask};
use tonic::Streaming;

use super::*;

/// How a failed task looks to the caller.
fn compute_status(e: SessionError) -> Status {
    match e {
        SessionError::PermissionDenied(_) => Status::permission_denied(DENIED),
        SessionError::Engine(EngineError::LimitExceeded(m)) => Status::resource_exhausted(m),
        SessionError::Engine(EngineError::Sql(m)) | SessionError::Unsupported(m) => {
            Status::invalid_argument(m)
        }
        e => {
            tracing::warn!(error = %e, "a compute task failed");
            Status::internal("the task failed on the remote node")
        }
    }
}

impl OikomeFlightService {
    /// Whether `ucan`, presented by `caller`, lets it run `method` here, a
    /// method this node offers. Returns the principal and the grant's
    /// limits.
    fn verify_compute(
        &self,
        caller: &Did,
        ucan: &str,
        method: &str,
    ) -> Result<(String, String, Limits), String> {
        if !self.catalog.offered_compute().iter().any(|m| m == method) {
            return Err(format!("'{method}' is not offered"));
        }
        let revoked = self.catalog.revoked_ucans();
        let verified = ucan::verify(
            ucan,
            &VerifyOptions {
                root: self.identity.did(),
                audience: caller,
                required: &Capability::compute(self.identity.did(), method, None),
                now: unix_now(),
                revoked: &revoked,
            },
        )
        .map_err(|e| e.to_string())?;
        Ok((verified.principal.to_string(), verified.id, verified.limits))
    }

    fn refresh_registry(&self) -> Result<(), Status> {
        self.catalog.refresh().map_err(|e| {
            tracing::warn!(error = %e, "could not reload the registry");
            Status::internal("registry unavailable")
        })?;
        Ok(())
    }

    /// `DoExchange`: run one compute task for a peer.
    pub(super) async fn compute(
        &self,
        entry: &mut AccessEntry,
        request: Request<Streaming<FlightData>>,
    ) -> Result<FlightStream<FlightData>, Status> {
        let auth = Self::header(&request, HEADER_AUTH).map(str::to_string);
        let ucan = Self::header(&request, HEADER_UCAN).map(str::to_string);
        let mut input = request.into_inner();
        let first = input
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("an empty task"))?;
        let cmd = first
            .flight_descriptor
            .as_ref()
            .map(|d| d.cmd.clone())
            .ok_or_else(|| Status::invalid_argument("a task without its request"))?;
        if cmd.len() > MAX_SQL_BYTES + 256 {
            return Err(Status::invalid_argument("task too large"));
        }
        let caller = self.authenticate_parts(
            entry,
            auth.as_deref(),
            ucan.as_deref(),
            METHOD_COMPUTE,
            &cmd,
        )?;
        let request: ComputeRequest = serde_json::from_slice(&cmd)
            .map_err(|e| Status::invalid_argument(format!("compute request: {e}")))?;
        self.refresh_registry()?;
        let (principal, token_id, limits) = self
            .verify_compute(&caller.did, &caller.ucan, &request.method)
            .map_err(|reason| {
                tracing::info!(caller = %caller.did, %reason, "compute request denied");
                entry.reason = Some(reason);
                Status::permission_denied(DENIED)
            })?;
        entry.caller = Some(principal.clone());
        entry.token_id = Some(token_id);
        let limits = effective_limits(limits);

        // The input, within the grant's input limit.
        let max_input = limits.input_bytes.unwrap_or(u64::MAX);
        let received = Arc::new(AtomicU64::new(0));
        let counted = received.clone();
        let messages = stream::once(async move { Ok(first) })
            .chain(input)
            .map(move |message| {
                let message = message.map_err(FlightError::from)?;
                let total = counted.fetch_add(
                    (message.data_header.len() + message.data_body.len()) as u64,
                    Ordering::Relaxed,
                ) + (message.data_header.len() + message.data_body.len()) as u64;
                if total > max_input {
                    return Err(FlightError::ProtocolError(format!(
                        "the input is over the {max_input} bytes allowed"
                    )));
                }
                Ok(message)
            });
        let mut decoder = FlightRecordBatchStream::new_from_flight_data(messages);
        let mut batches = Vec::new();
        while let Some(batch) = decoder.try_next().await.map_err(|e| match e {
            FlightError::ProtocolError(m) if m.contains("bytes allowed") => {
                Status::resource_exhausted(m)
            }
            e => Status::invalid_argument(format!("task input: {e}")),
        })? {
            batches.push(batch);
        }
        let schema = decoder
            .schema()
            .cloned()
            .ok_or_else(|| Status::invalid_argument("a task without an input schema"))?;

        let quota = quota_for(&self.catalog, &principal);
        let method = request.method.clone();
        let task = ComputeTask {
            peer: principal,
            request,
            schema,
            batches,
            limits,
        };
        // Checked again when the task's turn comes: revoking the grant or
        // withdrawing the method drops a waiting task.
        let still_granted = || {
            self.catalog.refresh().map_err(|e| e.to_string())?;
            self.verify_compute(&caller.did, &caller.ucan, &method)
                .map(|_| ())
        };
        let (schema, batches) = self
            .engines
            .compute()
            .run(task, &quota, still_granted)
            .await
            .map_err(compute_status)?;
        let flight = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(stream::iter(batches.into_iter().map(Ok)))
            .map_err(Status::from);
        Ok(Box::pin(flight))
    }

    /// `oikome.compute_methods`: the methods this node offers that the
    /// caller's grant covers, with its limits for each.
    pub(super) async fn compute_methods(
        &self,
        entry: &mut AccessEntry,
        request: Request<Action>,
    ) -> Result<Vec<u8>, Status> {
        let body = request.get_ref().body.clone();
        if body.len() > MAX_ACTION_BODY_BYTES {
            return Err(Status::invalid_argument("request too large"));
        }
        let caller = self.authenticate(entry, &request, METHOD_COMPUTE_METHODS, &body)?;
        self.refresh_registry()?;
        let mut methods = Vec::new();
        for method in self.catalog.offered_compute() {
            if let Ok((principal, _, limits)) =
                self.verify_compute(&caller.did, &caller.ucan, &method)
            {
                entry.caller = Some(principal);
                let limits = effective_limits(limits);
                methods.push(ComputeMethod {
                    method,
                    time_ms: limits.time_ms.unwrap_or_default(),
                    input_bytes: limits.input_bytes.unwrap_or_default(),
                    output_bytes: limits.output_bytes.unwrap_or_default(),
                    memory_bytes: limits.memory_bytes.unwrap_or_default(),
                });
            }
        }
        if methods.is_empty() {
            // Like any other refusal: a caller learns nothing it may not use.
            entry.reason = Some("no compute grant for an offered method".to_string());
            return Err(Status::permission_denied(DENIED));
        }
        serde_json::to_vec(&methods).map_err(|e| Status::internal(e.to_string()))
    }
}
