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

//! The folder of one Space on a storage (architecture v3 §7.5): a NAS
//! reached over WebDAV, a folder such as a NAS share mounted over SMB or
//! NFS, a bucket on RustFS, or Google Drive.
//!
//! Paths are relative to the Space's folder and made of plain segments.
//! Files are written under a temporary name and renamed into place, so a
//! reader sees a file whole or not at all.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use futures::stream::{self, BoxStream};
use futures::{StreamExt, TryStreamExt};
use opendal::{EntryMode, ErrorKind, Operator};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

/// Bytes read from a local file per upload chunk.
const UPLOAD_CHUNK_BYTES: usize = 1 << 20;
/// A part of a file written to object storage: S3's multipart uploads take
/// parts of 5 MiB and up, and memory holds one part, not the file.
const UPLOAD_PART_BYTES: usize = 8 << 20;
/// Bytes requested per read from the storage.
const READ_CHUNK_BYTES: usize = 1 << 20;

/// A failed storage operation. The message never contains credentials.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The file or folder does not exist.
    #[error("{0} is not on the storage")]
    NotFound(String),
    /// The storage refused the credentials (HTTP 401 or 403).
    #[error("the storage refused the credentials")]
    Denied,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Other(String),
}

impl StoreError {
    fn from_dal(path: &str, e: opendal::Error) -> Self {
        if e.kind() == ErrorKind::NotFound {
            return StoreError::NotFound(path.to_string());
        }
        let status = http_status(&e.to_string());
        if e.kind() == ErrorKind::PermissionDenied || matches!(status, Some(401 | 403)) {
            return StoreError::Denied;
        }
        // Only the status: the rest of the message holds response headers.
        match status {
            Some(status) => StoreError::Other(format!("{path}: {} (HTTP {status})", e.kind())),
            None => StoreError::Other(format!("{path}: {}", e.kind())),
        }
    }
}

/// The size of the local file `local` and its bytes as a stream.
pub async fn file_chunks(
    local: &Path,
) -> Result<(u64, BoxStream<'static, std::io::Result<Bytes>>), StoreError> {
    let shown = local.display().to_string();
    let size = tokio::fs::metadata(local)
        .await
        .map_err(|e| StoreError::Other(format!("{shown}: {e}")))?
        .len();
    let file = tokio::fs::File::open(local)
        .await
        .map_err(|e| StoreError::Other(format!("{shown}: {e}")))?;
    let chunks = stream::try_unfold(file, |mut file| async move {
        let mut buf = vec![0u8; UPLOAD_CHUNK_BYTES];
        let n = file.read(&mut buf).await?;
        if n == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        buf.truncate(n);
        Ok(Some((Bytes::from(buf), file)))
    });
    Ok((size, chunks.boxed()))
}

/// The HTTP status opendal puts in the message of a WebDAV error.
fn http_status(message: &str) -> Option<u16> {
    let rest = &message[message.find("status: ")? + "status: ".len()..];
    rest.get(..3)?.parse().ok()
}

/// How to sign in to a WebDAV server. The password and token are kept out
/// of `Debug`.
#[derive(Clone, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub token: Option<Zeroizing<String>>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A file or folder in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreEntry {
    /// Name within the listed folder, without a trailing `/`.
    pub name: String,
    pub is_dir: bool,
}

/// Uploads to WebDAV: opendal sends a WebDAV file in a single request body,
/// so large files are streamed with a plain PUT instead.
#[derive(Clone)]
struct WebdavPut {
    client: reqwest::Client,
    /// The Space's folder, ending in `/`.
    base: url::Url,
    credentials: Credentials,
}

/// The folder of one Space on a storage.
#[derive(Clone)]
pub struct RemoteStore {
    op: Operator,
    put: Option<WebdavPut>,
    /// The bucket, on object storage, for what a person is told.
    bucket: Option<String>,
}

/// A bucket on RustFS, reached over the S3 protocol, path-style.
#[derive(Debug, Clone, Copy)]
pub struct Bucket<'a> {
    /// `http(s)://host[:port]`.
    pub endpoint: &'a str,
    pub name: &'a str,
}

/// The region requests to object storage are signed for: RustFS's own
/// unless its owner sets another.
pub const S3_REGION: &str = "us-east-1";

