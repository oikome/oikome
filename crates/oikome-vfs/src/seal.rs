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

//! Client-side encryption of the files on a storage (architecture v3 §7.5).
//!
//! A storage has one 256-bit key. From it come a content key, a names key
//! and an 8-byte key id (BLAKE3 `derive_key`). A file is sealed in segments
//! of [`SEGMENT_BYTES`] with XChaCha20-Poly1305, in the STREAM construction:
//! each segment's nonce is a random per-file prefix, the segment counter
//! and a flag marking the last segment, so segments cannot be reordered,
//! dropped or cut off unnoticed. Every segment is bound to the file's name
//! on the storage (associated data), so files cannot be swapped either.
//!
//! ```text
//! "OIKS" | version (1) | key id (8) | nonce prefix (19) | segment | ... | last segment
//! segment = ciphertext of up to SEGMENT_BYTES bytes | tag (16)
//! ```
//!
//! Names on the storage come from the names key ([`SealKey::object_name`]),
//! so the storage sees neither table names nor data hashes.

use std::fmt;

use base64::Engine;
use bytes::{Bytes, BytesMut};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use futures::stream::{self, BoxStream};
use futures::{StreamExt, TryStreamExt};
use zeroize::Zeroizing;

/// Plaintext bytes per segment.
pub const SEGMENT_BYTES: usize = 64 << 10;
const MAGIC: &[u8; 4] = b"OIKS";
const VERSION: u8 = 1;
const KEY_ID_BYTES: usize = 8;
const PREFIX_BYTES: usize = 19;
const HEADER_BYTES: usize = MAGIC.len() + 1 + KEY_ID_BYTES + PREFIX_BYTES;
const TAG_BYTES: usize = 16;
const KEY_BYTES: usize = 32;

const CONTENT_CONTEXT: &str = "oikome 2026-11-02 storage file content";
const NAMES_CONTEXT: &str = "oikome 2026-11-02 storage file names";
const KEY_ID_CONTEXT: &str = "oikome 2026-11-02 storage key id";

/// A file that could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// The file was sealed with another key.
    #[error("the file was encrypted with another key")]
    WrongKey,
    /// The file is not a sealed file, or was changed, cut short or renamed.
    #[error("the file is damaged or was changed")]
    Damaged,
    #[error("{0}")]
    Io(String),
}

/// A storage's encryption key. Never shown by `Debug`.
pub struct SealKey {
    raw: Zeroizing<[u8; KEY_BYTES]>,
    content: XChaCha20Poly1305,
    names: Zeroizing<[u8; KEY_BYTES]>,
    id: [u8; KEY_ID_BYTES],
}

impl fmt::Debug for SealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealKey").finish_non_exhaustive()
    }
}

impl SealKey {
    fn from_raw(raw: Zeroizing<[u8; KEY_BYTES]>) -> Self {
        let content = Zeroizing::new(blake3::derive_key(CONTENT_CONTEXT, raw.as_ref()));
        let names = Zeroizing::new(blake3::derive_key(NAMES_CONTEXT, raw.as_ref()));
        let mut id = [0u8; KEY_ID_BYTES];
        id.copy_from_slice(&blake3::derive_key(KEY_ID_CONTEXT, raw.as_ref())[..KEY_ID_BYTES]);
        Self {
            content: XChaCha20Poly1305::new(content.as_ref().into()),
            names,
            id,
            raw,
        }
    }

    /// A new random key.
    pub fn generate() -> Result<Self, SealError> {
        let mut raw = Zeroizing::new([0u8; KEY_BYTES]);
        getrandom::fill(raw.as_mut())
            .map_err(|e| SealError::Io(format!("no randomness for a key: {e}")))?;
        Ok(Self::from_raw(raw))
    }

