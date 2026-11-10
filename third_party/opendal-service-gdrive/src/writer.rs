// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

// MODIFIED BY THE OIKOME AUTHORS (2026-11-09): a file larger than one
// chunk (8 MiB) is written through a resumable upload session, a chunk at a
// time, instead of being held whole and sent in one request
// (third_party/README.md, apache/opendal#8503). 2026-11-10: a new file's
// path is looked up once; the cache is refreshed only when the upload finds
// a folder gone.

use std::sync::Arc;

use bytes::Buf;
use http::StatusCode;

use super::core::GdriveFile;
use super::core::parse_error;
use super::core::{ErrorContext, GdriveCore};
use opendal_core::raw::*;
use opendal_core::*;

/// The bytes sent in each request of a resumable upload. Drive takes
/// chunks in multiples of 256 KiB; a file no larger than one chunk is sent
/// in a single request, as before.
pub const CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// `308 Resume Incomplete`: Drive has the chunk and wants the next one.
const RESUME_INCOMPLETE: u16 = 308;

/// Writes a file to Drive. Up to [`CHUNK_SIZE`] it is sent in one request
/// when the writer closes. A larger file goes through a resumable upload
/// session, a chunk at a time, so memory holds one chunk, not the file.
pub struct GdriveWriter {
    core: Arc<GdriveCore>,
    ctx: OperationContext,

    path: String,

    file_id: Option<String>,

    /// Bytes not sent yet.
    buf: oio::QueueBuf,
    /// The resumable session, once the file outgrew one chunk, and whether
    /// it makes a new file (then its id is learned at the end).
    session: Option<(String, bool)>,
    /// Bytes the session has.
    sent: u64,
}

impl GdriveWriter {
    pub fn new(
        core: Arc<GdriveCore>,
        ctx: OperationContext,
        path: String,
        file_id: Option<String>,
    ) -> Self {
        GdriveWriter {
            core,
            ctx,
            path,

            file_id,

            buf: oio::QueueBuf::new(),
            session: None,
            sent: 0,
        }
    }

    /// The id of the file at the writer's path, if there is one. A path
    /// not found was just asked of the drive: the cache is refreshed only
    /// if the upload then finds a folder or the file gone, not before every
    /// new file (that would look every folder of the path up again).
    async fn current_file_id(&self) -> Result<Option<String>> {
        Ok(match &self.file_id {
            Some(file_id) => Some(file_id.clone()),
            None => self.core.resolve_path(&self.ctx, &self.path).await?,
        })
    }