impl fmt::Debug for RemoteStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteStore")
            .field("scheme", &self.op.info().scheme())
            .field("root", &self.op.info().root())
            .finish_non_exhaustive()
    }
}

/// Check that `path` is relative and made of plain segments
/// (`[A-Za-z0-9._-]`, not `.` or `..`).
pub fn check_relative_path(path: &str) -> Result<(), StoreError> {
    let plain = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if path.is_empty() || path.len() > 1024 || !path.split('/').all(plain) {
        return Err(StoreError::Invalid(format!(
            "'{path}' is not a relative path of plain names"
        )));
    }
    Ok(())
}

/// Check a path to one of the owner's own files or folders on a storage,
/// viewed in place (§7.7): relative, with any names the owner gave them
/// (spaces, accents, other scripts), but no `.`, `..`, empty segments,
/// backslashes or control characters. Oikome only reads these.
pub fn check_owner_path(path: &str) -> Result<(), StoreError> {
    let fine = |s: &str| {
        !s.is_empty() && s != "." && s != ".." && !s.chars().any(|c| c.is_control() || c == '\\')
    };
    if path.is_empty() || path.len() > 4096 || !path.split('/').all(fine) {
        return Err(StoreError::Invalid(format!(
            "'{path}' is not a relative path within the folder"
        )));
    }
    Ok(())
}

/// A file or folder the owner keeps on a storage, as listed for viewing in
/// place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Milliseconds since the Unix epoch; 0 when the storage doesn't say.
    pub modified_ms: i64,
}

/// A temporary name next to `path`, unique within this process.
fn temp_name(path: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    match path.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/.tmp-{name}-{pid}-{n}"),
        None => format!(".tmp-{path}-{pid}-{n}"),
    }
}