    /// A key in its text form (see [`SealKey::export`]).
    pub fn parse(text: &str) -> Result<Self, SealError> {
        let bad = || SealError::Io("not an Oikome storage key".to_string());
        let bytes = Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(text.trim())
                .map_err(|_| bad())?,
        );
        let raw: [u8; KEY_BYTES] = bytes.as_slice().try_into().map_err(|_| bad())?;
        Ok(Self::from_raw(Zeroizing::new(raw)))
    }

    /// The key as text (base64url), to store or move to another node.
    pub fn export(&self) -> Zeroizing<String> {
        Zeroizing::new(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.raw.as_ref()))
    }

    /// The name on the storage of the file whose plain path is `logical`.
    pub fn object_name(&self, logical: &str) -> String {
        blake3::keyed_hash(&self.names, logical.as_bytes())
            .to_hex()
            .to_string()
    }

    fn nonce(prefix: &[u8; PREFIX_BYTES], counter: u32, last: bool) -> XNonce {
        let mut nonce = [0u8; 24];
        nonce[..PREFIX_BYTES].copy_from_slice(prefix);
        nonce[PREFIX_BYTES..PREFIX_BYTES + 4].copy_from_slice(&counter.to_be_bytes());
        nonce[23] = u8::from(last);
        nonce.into()
    }

    fn seal_segment(
        &self,
        prefix: &[u8; PREFIX_BYTES],
        counter: u32,
        last: bool,
        aad: &[u8],
        plain: &[u8],
    ) -> Result<Vec<u8>, SealError> {
        self.content
            .encrypt(
                &Self::nonce(prefix, counter, last),
                Payload { msg: plain, aad },
            )
            .map_err(|_| SealError::Io("encryption failed".to_string()))
    }

    fn open_segment(
        &self,
        prefix: &[u8; PREFIX_BYTES],
        counter: u32,
        last: bool,
        aad: &[u8],
        sealed: &[u8],
    ) -> Result<Vec<u8>, SealError> {
        self.content
            .decrypt(
                &Self::nonce(prefix, counter, last),
                Payload { msg: sealed, aad },
            )
            .map_err(|_| SealError::Damaged)
    }

    fn header(&self) -> Result<([u8; PREFIX_BYTES], Vec<u8>), SealError> {
        let mut prefix = [0u8; PREFIX_BYTES];
        getrandom::fill(&mut prefix)
            .map_err(|e| SealError::Io(format!("no randomness for a nonce: {e}")))?;
        let mut header = Vec::with_capacity(HEADER_BYTES);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&self.id);
        header.extend_from_slice(&prefix);
        Ok((prefix, header))
    }

    /// Check a file's header; returns its nonce prefix.
    fn read_header(&self, header: &[u8]) -> Result<[u8; PREFIX_BYTES], SealError> {
        if header.len() != HEADER_BYTES || &header[..4] != MAGIC || header[4] != VERSION {
            return Err(SealError::Damaged);
        }
        if header[5..5 + KEY_ID_BYTES] != self.id {
            return Err(SealError::WrongKey);
        }
        let mut prefix = [0u8; PREFIX_BYTES];
        prefix.copy_from_slice(&header[5 + KEY_ID_BYTES..]);
        Ok(prefix)
    }
}

/// The size of `plain` bytes once sealed.
pub fn sealed_size(plain: u64) -> u64 {
    let segments = plain.div_ceil(SEGMENT_BYTES as u64).max(1);
    HEADER_BYTES as u64 + plain + segments * TAG_BYTES as u64
}

fn next_counter(counter: u32) -> Result<u32, SealError> {
    counter
        .checked_add(1)
        .ok_or_else(|| SealError::Io("file too large to encrypt".to_string()))
}

/// Seal `plain` as the file named `name` on the storage.
pub fn seal_bytes(key: &SealKey, name: &str, plain: &[u8]) -> Result<Vec<u8>, SealError> {
    let (prefix, mut out) = key.header()?;
    let mut chunks = plain.chunks(SEGMENT_BYTES).peekable();
    let mut counter = 0u32;
    if chunks.peek().is_none() {
        out.extend(key.seal_segment(&prefix, 0, true, name.as_bytes(), &[])?);
        return Ok(out);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        out.extend(key.seal_segment(&prefix, counter, last, name.as_bytes(), chunk)?);
        counter = next_counter(counter)?;
    }
    Ok(out)
}

/// Open the sealed file named `name`.
pub fn open_bytes(key: &SealKey, name: &str, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
    if sealed.len() < HEADER_BYTES + TAG_BYTES {
        return Err(SealError::Damaged);
    }
    let prefix = key.read_header(&sealed[..HEADER_BYTES])?;
    let mut segments = sealed[HEADER_BYTES..]
        .chunks(SEGMENT_BYTES + TAG_BYTES)
        .peekable();
    let mut out = Vec::with_capacity(sealed.len());
    let mut counter = 0u32;
    while let Some(segment) = segments.next() {
        let last = segments.peek().is_none();
        out.extend(key.open_segment(&prefix, counter, last, name.as_bytes(), segment)?);
        counter = next_counter(counter)?;
    }
    Ok(out)
}

