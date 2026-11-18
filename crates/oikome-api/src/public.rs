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

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow_array::{Array, BinaryArray, StringArray};
use arrow_ipc::reader::StreamReader;
use chrono::{SecondsFormat, Utc};
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use oikome_identity::Did;
use oikome_node::{AccessEntry, Outcome, QueryResult, SessionConfig};
use oikome_semantic::{rewrite::quote_ident, split_statements};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{
    encode_rows, Api, ApiError, Caller, Empty, NewPublicShare, PublicAuthRequest,
    PublicAuthVerification, PublicChallenge, PublicFileRef, PublicManifest, PublicQuery,
    PublicSession, PublicShare, PublicShareRef, PublicSnapshot, PublicSnapshotFile, Rows, RunSql,
    SqlResult,
};

const REGISTRY: &str = "public-shares.json";
const SNAPSHOTS: &str = "public";
const CHALLENGE_LIFE: Duration = Duration::from_secs(5 * 60);
const SESSION_LIFE: Duration = Duration::from_secs(60 * 60);
const MAX_PUBLIC_SQL: usize = 64 * 1024;
const PUBLIC_METHOD: &str = "public_query";
const PUBLIC_FILE_COLUMNS: [&str; 7] = [
    "path", "name", "position", "caption", "taken", "kind", "hash",
];

#[derive(Clone)]
struct Challenge {
    id: String,
    did: String,
    message: String,
    expires: Instant,
}

#[derive(Clone)]
struct Session {
    id: String,
    did: String,
    principal: String,
    expires: Instant,
}

type PublicCalls = Arc<Mutex<HashMap<(String, String), Vec<Instant>>>>;

#[derive(Clone, Default)]
pub(crate) struct LiveState {
    challenges: Arc<Mutex<HashMap<String, Challenge>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    calls: PublicCalls,
}

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
        "allowCopy": manifest.allow_copy,
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

fn visitor_did(text: &str) -> Result<(String, Option<u64>), ApiError> {
    if text.starts_with("did:key:") {
        let did = Did::parse(text)
            .map_err(|_| ApiError::invalid("a valid did:key or Ethereum did:pkh is required"))?;
        return Ok((did.to_string(), None));
    }
    let Some(rest) = text.strip_prefix("did:pkh:eip155:") else {
        return Err(ApiError::invalid(
            "a valid did:key or Ethereum did:pkh is required",
        ));
    };
    let Some((chain, address)) = rest.split_once(':') else {
        return Err(ApiError::invalid(
            "an Ethereum DID names its chain and address",
        ));
    };
    let chain = chain
        .parse::<u64>()
        .ok()
        .filter(|chain| *chain > 0)
        .ok_or_else(|| ApiError::invalid("the Ethereum chain id is not valid"))?;
    if address.len() != 42
        || !address.starts_with("0x")
        || !address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::invalid("the Ethereum address is not valid"));
    }
    Ok((
        format!("did:pkh:eip155:{chain}:{}", address.to_ascii_lowercase()),
        Some(chain),
    ))
}

fn public_principal(id: &str, did: &str) -> String {
    format!("public:{id}:{}", blake3::hash(did.as_bytes()))
}

fn signature_bytes(signature: &str) -> Result<Vec<u8>, ApiError> {
    hex::decode(signature.strip_prefix("0x").unwrap_or(signature))
        .map_err(|_| ApiError::invalid("the signature is not hexadecimal"))
}

fn verify_visitor(did: &str, message: &str, signature: &str) -> Result<(), ApiError> {
    let signature = signature_bytes(signature)?;
    if did.starts_with("did:key:") {
        return Did::parse(did)
            .and_then(|did| did.verify(message.as_bytes(), &signature))
            .map_err(|_| ApiError::permission_denied("the signature is not valid"));
    }
    if signature.len() != 65 {
        return Err(ApiError::invalid("an Ethereum signature is 65 bytes"));
    }
    let recovery = match signature[64] {
        27 | 28 => signature[64] - 27,
        value => value,
    };
    let recovery = RecoveryId::from_byte(recovery)
        .ok_or_else(|| ApiError::invalid("the Ethereum recovery id is not valid"))?;
    let signature = Signature::try_from(&signature[..64])
        .map_err(|_| ApiError::invalid("the Ethereum signature is not valid"))?;
    let prefixed = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hash = Keccak256::new();
    hash.update(prefixed.as_bytes());
    hash.update(message.as_bytes());
    let hash = hash.finalize();
    let key = VerifyingKey::recover_from_prehash(&hash, &signature, recovery)
        .map_err(|_| ApiError::permission_denied("the signature is not valid"))?;
    let encoded = key.to_encoded_point(false);
    let address = Keccak256::digest(&encoded.as_bytes()[1..]);
    let expected = did
        .rsplit_once(":0x")
        .map(|(_, address)| address)
        .ok_or_else(|| ApiError::invalid("the Ethereum address is not valid"))?;
    let expected = hex::decode(expected).map_err(ApiError::internal)?;
    if address[12..] != expected {
        return Err(ApiError::permission_denied(
            "the signature belongs to another Ethereum address",
        ));
    }
    Ok(())
}