    /// Start a resumable session for the writer's path.
    async fn start_session(&self) -> Result<(String, bool)> {
        let mut current_file_id = self.current_file_id().await?;
        let mut retried = false;
        loop {
            let resp = self
                .core
                .gdrive_upload_resumable_start(&self.ctx, &self.path, current_file_id.as_deref())
                .await?;
            match resp.status() {
                StatusCode::OK | StatusCode::CREATED => {
                    let location = resp
                        .headers()
                        .get(http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .ok_or_else(|| {
                            Error::new(
                                ErrorKind::Unexpected,
                                "the resumable upload session has no location",
                            )
                        })?;
                    return Ok((location.to_string(), current_file_id.is_none()));
                }
                // An id cached for the path or its folder is gone: look the
                // path up again.
                StatusCode::NOT_FOUND if !retried => {
                    retried = true;
                    self.core.refresh_path(&self.path).await;
                    current_file_id = self.core.resolve_path(&self.ctx, &self.path).await?;
                }
                _ => {
                    return Err(parse_error(
                        ErrorContext::new(ServiceOperation("UploadFile")),
                        resp,
                    ));
                }
            }
        }
    }

    /// Record the file a finished upload made or replaced.
    async fn finished(&self, size: u64, made: Option<GdriveFile>) {
        let mut metadata = MetadataBuilder::file(size);
        if let Some(file) = made {
            metadata.content_type(file.mime_type);
            self.core.cache_file_id(&self.path, &file.id).await;
        }
        self.core
            .record_recent_upsert(&self.path, metadata.build())
            .await;
    }

    /// The whole file in one request: a file no larger than one chunk.
    async fn write_once(&self, bs: Buffer) -> Result<Metadata> {
        let size = bs.len();

        let mut current_file_id = self.current_file_id().await?;
        let mut retried = false;

        loop {
            let resp = if let Some(file_id) = &current_file_id {
                self.core
                    .gdrive_upload_overwrite_simple_request(
                        &self.ctx,
                        file_id,
                        size as u64,
                        bs.clone(),
                    )
                    .await
            } else {
                self.core
                    .gdrive_upload_simple_request(&self.ctx, &self.path, size as u64, bs.clone())
                    .await
            }?;

            match resp.status() {
                StatusCode::OK | StatusCode::CREATED => {
                    let made = if current_file_id.is_none() {
                        let bs = resp.into_body();
                        Some(
                            serde_json::from_reader(bs.reader())
                                .map_err(new_json_deserialize_error)?,
                        )
                    } else {
                        None
                    };
                    self.finished(size as u64, made).await;
                    return Ok(MetadataBuilder::unknown().build());
                }
                StatusCode::NOT_FOUND if !retried => {
                    retried = true;
                    self.core.refresh_path(&self.path).await;
                    current_file_id = self.core.resolve_path(&self.ctx, &self.path).await?;
                    continue;
                }
                _ => {
                    return Err(parse_error(
                        ErrorContext::new(ServiceOperation("UploadFile")),
                        resp,
                    ));
                }
            }
        }
    }

    /// Send one chunk before the last; Drive must have all of it.
    async fn send_chunk(&mut self, session: &str, chunk: Buffer) -> Result<()> {
        let size = chunk.len() as u64;
        let resp = self
            .core
            .gdrive_upload_resumable_chunk(&self.ctx, session, self.sent, chunk, None)
            .await?;
        if resp.status().as_u16() != RESUME_INCOMPLETE {
            return Err(parse_error(
                ErrorContext::new(ServiceOperation("UploadFile")),
                resp,
            ));
        }
        // `Range: bytes=0-<last>` is what Drive has: every byte sent.
        let has = resp
            .headers()
            .get(http::header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('-').next())
            .and_then(|last| last.parse::<u64>().ok())
            .map(|last| last + 1);
        if has != Some(self.sent + size) {
            return Err(Error::new(
                ErrorKind::Unexpected,
                "Drive did not keep the whole chunk of a resumable upload",
            )
            .set_temporary());
        }
        self.sent += size;
        Ok(())
    }
}

impl oio::Write for GdriveWriter {
    async fn write(&mut self, bs: Buffer) -> Result<()> {
        self.buf.push(bs);
        // Keep what is past a whole chunk: the last request needs bytes.
        while self.buf.len() > CHUNK_SIZE {
            let all = self.buf.take().collect();
            let (chunk, rest) = (all.slice(0..CHUNK_SIZE), all.slice(CHUNK_SIZE..));
            let session = match &self.session {
                Some((session, _)) => session.clone(),
                None => {
                    let started = self.start_session().await?;
                    let session = started.0.clone();
                    self.session = Some(started);
                    session
                }
            };
            self.buf.push(rest);
            self.send_chunk(&session, chunk).await?;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<Metadata> {
        let rest = self.buf.take().collect();
        let Some((session, makes_file)) = self.session.clone() else {
            return self.write_once(rest).await;
        };
        let total = self.sent + rest.len() as u64;
        let resp = self
            .core
            .gdrive_upload_resumable_chunk(&self.ctx, &session, self.sent, rest, Some(total))
            .await?;
        match resp.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let made = if makes_file {
                    let bs = resp.into_body();
                    Some(serde_json::from_reader(bs.reader()).map_err(new_json_deserialize_error)?)
                } else {
                    None
                };
                self.session = None;
                self.finished(total, made).await;
                Ok(MetadataBuilder::unknown().build())
            }
            _ => Err(parse_error(
                ErrorContext::new(ServiceOperation("UploadFile")),
                resp,
            )),
        }
    }

    async fn abort(&mut self) -> Result<()> {
        self.buf.clear();
        if let Some((session, _)) = self.session.take() {
            // Drive answers a cancelled session with 499.
            let resp = self
                .core
                .gdrive_upload_resumable_cancel(&self.ctx, &session)
                .await?;
            let status = resp.status().as_u16();
            if !(status == 499 || resp.status().is_success()) {
                return Err(parse_error(
                    ErrorContext::new(ServiceOperation("UploadFile")),
                    resp,
                ));
            }
        }
        Ok(())
    }
}
