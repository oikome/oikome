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

//! Public snapshots (architecture §13.4): a detached saved Space becomes a
//! ZIP with a signed manifest and content hashes. Live queries share the same
//! owner configuration; revoking closes both kinds of access.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

use arrow_array::{Array, BinaryArray, StringArray};
use arrow_ipc::reader::StreamReader;
use oikome_identity::Did;
use oikome_semantic::rewrite::quote_ident;
use serde_json::{json, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    Api, ApiError, Caller, Empty, NewPublicShare, PublicManifest, PublicShare, PublicShareRef,
    PublicSnapshot, PublicSnapshotFile, RunSql, SqlResult,
};

const REGISTRY: &str = "public-shares.json";
const SNAPSHOTS: &str = "public";

fn registry_path(root: &std::path::Path) -> PathBuf {
    root.join(REGISTRY)
}

fn snapshot_path(root: &std::path::Path, id: &str) -> PathBuf {
    root.join(SNAPSHOTS).join(format!("{id}.zip"))
}

fn read_registry(root: &std::path::Path) -> Result<Vec<PublicShare>, ApiError> {
    match std::fs::read(registry_path(root)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(ApiError::internal),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(ApiError::internal(e)),
    }
}

fn manifest_payload(manifest: &PublicManifest) -> Value {
    json!({
        "version": manifest.version,
        "owner": manifest.owner,
        "space": manifest.space,
        "title": manifest.title,
        "cover": manifest.cover,
        "createdAtMs": manifest.created_at_ms,
        "removeLocation": manifest.remove_location,
        "files": manifest.files,
    })
}

fn verify_manifest(manifest: &PublicManifest) -> Result<(), ApiError> {
    let signature = hex::decode(&manifest.signature).map_err(ApiError::internal)?;
    let did = Did::parse(&manifest.owner).map_err(ApiError::internal)?;
    let payload = serde_json::to_vec(&manifest_payload(manifest)).map_err(ApiError::internal)?;
    did.verify(&payload, &signature)
        .map_err(|_| ApiError::invalid("the public manifest's signature is not valid"))
}

fn read_u16(bytes: &[u8], at: usize, little: bool) -> Option<u16> {
    let bytes: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
    Some(if little {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}

fn strip_tiff_gps(payload: &[u8]) -> Option<Vec<u8>> {
    let little = match payload.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if read_u16(payload, 2, little)? != 42 {
        return None;
    }
    let first = match little {
        true => u32::from_le_bytes(payload.get(4..8)?.try_into().ok()?),
        false => u32::from_be_bytes(payload.get(4..8)?.try_into().ok()?),
    } as usize;
    let count = read_u16(payload, first, little)? as usize;
    let entries = first.checked_add(2)?;
    let mut kept = Vec::new();
    for i in 0..count {
        let at = entries.checked_add(i.checked_mul(12)?)?;
        let entry = payload.get(at..at + 12)?;
        if read_u16(entry, 0, little)? == 0x8825 {
            kept.extend_from_slice(&[0; 12]);
        } else {
            kept.extend_from_slice(entry);
        }
    }
    let mut out = payload.get(..first)?.to_vec();
    let count = kept.len() as u16 / 12;
    let count_bytes = if little {
        count.to_le_bytes()
    } else {
        count.to_be_bytes()
    };
    out.extend_from_slice(&count_bytes);
    out.extend_from_slice(&kept);
    out.extend_from_slice(payload.get(entries + count as usize * 12..)?);
    Some(out)
}

fn strip_jpeg_location(bytes: &[u8]) -> Vec<u8> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return bytes.to_vec();
    }
    let mut out = vec![0xff, 0xd8];
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            out.extend_from_slice(&bytes[at..]);
            break;
        }
        let marker = bytes[at + 1];
        if marker == 0xd9 || marker == 0xda {
            out.extend_from_slice(&bytes[at..]);
            break;
        }
        let size = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        let Some(end) = at.checked_add(size + 2).filter(|end| *end <= bytes.len()) else {
            return bytes.to_vec();
        };
        let payload = &bytes[at + 4..end];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            if let Some(clean) = strip_tiff_gps(&payload[6..]) {
                out.extend_from_slice(&[0xff, marker]);
                out.extend_from_slice(&((clean.len() + 8) as u16).to_be_bytes());
                out.extend_from_slice(b"Exif\0\0");
                out.extend_from_slice(&clean);
            }
        } else {
            out.extend_from_slice(&bytes[at..end]);
        }
        at = end;
    }
    out
}

fn strip_png_location(bytes: &[u8]) -> Vec<u8> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return bytes.to_vec();
    }
    let mut out = SIGNATURE.to_vec();
    let mut at = SIGNATURE.len();
    while at + 12 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4])) as usize;
        let Some(end) = at.checked_add(size + 12).filter(|end| *end <= bytes.len()) else {
            return bytes.to_vec();
        };
        if &bytes[at + 4..at + 8] != b"eXIf" {
            out.extend_from_slice(&bytes[at..end]);
        }
        at = end;
    }
    out
}

