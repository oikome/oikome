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

//! Downloading a file the owner asked for (a model, an extension), saying
//! how far it is as it goes, and resuming the part an earlier try left.

use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::SessionError;

/// A downloaded file: its size and SHA-256.
pub(crate) struct Fetched {
    pub size: u64,
    pub sha256: String,
}

fn io(path: &Path, e: std::io::Error) -> SessionError {
    SessionError::Unsupported(format!("{}: {e}", path.display()))
}

/// A client for downloads the owner asked for.
pub(crate) fn client() -> Result<reqwest::Client, SessionError> {
    reqwest::Client::builder()
        .user_agent(concat!("oikome/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| SessionError::Unsupported(format!("download: {e}")))
}

/// Download `url` into `part`, continuing what is there, calling `progress`
/// with the bytes there so far and the whole size when known. Stops at
/// `expected` bytes when given. The caller checks the result and moves it.
pub(crate) async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    expected: Option<u64>,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Fetched, SessionError> {
    let failed = |e: reqwest::Error| SessionError::Unsupported(format!("download: {e}"));
    let mut hasher = Sha256::new();
    let mut have = match tokio::fs::metadata(part).await {
        Ok(m) if expected.is_none_or(|size| m.len() <= size) => {
            hash_into(part, &mut hasher).await?;
            m.len()
        }
        _ => 0,
    };
    progress(have, expected);
    if expected != Some(have) {
        let mut request = client.get(url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let response = request.send().await.map_err(failed)?;
        let status = response.status();
        // What is there already is the whole file.
        let complete = status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && have > 0;
        if !complete {
            if !status.is_success() {
                let host = reqwest::Url::parse(url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                return Err(SessionError::Unsupported(format!(
                    "download: {host} answered {status}"
                )));
            }
            let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
            if !resumed {
                // The whole file came: start the part again.
                hasher = Sha256::new();
                have = 0;
            }
            let total = expected.or_else(|| response.content_length().map(|n| n + have));
            progress(have, total);
            let mut writer = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resumed)
                .truncate(!resumed)
                .open(part)
                .await
                .map_err(|e| io(part, e))?;
            let mut body = response.bytes_stream();
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(failed)?;
                have += chunk.len() as u64;
                if expected.is_some_and(|size| have > size) {
                    break;
                }
                hasher.update(&chunk);
                writer.write_all(&chunk).await.map_err(|e| io(part, e))?;
                progress(have, total);
            }
            writer.flush().await.map_err(|e| io(part, e))?;
        }
    }
    Ok(Fetched {
        size: have,
        sha256: hex(&hasher.finalize()),
    })
}

pub(crate) async fn hash_into(path: &Path, hasher: &mut Sha256) -> Result<(), SessionError> {
    let mut f = tokio::fs::File::open(path).await.map_err(|e| io(path, e))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).await.map_err(|e| io(path, e))?;
        if n == 0 {
            return Ok(());
        }
        hasher.update(&buf[..n]);
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