/// What [`seal_stream`] and [`open_stream`] carry between chunks.
struct Pump<S> {
    input: S,
    buf: BytesMut,
    counter: u32,
    prefix: Option<[u8; PREFIX_BYTES]>,
    ended: bool,
    done: bool,
}

/// Seal `plain` as the file named `name` on the storage, as a stream of
/// [`sealed_size`] bytes.
pub fn seal_stream(
    key: std::sync::Arc<SealKey>,
    name: String,
    plain: BoxStream<'static, std::io::Result<Bytes>>,
) -> BoxStream<'static, std::io::Result<Bytes>> {
    let pump = Pump {
        input: plain,
        buf: BytesMut::new(),
        counter: 0,
        prefix: None,
        ended: false,
        done: false,
    };
    let io = |e: SealError| std::io::Error::other(e.to_string());
    stream::try_unfold(pump, move |mut p| {
        let (key, name) = (key.clone(), name.clone());
        async move {
            if p.done {
                return Ok(None);
            }
            let Some(prefix) = p.prefix else {
                let (prefix, header) = key.header().map_err(io)?;
                p.prefix = Some(prefix);
                return Ok(Some((Bytes::from(header), p)));
            };
            // A segment is the last one only once the input has ended, so
            // keep more than a segment buffered before sealing one.
            while !p.ended && p.buf.len() <= SEGMENT_BYTES {
                match p.input.try_next().await? {
                    Some(chunk) => p.buf.extend_from_slice(&chunk),
                    None => p.ended = true,
                }
            }
            let last = p.ended && p.buf.len() <= SEGMENT_BYTES;
            let take = p.buf.len().min(SEGMENT_BYTES);
            let plain = p.buf.split_to(take);
            let sealed = key
                .seal_segment(&prefix, p.counter, last, name.as_bytes(), &plain)
                .map_err(io)?;
            p.counter = next_counter(p.counter).map_err(io)?;
            p.done = last;
            Ok(Some((Bytes::from(sealed), p)))
        }
    })
    .boxed()
}

/// Read from `p.input` until `p.buf` holds `want` bytes or the input ends.
async fn fill<E: fmt::Display>(
    p: &mut Pump<BoxStream<'static, Result<Bytes, E>>>,
    want: usize,
) -> Result<(), SealError> {
    while !p.ended && p.buf.len() < want {
        match p
            .input
            .try_next()
            .await
            .map_err(|e| SealError::Io(e.to_string()))?
        {
            Some(chunk) => p.buf.extend_from_slice(&chunk),
            None => p.ended = true,
        }
    }
    Ok(())
}