fn sanitize(name: &str, kind: &str, bytes: &[u8], remove_location: bool) -> Vec<u8> {
    if !remove_location || kind != "image" {
        return bytes.to_vec();
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        strip_jpeg_location(bytes)
    } else if lower.ends_with(".png") {
        strip_png_location(bytes)
    } else {
        bytes.to_vec()
    }
}

impl Api {
    fn public_root(&self) -> PathBuf {
        PathBuf::from(self.catalog.oikome_dir())
    }

    fn write_public_registry(&self, shares: &[PublicShare]) -> Result<(), ApiError> {
        let _one = self
            .settings_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = self.public_root();
        std::fs::create_dir_all(&root).map_err(ApiError::internal)?;
        let temporary = root.join(format!("{REGISTRY}.{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(shares).map_err(ApiError::internal)?,
        )
        .map_err(ApiError::internal)?;
        std::fs::rename(&temporary, registry_path(&root)).map_err(ApiError::internal)?;
        Ok(())
    }

    fn public_share(&self, id: &str) -> Result<PublicShare, ApiError> {
        read_registry(&self.public_root())?
            .into_iter()
            .find(|share| share.id == id)
            .ok_or_else(|| ApiError::not_found("this public share does not exist"))
    }

    async fn public_query(&self, sql: String) -> Result<Vec<arrow_array::RecordBatch>, ApiError> {
        let result = self
            .run_sql(&Caller::Owner, RunSql { sql, space: None })
            .await?;
        let SqlResult::Rows(rows) = result else {
            return Err(ApiError::invalid("a saved collection is required"));
        };
        if rows.truncated {
            return Err(ApiError::invalid(
                "this collection is too large to publish in one snapshot",
            ));
        }
        let reader =
            StreamReader::try_new(rows.ipc.as_slice(), None).map_err(ApiError::internal)?;
        let batches = reader
            .collect::<Result<Vec<_>, _>>()
            .map_err(ApiError::internal)?;
        Ok(batches)
    }

    /// Create a signed public snapshot and its live-query configuration.
    pub async fn create_public_share(
        &self,
        caller: &Caller,
        req: NewPublicShare,
    ) -> Result<PublicShare, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let requests_per_minute = req.requests_per_minute.unwrap_or(30);
        if !(1..=120).contains(&requests_per_minute) {
            return Err(ApiError::invalid("requests per minute must be 1 to 120"));
        }
        let max_rows = req.max_rows.unwrap_or(1_000);
        if !(1..=10_000).contains(&max_rows) {
            return Err(ApiError::invalid("rows per query must be 1 to 10,000"));
        }
        let max_ms = req.max_ms.unwrap_or(5_000);
        if !(100..=30_000).contains(&max_ms) {
            return Err(ApiError::invalid("query time must be 100 ms to 30 seconds"));
        }
        let remove_location = req.remove_location.unwrap_or(true);
        let batches = self
            .public_query(format!(
                "SELECT * FROM {}.{}",
                quote_ident(&req.space),
                "_oikome_files"
            ))
            .await?;
        let strings = |index: usize| {
            let column = batches
                .first()
                .map(|batch| batch.column(index).clone())
                .ok_or_else(|| ApiError::invalid("a saved collection is required"))?;
            column
                .as_any()
                .downcast_ref::<StringArray>()
                .cloned()
                .ok_or_else(|| ApiError::invalid("a saved collection is required"))
        };
        let paths = strings(0)?;
        let names = strings(1)?;
        let captions = strings(3)?;
        let taken = strings(4)?;
        let contents = batches
            .first()
            .map(|batch| batch.column(5).clone())
            .ok_or_else(|| ApiError::invalid("a saved collection is required"))?
            .as_any()
            .downcast_ref::<BinaryArray>()
            .cloned()
            .ok_or_else(|| ApiError::invalid("a saved collection is required"))?;
        let kinds = strings(6)?;
        let (title, cover) = {
            let batches = self
                .public_query(format!(
                    "SELECT * FROM {}.{}",
                    quote_ident(&req.space),
                    "_oikome_portfolio"
                ))
                .await?;
            let titles = batches
                .first()
                .and_then(|batch| batch.column(0).as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| ApiError::invalid("a saved collection is required"))?;
            let covers = batches
                .first()
                .and_then(|batch| batch.column(1).as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| ApiError::invalid("a saved collection is required"))?;
            (
                titles.value(0).to_string(),
                covers.is_valid(0).then(|| covers.value(0).to_string()),
            )
        };
        let mut id = [0u8; 32];
        getrandom::fill(&mut id).map_err(|e| ApiError::internal(e.to_string()))?;
        let id = hex::encode(id);
        let created_at_ms = crate::now_ms();
        let owner = self.node_did()?;
        let root = self.public_root();
        std::fs::create_dir_all(root.join(SNAPSHOTS)).map_err(ApiError::internal)?;
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut manifest_files = Vec::new();
        for i in 0..paths.len() {
            let path = paths.value(i).to_string();
            let name = names.value(i).to_string();
            let kind = kinds.value(i).to_string();
            let bytes = sanitize(&name, &kind, contents.value(i), remove_location);
            let hash = blake3::hash(&bytes).to_string();
            zip.start_file(&path, options).map_err(ApiError::internal)?;
            zip.write_all(&bytes).map_err(ApiError::internal)?;
            manifest_files.push(PublicSnapshotFile {
                path,
                name,
                caption: captions.is_valid(i).then(|| captions.value(i).to_string()),
                taken: taken.is_valid(i).then(|| taken.value(i).to_string()),
                kind,
                hash,
                size: bytes.len() as u64,
            });
        }
        let mut manifest = PublicManifest {
            version: 1,
            owner,
            space: req.space.clone(),
            title,
            cover,
            created_at_ms,
            remove_location,
            files: manifest_files,
            signature: String::new(),
        };
        let payload =
            serde_json::to_vec(&manifest_payload(&manifest)).map_err(ApiError::internal)?;
        manifest.signature = hex::encode(self.engines.identity()?.sign(&payload));
        zip.start_file("manifest.json", options)
            .map_err(ApiError::internal)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(ApiError::internal)?)
            .map_err(ApiError::internal)?;
        let bytes = zip.finish().map_err(ApiError::internal)?.into_inner();
        std::fs::write(snapshot_path(&root, &id), &bytes).map_err(ApiError::internal)?;
        let snapshot = PublicSnapshot {
            hash: blake3::hash(&bytes).to_string(),
            size: bytes.len() as u64,
            manifest,
        };
        let share = PublicShare {
            id,
            space: req.space,
            allow_copy: req.allow_copy.unwrap_or(false),
            requests_per_minute,
            max_rows,
            max_ms,
            remove_location,
            active: true,
            created_at_ms,
            snapshot: Some(snapshot),
        };
        let mut shares = read_registry(&root)?;
        shares.push(share.clone());
        self.write_public_registry(&shares)?;
        Ok(share)
    }

    /// The owner's public shares, including revoked ones.
    pub async fn list_public_shares(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<PublicShare>, ApiError> {
        caller.require_owner()?;
        read_registry(&self.public_root())
    }

    /// Revoke a public share. Existing downloaded copies are not claimed to be deleted.
    pub async fn revoke_public_share(
        &self,
        caller: &Caller,
        req: PublicShareRef,
    ) -> Result<PublicShare, ApiError> {
        caller.require_owner()?;
        let mut shares = read_registry(&self.public_root())?;
        let share = shares
            .iter_mut()
            .find(|share| share.id == req.id)
            .ok_or_else(|| ApiError::not_found("this public share does not exist"))?;
        share.active = false;
        let result = share.clone();
        self.write_public_registry(&shares)?;
        Ok(result)
    }

    /// A public share's signed manifest, verified again before it is returned.
    pub async fn public_manifest(&self, req: PublicShareRef) -> Result<PublicManifest, ApiError> {
        let share = self.public_share(&req.id)?;
        if !share.active {
            return Err(ApiError::not_found("this public share has ended"));
        }
        let Some(snapshot) = share.snapshot else {
            return Err(ApiError::not_found("this public share has no snapshot"));
        };
        verify_manifest(&snapshot.manifest)?;
        Ok(snapshot.manifest)
    }

    /// The public snapshot ZIP, with its signed manifest checked first.
    pub async fn public_snapshot(&self, req: PublicShareRef) -> Result<Vec<u8>, ApiError> {
        self.public_manifest(req.clone()).await?;
        let path = snapshot_path(&self.public_root(), &req.id);
        let bytes = std::fs::read(path).map_err(ApiError::internal)?;
        let mut zip = ZipArchive::new(Cursor::new(bytes.clone())).map_err(ApiError::internal)?;
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .map_err(ApiError::internal)?
            .read_to_string(&mut manifest)
            .map_err(ApiError::internal)?;
        let manifest: PublicManifest =
            serde_json::from_str(&manifest).map_err(ApiError::internal)?;
        verify_manifest(&manifest)?;
        for file in &manifest.files {
            let mut bytes = Vec::new();
            zip.by_name(&file.path)
                .map_err(ApiError::internal)?
                .read_to_end(&mut bytes)
                .map_err(ApiError::internal)?;
            if blake3::hash(&bytes).to_string() != file.hash {
                return Err(ApiError::invalid(format!("{} has changed", file.path)));
            }
        }
        Ok(bytes)
    }
}