impl RemoteStore {
    /// The folder `folder` (plain segments; `""` for the top) under the
    /// WebDAV server at `endpoint` (`https://host[:port][/path]`).
    pub fn webdav(
        endpoint: &str,
        folder: &str,
        credentials: Credentials,
    ) -> Result<Self, StoreError> {
        if !folder.is_empty() {
            check_relative_path(folder)?;
        }
        let mut base = url::Url::parse(endpoint)
            .map_err(|e| StoreError::Invalid(format!("WebDAV address: {e}")))?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(StoreError::Invalid(
                "a WebDAV address is http(s)://host[:port][/path], without credentials".into(),
            ));
        }
        {
            let mut segments = base
                .path_segments_mut()
                .map_err(|_| StoreError::Invalid("WebDAV address without a path".into()))?;
            segments.pop_if_empty();
            for segment in folder.split('/').filter(|s| !s.is_empty()) {
                segments.push(segment);
            }
            segments.push("");
        }
        let root = match folder {
            "" => "/".to_string(),
            f => format!("/{f}/"),
        };
        let mut builder = opendal::services::Webdav::default()
            .endpoint(endpoint.trim_end_matches('/'))
            .root(&root);
        if let Some(user) = &credentials.username {
            builder = builder.username(user);
        }
        if let Some(password) = &credentials.password {
            builder = builder.password(password.as_str());
        }
        if let Some(token) = &credentials.token {
            builder = builder.token(token.as_str());
        }
        crate::ready();
        let op = Operator::new(builder)
            .map_err(|e| StoreError::Invalid(format!("WebDAV: {}", e.kind())))?;
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| StoreError::Other(format!("HTTP client: {e}")))?;
        Ok(Self {
            op,
            put: Some(WebdavPut {
                client,
                base,
                credentials,
            }),
            bucket: None,
        })
    }

    /// The owner's folder `folder` under the WebDAV server at `endpoint`,
    /// to view in place: read only, so its names need only be the owner's
    /// own ([`check_owner_path`]); nothing is ever written through it.
    pub fn webdav_view(
        endpoint: &str,
        folder: &str,
        credentials: Credentials,
    ) -> Result<Self, StoreError> {
        let folder = folder.trim_matches('/');
        if !folder.is_empty() {
            check_owner_path(folder)?;
        }
        let base = url::Url::parse(endpoint)
            .map_err(|e| StoreError::Invalid(format!("WebDAV address: {e}")))?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(StoreError::Invalid(
                "a WebDAV address is http(s)://host[:port][/path], without credentials".into(),
            ));
        }
        let root = match folder {
            "" => "/".to_string(),
            f => format!("/{f}/"),
        };
        let mut builder = opendal::services::Webdav::default()
            .endpoint(endpoint.trim_end_matches('/'))
            .root(&root);
        if let Some(user) = &credentials.username {
            builder = builder.username(user);
        }
        if let Some(password) = &credentials.password {
            builder = builder.password(password.as_str());
        }
        if let Some(token) = &credentials.token {
            builder = builder.token(token.as_str());
        }
        crate::ready();
        let op = Operator::new(builder)
            .map_err(|e| StoreError::Invalid(format!("WebDAV: {}", e.kind())))?;
        Ok(Self {
            op,
            put: None,
            bucket: None,
        })
    }

    /// The owner's entries in the folder `dir` (`""` for the top), with
    /// their sizes and times, to view in place.
    pub async fn list_owned(&self, dir: &str) -> Result<Vec<OwnerEntry>, StoreError> {
        let dir = match dir.trim_matches('/') {
            "" => String::new(),
            d => {
                check_owner_path(d)?;
                format!("{d}/")
            }
        };
        let entries = self
            .op
            .list(&dir)
            .await
            .map_err(|e| StoreError::from_dal(&dir, e))?;
        let mut out = Vec::new();
        for entry in entries {
            let Some(name) = entry
                .path()
                .strip_prefix(dir.as_str())
                .map(|n| n.trim_end_matches('/').to_string())
            else {
                continue;
            };
            if name.is_empty() || name.contains('/') {
                continue;
            }
            let is_dir = entry.metadata().mode() == EntryMode::DIR;
            let (mut size, mut modified) = (
                entry.metadata().content_length(),
                entry.metadata().last_modified(),
            );
            // Not every server says in its listing.
            if !is_dir && modified.is_none() {
                if let Ok(meta) = self.op.stat(entry.path()).await {
                    size = meta.content_length();
                    modified = meta.last_modified();
                }
            }
            out.push(OwnerEntry {
                name,
                is_dir,
                size,
                modified_ms: modified.map_or(0, |t| {
                    std::time::SystemTime::from(t)
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                }),
            });
        }
        Ok(out)
    }

    /// `range` of the owner's file at `path` (all of it with `..`), at most
    /// `max` bytes, to view in place.
    pub async fn read_owned(
        &self,
        path: &str,
        range: std::ops::Range<u64>,
        max: u64,
    ) -> Result<Bytes, StoreError> {
        check_owner_path(path)?;
        let size = self
            .op
            .stat(path)
            .await
            .map_err(|e| StoreError::from_dal(path, e))?
            .content_length();
        let end = range.end.min(size);
        if range.start > end {
            return Ok(Bytes::new());
        }
        if end - range.start > max {
            return Err(StoreError::Invalid(format!(
                "{path}: more than {max} bytes asked for"
            )));
        }
        self.op
            .read_with(path)
            .range(range.start..end)
            .await
            .map(|b| b.to_bytes())
            .map_err(|e| StoreError::from_dal(path, e))
    }

    /// The folder `folder` (plain segments; `""` for `root` itself) under
    /// the local directory `root`, which must be an absolute path and
    /// exist: a share that is not mounted is not replaced by an empty local
    /// directory.
    pub fn folder(root: &Path, folder: &str) -> Result<Self, StoreError> {
        if !folder.is_empty() {
            check_relative_path(folder)?;
        }
        if !root.is_absolute() {
            return Err(StoreError::Invalid(format!(
                "'{}' is not an absolute path",
                root.display()
            )));
        }
        if !root.is_dir() {
            return Err(StoreError::Invalid(format!(
                "folder '{}' does not exist; is the share mounted?",
                root.display()
            )));
        }
        let dir = if folder.is_empty() {
            root.to_path_buf()
        } else {
            root.join(folder)
        };
        let dir = dir
            .to_str()
            .ok_or_else(|| StoreError::Invalid("a folder path that is not UTF-8".into()))?;
        crate::ready();
        let op = Operator::new(opendal::services::Fs::default().root(dir))
            .map_err(|e| StoreError::Invalid(format!("folder: {}", e.kind())))?;
        Ok(Self {
            op,
            put: None,
            bucket: None,
        })
    }

    /// The folder `folder` (plain segments; `""` for the bucket's top) in
    /// a bucket on RustFS, over the S3 protocol, signed with exactly
    /// `access_key_id` and `secret`: never with keys from the environment,
    /// `~/.aws` or a cloud machine's metadata.
    pub fn s3(
        bucket: Bucket<'_>,
        folder: &str,
        access_key_id: &str,
        secret: &str,
    ) -> Result<Self, StoreError> {
        if !folder.is_empty() {
            check_relative_path(folder)?;
        }
        Self::s3_at(bucket, folder, access_key_id, secret)
    }

    /// The owner's folder `folder` in a bucket, to view in place: read
    /// only, so its names need only be the owner's own ([`check_owner_path`]).
    pub fn s3_view(
        bucket: Bucket<'_>,
        folder: &str,
        access_key_id: &str,
        secret: &str,
    ) -> Result<Self, StoreError> {
        let folder = folder.trim_matches('/');
        if !folder.is_empty() {
            check_owner_path(folder)?;
        }
        Self::s3_at(bucket, folder, access_key_id, secret)
    }

    fn s3_at(
        bucket: Bucket<'_>,
        folder: &str,
        access_key_id: &str,
        secret: &str,
    ) -> Result<Self, StoreError> {
        let root = match folder {
            "" => "/".to_string(),
            f => format!("/{f}/"),
        };
        crate::ready();
        let builder = opendal::services::S3::default()
            .endpoint(bucket.endpoint)
            .bucket(bucket.name)
            .region(S3_REGION)
            .root(&root)
            .access_key_id(access_key_id)
            .secret_access_key(secret)
            .disable_config_load()
            .disable_ec2_metadata();
        let op = Operator::new(builder)
            .map_err(|e| StoreError::Invalid(format!("object storage: {}", e.kind())))?;
        Ok(Self {
            op,
            put: None,
            bucket: Some(bucket.name.to_string()),
        })
    }

    /// The folder `folder` (plain segments) of the owner's Google Drive,
    /// reached as Oikome with `client` and the owner's `refresh_token`.
    /// Only files Oikome made are seen (scope `drive.file`).
    pub fn gdrive(
        folder: &str,
        client: &crate::GoogleClient,
        refresh_token: &str,
    ) -> Result<Self, StoreError> {
        check_relative_path(folder)?;
        crate::ready();
        let builder = opendal::services::Gdrive::default()
            .root(&format!("/{folder}/"))
            .client_id(client.client_id())
            .client_secret(client.client_secret())
            .refresh_token(refresh_token);
        let op = Operator::new(builder)
            .map_err(|e| StoreError::Invalid(format!("Google Drive: {}", e.kind())))?;
        Ok(Self {
            op,
            put: None,
            bucket: None,
        })
    }

    /// The owner's folder `folder` (`""` for the top) of their Google
    /// Drive, to view in place: read only, so its names need only be the
    /// owner's own ([`check_owner_path`]). Needs the drive's read access
    /// (`drive.readonly`).
    pub fn gdrive_view(
        folder: &str,
        client: &crate::GoogleClient,
        refresh_token: &str,
    ) -> Result<Self, StoreError> {
        let folder = folder.trim_matches('/');
        let root = if folder.is_empty() {
            "/".to_string()
        } else {
            check_owner_path(folder)?;
            format!("/{folder}/")
        };
        crate::ready();
        let builder = opendal::services::Gdrive::default()
            .root(&root)
            .client_id(client.client_id())
            .client_secret(client.client_secret())
            .refresh_token(refresh_token);
        let op = Operator::new(builder)
            .map_err(|e| StoreError::Invalid(format!("Google Drive: {}", e.kind())))?;
        Ok(Self {
            op,
            put: None,
            bucket: None,
        })
    }

    /// Whether a write lands whole at its path, so it needs no temporary
    /// name to rename: object storage has no rename, and a PUT there is
    /// seen complete or not at all. So is an upload to Google Drive, whose
    /// rename would also put the file it replaces in the drive's trash.
    fn writes_whole(&self) -> bool {
        let info = self.op.info();
        !info.capability().rename || info.scheme() == opendal::services::GDRIVE_SCHEME
    }

    /// Check that the folder exists, the storage answers and it accepts
    /// the credentials.
    pub async fn check(&self) -> Result<(), StoreError> {
        // On Google Drive Oikome sees only what it made, its folder too:
        // made here when it isn't there yet.
        if self.op.info().scheme() == opendal::services::GDRIVE_SCHEME {
            self.op
                .create_dir("/")
                .await
                .map_err(|e| StoreError::from_dal("/", e))?;
        }
        // opendal answers a stat of the root itself; a listing asks the
        // storage.
        match self.op.list("/").await {
            Ok(_) => Ok(()),
            // opendal's S3 calls NoSuchBucket (404) a configuration error.
            Err(e)
                if e.kind() == ErrorKind::NotFound
                    || (self.bucket.is_some() && e.kind() == ErrorKind::ConfigInvalid) =>
            {
                Err(StoreError::Invalid(match &self.bucket {
                    Some(bucket) => {
                        format!("the bucket {bucket} doesn't exist; create it on the storage first")
                    }
                    None => "the folder does not exist on the storage".into(),
                }))
            }
            Err(e) => Err(StoreError::from_dal("/", e)),
        }
    }

    /// The size of the file at `path`, or `None` if there is none.
    pub async fn size(&self, path: &str) -> Result<Option<u64>, StoreError> {
        check_relative_path(path)?;
        match self.op.stat(path).await {
            Ok(meta) if meta.mode() == EntryMode::FILE => Ok(Some(meta.content_length())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::from_dal(path, e)),
        }
    }

    /// The entries of the folder `dir` (`""` for the Space's folder); empty
    /// if the folder does not exist.
    pub async fn list(&self, dir: &str) -> Result<Vec<StoreEntry>, StoreError> {
        let dir = match dir {
            "" => String::new(),
            d => {
                check_relative_path(d)?;
                format!("{d}/")
            }
        };
        let entries = match self.op.list(&dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError::from_dal(&dir, e)),
        };
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                // Some servers list the folder itself.
                let name = entry
                    .path()
                    .strip_prefix(dir.as_str())?
                    .trim_end_matches('/');
                (!name.is_empty() && !name.contains('/')).then(|| StoreEntry {
                    name: name.to_string(),
                    is_dir: entry.metadata().mode() == EntryMode::DIR,
                })
            })
            .collect())
    }

    /// The whole file at `path`, which must not be larger than `max`
    /// bytes; for small files.
    pub async fn read(&self, path: &str, max: u64) -> Result<Bytes, StoreError> {
        check_relative_path(path)?;
        // One request, not a stat and then a read: on a drive each is a
        // round trip. The files read so are Oikome's own small ones (its
        // record, snapshots, claims), so reading one whole is no burden.
        let bytes = self
            .op
            .read(path)
            .await
            .map(|b| b.to_bytes())
            .map_err(|e| StoreError::from_dal(path, e))?;
        if bytes.len() as u64 > max {
            return Err(StoreError::Invalid(format!(
                "{path} is larger than {max} bytes"
            )));
        }
        Ok(bytes)
    }

    /// The file at `path` as a stream of chunks.
    pub async fn reader(
        &self,
        path: &str,
    ) -> Result<BoxStream<'static, Result<Bytes, StoreError>>, StoreError> {
        check_relative_path(path)?;
        let owned = path.to_string();
        let reader = self
            .op
            .reader_with(path)
            .chunk(READ_CHUNK_BYTES)
            .await
            .map_err(|e| StoreError::from_dal(path, e))?;
        let chunks = reader
            .into_bytes_stream(..)
            .await
            .map_err(|e| StoreError::from_dal(path, e))?;
        Ok(chunks
            .map_err(move |e| StoreError::Other(format!("reading {owned}: {e}")))
            .boxed())
    }

    /// Write `bytes` to `path`, replacing any file there.
    pub async fn write(&self, path: &str, bytes: Bytes) -> Result<(), StoreError> {
        check_relative_path(path)?;
        let whole = self.writes_whole();
        let target = if whole {
            path.to_string()
        } else {
            temp_name(path)
        };
        let size = bytes.len() as u64;
        let written = async {
            self.op
                .write(&target, bytes)
                .await
                .map_err(|e| StoreError::from_dal(path, e))?;
            // Written in place, the storage holds it whole or not at all.
            if whole {
                return Ok(());
            }
            self.finish(&target, path, size).await
        }
        .await;
        // A failed PUT leaves the file there before as it was: only a
        // temporary file is cleaned up.
        if written.is_err() && !whole {
            let _ = self.op.delete(&target).await;
        }
        written
    }

    /// Copy the local file `local` to `path`, replacing any file there.
    /// Returns the bytes uploaded.
    pub async fn upload(&self, local: &Path, path: &str) -> Result<u64, StoreError> {
        let (size, chunks) = file_chunks(local).await?;
        self.upload_stream(path, size, chunks).await
    }

    /// Write the `size` bytes of `chunks` to `path`, replacing any file
    /// there. Returns the bytes uploaded.
    pub async fn upload_stream(
        &self,
        path: &str,
        size: u64,
        chunks: BoxStream<'static, std::io::Result<Bytes>>,
    ) -> Result<u64, StoreError> {
        check_relative_path(path)?;
        let whole = self.writes_whole();
        let tmp = if whole {
            path.to_string()
        } else {
            temp_name(path)
        };
        let written = async {
            match &self.put {
                Some(put) => self.put_stream(put, &tmp, size, chunks).await?,
                None => {
                    let mut writer = self
                        .op
                        .writer_with(&tmp)
                        .chunk(UPLOAD_PART_BYTES)
                        .await
                        .map_err(|e| StoreError::from_dal(path, e))?;
                    let mut chunks = chunks;
                    let mut sent = 0u64;
                    while let Some(chunk) = chunks
                        .try_next()
                        .await
                        .map_err(|e| StoreError::Other(format!("{path}: {e}")))?
                    {
                        sent += chunk.len() as u64;
                        writer
                            .write(chunk)
                            .await
                            .map_err(|e| StoreError::from_dal(path, e))?;
                    }
                    // Written in place, the upload completes whole or not at
                    // all: the bytes sent tell its size, without asking the
                    // storage again.
                    if whole && sent != size {
                        let _ = writer.abort().await;
                        return Err(StoreError::Other(format!(
                            "{path}: {sent} of {size} bytes were read to send"
                        )));
                    }
                    writer
                        .close()
                        .await
                        .map_err(|e| StoreError::from_dal(path, e))?;
                    if whole {
                        return Ok(());
                    }
                }
            }
            self.finish(&tmp, path, size).await
        }
        .await;
        if written.is_err() && !whole {
            let _ = self.op.delete(&tmp).await;
        }
        written.map(|_| size)
    }

    /// PUT a stream of `size` bytes to the WebDAV path `tmp`.
    async fn put_stream(
        &self,
        put: &WebdavPut,
        tmp: &str,
        size: u64,
        chunks: BoxStream<'static, std::io::Result<Bytes>>,
    ) -> Result<(), StoreError> {
        if let Some((dir, _)) = tmp.rsplit_once('/') {
            self.op
                .create_dir(&format!("{dir}/"))
                .await
                .map_err(|e| StoreError::from_dal(dir, e))?;
        }
        let mut url = put.base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| StoreError::Invalid("WebDAV address without a path".into()))?;
            segments.pop_if_empty();
            for segment in tmp.split('/') {
                segments.push(segment);
            }
        }
        let mut request = put
            .client
            .put(url)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(reqwest::Body::wrap_stream(chunks));
        if let Some(token) = &put.credentials.token {
            request = request.bearer_auth(token.as_str());
        } else if let Some(user) = &put.credentials.username {
            request = request.basic_auth(user, put.credentials.password.as_deref());
        }
        let response = request
            .send()
            .await
            .map_err(|e| StoreError::Other(format!("uploading {tmp}: {}", e.without_url())))?;
        match response.status() {
            s if s.is_success() => Ok(()),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(StoreError::Denied)
            }
            s => Err(StoreError::Other(format!("uploading {tmp}: HTTP {s}"))),
        }
    }

    /// Check the size of the uploaded `tmp`, then rename it to `path`
    /// (unless it was written there whole).
    async fn finish(&self, tmp: &str, path: &str, size: u64) -> Result<(), StoreError> {
        let written = self
            .op
            .stat(tmp)
            .await
            .map_err(|e| StoreError::from_dal(path, e))?
            .content_length();
        if written != size {
            return Err(StoreError::Other(format!(
                "{path}: the storage holds {written} of {size} bytes"
            )));
        }
        if tmp == path {
            return Ok(());
        }
        self.op
            .rename(tmp, path)
            .await
            .map_err(|e| StoreError::from_dal(path, e))
    }

    /// Delete the file at `path`; a missing file is not an error.
    pub async fn delete(&self, path: &str) -> Result<(), StoreError> {
        check_relative_path(path)?;
        self.op
            .delete(path)
            .await
            .map_err(|e| StoreError::from_dal(path, e))
    }

    /// Delete the folder `dir` and everything in it.
    pub async fn delete_dir(&self, dir: &str) -> Result<(), StoreError> {
        check_relative_path(dir)?;
        self.op
            .delete_with(&format!("{dir}/"))
            .recursive(true)
            .await
            .map_err(|e| StoreError::from_dal(dir, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> (tempfile::TempDir, RemoteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RemoteStore::folder(dir.path(), "space").unwrap();
        (dir, store)
    }

    #[test]
    fn paths_are_plain_and_relative() {
        for good in ["a", "a/b.json", "files/0a1b.parquet", "x/.tmp-y-1-2"] {
            check_relative_path(good).unwrap();
        }
        for bad in [
            "", "/a", "a/", "a//b", "../a", "a/../b", "./a", "a b", "a\\b", "a?b",
        ] {
            assert!(check_relative_path(bad).is_err(), "{bad}");
        }
        assert!(RemoteStore::folder(Path::new("relative"), "s").is_err());
        assert!(RemoteStore::webdav("https://u:p@nas/dav", "s", Credentials::default()).is_err());
        assert!(RemoteStore::webdav("ftp://nas", "s", Credentials::default()).is_err());
        assert!(RemoteStore::webdav("https://nas/dav", "../s", Credentials::default()).is_err());
    }

    #[test]
    fn credentials_are_redacted() {
        let credentials = Credentials {
            username: Some("me".into()),
            password: Some(Zeroizing::new("hunter2".into())),
            token: Some(Zeroizing::new("tok".into())),
        };
        let shown = format!("{credentials:?}");
        assert!(shown.contains("me") && !shown.contains("hunter2") && !shown.contains("tok\""));
    }

    #[test]
    fn only_the_status_of_an_http_error_is_kept() {
        assert_eq!(
            http_status("Unexpected at stat, response: Parts { status: 409, version"),
            Some(409)
        );
        assert_eq!(http_status("no status here"), None);
    }

    #[tokio::test]
    async fn files_are_written_whole_and_listed() {
        let (dir, store) = folder();
        RemoteStore::folder(dir.path(), "")
            .unwrap()
            .check()
            .await
            .unwrap();
        assert!(RemoteStore::folder(&dir.path().join("unmounted"), "s").is_err());
        assert_eq!(store.list("").await.unwrap(), vec![]);
        assert_eq!(store.size("a/b.json").await.unwrap(), None);
        store.write("a/b.json", Bytes::from("one")).await.unwrap();
        store.write("a/b.json", Bytes::from("three")).await.unwrap();
        assert_eq!(
            store.read("a/b.json", 5).await.unwrap(),
            Bytes::from("three")
        );
        assert!(store.read("a/b.json", 4).await.is_err());
        assert_eq!(store.size("a/b.json").await.unwrap(), Some(5));

        let local = dir.path().join("local.bin");
        let data: Vec<u8> = (0..3 * UPLOAD_CHUNK_BYTES + 7).map(|i| i as u8).collect();
        std::fs::write(&local, &data).unwrap();
        assert_eq!(
            store.upload(&local, "a/c/d.bin").await.unwrap(),
            data.len() as u64
        );
        let read: Vec<Bytes> = store
            .reader("a/c/d.bin")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(read.concat(), data);

        let mut listed = store.list("a").await.unwrap();
        listed.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(
            listed,
            vec![
                StoreEntry {
                    name: "b.json".into(),
                    is_dir: false,
                },
                StoreEntry {
                    name: "c".into(),
                    is_dir: true,
                },
            ]
        );
        assert!(matches!(
            store.read("a/missing", 10).await,
            Err(StoreError::NotFound(_))
        ));
        store.delete("a/b.json").await.unwrap();
        store.delete("a/b.json").await.unwrap();
        store.delete_dir("a/c").await.unwrap();
        assert_eq!(store.list("a").await.unwrap(), vec![]);
    }
}