/// Open the sealed file named `name` from a stream of its bytes.
pub fn open_stream<E: fmt::Display + Send + 'static>(
    key: std::sync::Arc<SealKey>,
    name: String,
    sealed: BoxStream<'static, Result<Bytes, E>>,
) -> BoxStream<'static, Result<Bytes, SealError>> {
    let pump = Pump {
        input: sealed,
        buf: BytesMut::new(),
        counter: 0,
        prefix: None,
        ended: false,
        done: false,
    };
    const SEALED_SEGMENT: usize = SEGMENT_BYTES + TAG_BYTES;
    stream::try_unfold(pump, move |mut p| {
        let (key, name) = (key.clone(), name.clone());
        async move {
            if p.done {
                return Ok(None);
            }
            let prefix = match p.prefix {
                Some(prefix) => prefix,
                None => {
                    fill(&mut p, HEADER_BYTES).await?;
                    if p.buf.len() < HEADER_BYTES {
                        return Err(SealError::Damaged);
                    }
                    let header = p.buf.split_to(HEADER_BYTES);
                    let prefix = key.read_header(&header)?;
                    p.prefix = Some(prefix);
                    prefix
                }
            };
            // More than a segment buffered, or the input ended: only then is
            // it known whether the next segment is the last.
            fill(&mut p, SEALED_SEGMENT + 1).await?;
            let last = p.ended && p.buf.len() <= SEALED_SEGMENT;
            if p.buf.len() < TAG_BYTES {
                return Err(SealError::Damaged);
            }
            let take = p.buf.len().min(SEALED_SEGMENT);
            let segment = p.buf.split_to(take);
            let plain = key.open_segment(&prefix, p.counter, last, name.as_bytes(), &segment)?;
            p.counter = next_counter(p.counter)?;
            p.done = last;
            Ok(Some((Bytes::from(plain), p)))
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn key() -> Arc<SealKey> {
        Arc::new(SealKey::generate().unwrap())
    }

    fn plain(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    /// `data` as a stream in uneven chunks.
    fn chunked(data: Vec<u8>) -> BoxStream<'static, std::io::Result<Bytes>> {
        let mut parts = Vec::new();
        let mut rest = Bytes::from(data);
        let mut size = 1;
        while !rest.is_empty() {
            let n = size.min(rest.len());
            parts.push(Ok(rest.split_to(n)));
            size = size * 3 + 1000;
        }
        stream::iter(parts).boxed()
    }

    async fn seal_all(key: &Arc<SealKey>, name: &str, data: Vec<u8>) -> Vec<u8> {
        let parts: Vec<Bytes> = seal_stream(key.clone(), name.into(), chunked(data))
            .try_collect()
            .await
            .unwrap();
        parts.concat()
    }

    async fn open_all(
        key: &Arc<SealKey>,
        name: &str,
        sealed: Vec<u8>,
    ) -> Result<Vec<u8>, SealError> {
        let parts: Vec<Bytes> = open_stream(key.clone(), name.into(), chunked(sealed))
            .try_collect()
            .await?;
        Ok(parts.concat())
    }

    #[tokio::test]
    async fn files_round_trip_at_every_size() {
        let key = key();
        for n in [
            0,
            1,
            SEGMENT_BYTES - 1,
            SEGMENT_BYTES,
            SEGMENT_BYTES + 1,
            3 * SEGMENT_BYTES + 5,
        ] {
            let data = plain(n);
            let sealed = seal_all(&key, "objects/a", data.clone()).await;
            assert_eq!(sealed.len() as u64, sealed_size(n as u64), "{n}");
            assert_eq!(
                open_all(&key, "objects/a", sealed.clone()).await.unwrap(),
                data,
                "{n}"
            );
            // The whole-buffer forms agree with the streams.
            assert_eq!(open_bytes(&key, "objects/a", &sealed).unwrap(), data, "{n}");
            let small = seal_bytes(&key, "objects/a", &data).unwrap();
            assert_eq!(small.len() as u64, sealed_size(n as u64));
            assert_eq!(
                open_all(&key, "objects/a", small).await.unwrap(),
                data,
                "{n}"
            );
        }
    }

    #[tokio::test]
    async fn a_wrong_key_a_change_a_cut_or_a_rename_is_refused() {
        let key = key();
        let data = plain(2 * SEGMENT_BYTES + 10);
        let sealed = seal_all(&key, "objects/a", data).await;

        let other = Arc::new(SealKey::generate().unwrap());
        assert!(matches!(
            open_all(&other, "objects/a", sealed.clone()).await,
            Err(SealError::WrongKey)
        ));
        assert!(matches!(
            open_bytes(&other, "objects/a", &sealed),
            Err(SealError::WrongKey)
        ));

        let mut changed = sealed.clone();
        changed[HEADER_BYTES + 100] ^= 1;
        assert!(matches!(
            open_all(&key, "objects/a", changed).await,
            Err(SealError::Damaged)
        ));

        // Cut at a segment boundary: the new last segment was not sealed as last.
        let cut = sealed[..HEADER_BYTES + SEGMENT_BYTES + TAG_BYTES].to_vec();
        assert!(matches!(
            open_all(&key, "objects/a", cut).await,
            Err(SealError::Damaged)
        ));
        assert!(matches!(
            open_all(&key, "objects/a", sealed[..10].to_vec()).await,
            Err(SealError::Damaged)
        ));

        // The same bytes under another name.
        assert!(matches!(
            open_all(&key, "objects/b", sealed).await,
            Err(SealError::Damaged)
        ));
    }

    #[test]
    fn keys_export_and_name_files() {
        let key = SealKey::generate().unwrap();
        let again = SealKey::parse(&key.export()).unwrap();
        assert_eq!(
            key.object_name("files/x.parquet"),
            again.object_name("files/x.parquet")
        );
        assert_ne!(
            key.object_name("files/x.parquet"),
            key.object_name("files/y.parquet")
        );
        assert_ne!(
            key.object_name("files/x.parquet"),
            SealKey::generate().unwrap().object_name("files/x.parquet")
        );
        assert!(!format!("{key:?}").contains(key.export().as_str()));
        for bad in ["", "not a key", "AAAA"] {
            assert!(SealKey::parse(bad).is_err());
        }
    }
}