fn random_hex() -> Result<String, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(hex::encode(bytes))
}

impl Api {
    fn public_root(&self) -> PathBuf {
        self.catalog.oikome_dir()
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

    /// A share visitors may still reach: not revoked, not past its end.
    fn live(share: &PublicShare) -> bool {
        share.active && share.expires_at_ms.is_none_or(|end| crate::now_ms() < end)
    }

    fn public_share(&self, id: &str) -> Result<PublicShare, ApiError> {
        read_registry(&self.public_root())?
            .into_iter()
            .find(|share| share.id == id)
            .ok_or_else(|| ApiError::not_found("this public share does not exist"))
    }

    async fn grant_public_visitor(
        &self,
        share: &PublicShare,
        principal: &str,
    ) -> Result<(), ApiError> {
        let owner = self.session(None);
        if share.allow_copy {
            return owner
                .grant_space(&share.space, &[principal.to_string()])
                .map_err(ApiError::from);
        }
        let columns = PUBLIC_FILE_COLUMNS
            .iter()
            .map(|column| quote_ident(column))
            .collect::<Vec<_>>()
            .join(", ");
        for (table, columns) in [
            ("_oikome_files", columns.as_str()),
            ("_oikome_portfolio", "title, cover"),
        ] {
            owner
                .sql(&format!(
                    "GRANT SELECT ({columns}) ON {}.{} TO {}",
                    quote_ident(&share.space),
                    quote_ident(table),
                    quote_ident(principal)
                ))
                .await?;
        }
        Ok(())
    }

    async fn revoke_public_visitor(&self, share: &PublicShare, did: &str) -> Result<(), ApiError> {
        let owner = self.session(None);
        let principal = public_principal(&share.id, did);
        if share.allow_copy {
            owner.revoke_space(&share.space, &[principal])?;
            return Ok(());
        }
        for table in ["_oikome_files", "_oikome_portfolio"] {
            owner
                .sql(&format!(
                    "REVOKE SELECT ON {}.{} FROM {}",
                    quote_ident(&share.space),
                    quote_ident(table),
                    quote_ident(&principal)
                ))
                .await?;
        }
        Ok(())
    }

    fn log_public(
        &self,
        did: Option<String>,
        share: &PublicShare,
        outcome: Outcome,
        reason: Option<String>,
    ) {
        if let Some(log) = &self.access_log {
            log.record(AccessEntry {
                at_ms: crate::now_ms(),
                caller: did,
                method: PUBLIC_METHOD.into(),
                spaces: vec![share.space.clone()],
                token_id: Some(share.id.clone()),
                outcome,
                reason,
                detail: None,
                merge: false,
            });
        }
    }

    async fn snapshot_rows(&self, sql: String) -> Result<Vec<arrow_array::RecordBatch>, ApiError> {
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
        let allow_copy = req.allow_copy.unwrap_or(false);
        if req.expires_at_ms.is_some_and(|end| end <= crate::now_ms()) {
            return Err(ApiError::invalid("a public share must end in the future"));
        }
        let batches = self
            .snapshot_rows(format!(
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
                .snapshot_rows(format!(
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
            allow_copy,
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
            allow_copy,
            requests_per_minute,
            max_rows,
            max_ms,
            remove_location,
            active: true,
            created_at_ms,
            expires_at_ms: req.expires_at_ms,
            snapshot: Some(snapshot),
            visitors: Vec::new(),
        };
        let mut shares = read_registry(&root)?;
        shares.push(share.clone());
        self.write_public_registry(&shares)?;
        self.reach_for(req.reach, &[crate::network::public_key(&share.id)])
            .await?;
        Ok(share)
    }

    pub(crate) fn active_public_shares(&self) -> Result<Vec<PublicShare>, ApiError> {
        Ok(read_registry(&self.public_root())?
            .into_iter()
            .filter(|share| share.active)
            .collect())
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
            .iter()
            .find(|share| share.id == req.id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("this public share does not exist"))?;
        for did in &share.visitors {
            self.revoke_public_visitor(&share, did).await?;
        }
        if let Some(stored) = shares.iter_mut().find(|stored| stored.id == req.id) {
            stored.active = false;
        }
        self.write_public_registry(&shares)?;
        self.public_live
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, session| session.id != req.id);
        self.public_live
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(id, _), _| id != &req.id);
        self.update_reach();
        Ok(PublicShare {
            active: false,
            ..share
        })
    }

    /// A public share's signed manifest, verified again before it is returned.
    pub async fn public_manifest(&self, req: PublicShareRef) -> Result<PublicManifest, ApiError> {
        let share = self.public_share(&req.id)?;
        if !Self::live(&share) {
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
        if !self.public_share(&req.id)?.allow_copy {
            return Err(ApiError::permission_denied(
                "this public share does not allow copying",
            ));
        }
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

    /// A file of a live public snapshot, checked against its signed manifest.
    async fn snapshot_file(
        &self,
        req: &PublicFileRef,
    ) -> Result<(PublicSnapshotFile, Vec<u8>), ApiError> {
        let manifest = self
            .public_manifest(PublicShareRef { id: req.id.clone() })
            .await?;
        let file = manifest
            .files
            .into_iter()
            .find(|file| file.path == req.path)
            .ok_or_else(|| ApiError::not_found("this public file does not exist"))?;
        let bytes = std::fs::read(snapshot_path(&self.public_root(), &req.id))
            .map_err(ApiError::internal)?;
        let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(ApiError::internal)?;
        let mut content = Vec::new();
        zip.by_name(&file.path)
            .map_err(ApiError::internal)?
            .read_to_end(&mut content)
            .map_err(ApiError::internal)?;
        if blake3::hash(&content).to_string() != file.hash {
            return Err(ApiError::invalid("the public file has changed"));
        }
        Ok((file, content))
    }

    /// One sanitized original from a copy-enabled public snapshot.
    pub async fn public_file(&self, req: PublicFileRef) -> Result<Vec<u8>, ApiError> {
        if !self.public_share(&req.id)?.allow_copy {
            return Err(ApiError::permission_denied(
                "this public share does not allow copying",
            ));
        }
        Ok(self.snapshot_file(&req).await?.1)
    }

    /// A photo's preview, at most 512 pixels on its longest edge, re-encoded
    /// as JPEG, so no metadata of the original goes with it. Every live
    /// share has them, copying allowed or not: a visitor sees the pictures,
    /// never the original files.
    pub async fn public_preview(&self, req: PublicFileRef) -> Result<Vec<u8>, ApiError> {
        let (file, content) = self.snapshot_file(&req).await?;
        if file.kind != "image" {
            return Err(ApiError::not_found("this public file has no preview"));
        }
        let preview =
            tokio::task::spawn_blocking(move || oikome_node::inplace::thumbnail(&content))
                .await
                .map_err(ApiError::internal)?;
        preview
            .map(|(jpeg, _, _)| jpeg)
            .ok_or_else(|| ApiError::not_found("this public file has no preview"))
    }

    /// Issue a one-time challenge for a visitor's DID.
    pub async fn public_auth_challenge(
        &self,
        req: PublicAuthRequest,
    ) -> Result<PublicChallenge, ApiError> {
        let share = self.public_share(&req.id)?;
        if !Self::live(&share) {
            return Err(ApiError::not_found("this public share has ended"));
        }
        let (did, chain) = visitor_did(&req.did)?;
        let nonce = random_hex()?;
        let issued = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let message = if let Some(chain) = chain {
            let address = did
                .rsplit_once(':')
                .map(|(_, address)| address)
                .unwrap_or("");
            format!(
                "oikome wants you to sign in with your Ethereum account:\n{address}\n\n\
                 Sign in to read this Oikome public share.\n\n\
                 URI: oikome://public/{}\nVersion: 1\nChain ID: {chain}\n\
                 Nonce: {nonce}\nIssued At: {issued}",
                share.id
            )
        } else {
            format!(
                "Oikome public share\nID: {}\nNonce: {nonce}\nIssued At: {issued}",
                share.id
            )
        };
        let expires = Instant::now() + CHALLENGE_LIFE;
        let expires_at_ms = crate::now_ms() + CHALLENGE_LIFE.as_millis() as u64;
        let mut challenges = self
            .public_live
            .challenges
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        challenges.retain(|_, challenge| challenge.expires > Instant::now());
        challenges.insert(
            nonce.clone(),
            Challenge {
                id: share.id,
                did,
                message: message.clone(),
                expires,
            },
        );
        Ok(PublicChallenge {
            message,
            nonce,
            expires_at_ms,
        })
    }

    /// Verify a wallet or Ed25519 signature and open a short-lived session.
    pub async fn public_auth_verify(
        &self,
        req: PublicAuthVerification,
    ) -> Result<PublicSession, ApiError> {
        let (did, _) = visitor_did(&req.did)?;
        let challenge = {
            let mut challenges = self
                .public_live
                .challenges
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            challenges.retain(|_, challenge| challenge.expires > Instant::now());
            let nonce = challenges
                .iter()
                .find(|(_, challenge)| {
                    challenge.id == req.id
                        && challenge.did == did
                        && challenge.message == req.message
                })
                .map(|(nonce, _)| nonce.clone());
            nonce.and_then(|nonce| challenges.remove(&nonce))
        }
        .ok_or_else(|| ApiError::permission_denied("this sign-in challenge is not valid"))?;
        verify_visitor(&challenge.did, &challenge.message, &req.signature)?;
        let share = self.public_share(&req.id)?;
        if !Self::live(&share) {
            return Err(ApiError::not_found("this public share has ended"));
        }
        let principal = public_principal(&share.id, &did);
        self.grant_public_visitor(&share, &principal).await?;
        let mut shares = read_registry(&self.public_root())?;
        let stored = shares
            .iter_mut()
            .find(|stored| stored.id == share.id && stored.active)
            .ok_or_else(|| ApiError::not_found("this public share has ended"))?;
        if !stored.visitors.contains(&did) {
            stored.visitors.push(did.clone());
        }
        self.write_public_registry(&shares)?;
        let token = random_hex()?;
        let expires = Instant::now() + SESSION_LIFE;
        let expires_at_ms = crate::now_ms() + SESSION_LIFE.as_millis() as u64;
        self.public_live
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                blake3::hash(token.as_bytes()).to_string(),
                Session {
                    id: share.id,
                    did: did.clone(),
                    principal,
                    expires,
                },
            );
        Ok(PublicSession {
            token,
            did,
            expires_at_ms,
        })
    }

    /// Query only what this public share grants, within its per-visitor limits.
    pub async fn public_query(&self, req: PublicQuery) -> Result<Rows, ApiError> {
        let share = self.public_share(&req.id).ok();
        let session = {
            let mut sessions = self
                .public_live
                .sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            sessions.retain(|_, session| session.expires > Instant::now());
            sessions
                .get(&blake3::hash(req.token.as_bytes()).to_string())
                .cloned()
        };
        let Some(session) = session.filter(|session| session.id == req.id) else {
            if let Some(share) = &share {
                self.log_public(
                    None,
                    share,
                    Outcome::Denied,
                    Some("visitor authentication required".into()),
                );
            }
            return Err(ApiError::new(
                crate::ErrorCode::Unauthenticated,
                "sign in to this public share first",
            ));
        };
        let Some(share) = share.filter(Self::live) else {
            return Err(ApiError::not_found("this public share has ended"));
        };
        let now = Instant::now();
        let within_limit = {
            let mut calls = self
                .public_live
                .calls
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let calls = calls
                .entry((share.id.clone(), session.did.clone()))
                .or_default();
            calls.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
            calls.push(now);
            calls.len() <= share.requests_per_minute as usize
        };
        let result = if !within_limit {
            Err(ApiError::permission_denied(
                "this visitor's request limit has been reached",
            ))
        } else {
            self.run_public_query(&share, &session, &req.sql).await
        };
        self.log_public(
            Some(session.did),
            &share,
            if result.is_ok() {
                Outcome::Allowed
            } else {
                Outcome::Denied
            },
            result.as_ref().err().map(|error| error.message.clone()),
        );
        result
    }

    async fn run_public_query(
        &self,
        share: &PublicShare,
        session: &Session,
        sql: &str,
    ) -> Result<Rows, ApiError> {
        if sql.len() > MAX_PUBLIC_SQL {
            return Err(ApiError::invalid("a public query is limited to 64 KiB"));
        }
        let statements = split_statements(sql);
        if statements.len() != 1 {
            return Err(ApiError::invalid("send one read-only SQL statement"));
        }
        self.refresh()?;
        let session = oikome_node::OikomeSession::new(
            self.catalog.clone(),
            self.engines.clone(),
            SessionConfig {
                default_space: Some(share.space.clone()),
                principal: Some(session.principal.clone()),
            },
        );
        let result = tokio::time::timeout(
            Duration::from_millis(share.max_ms),
            session.sql(&statements[0]),
        )
        .await
        .map_err(|_| ApiError::permission_denied("the public query time limit was reached"))??;
        let QueryResult::Batches { schema, batches } = result else {
            return Err(ApiError::invalid("a public query must return rows"));
        };
        let rows = encode_rows(
            &schema,
            &batches,
            share.max_rows as usize,
            crate::MAX_RESULT_BYTES,
        )?;
        if rows.truncated {
            return Err(ApiError::permission_denied(
                "the public query row or result-size limit was reached",
            ));
        }
        Ok(rows)
    }
}
