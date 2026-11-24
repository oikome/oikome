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

//! Previews made by plugins (PR 32): for a file of a format Oikome does not
//! show itself (a camera's RAW file), an installed plugin's code makes an
//! image of it. The code runs in the sandbox (`oikome-sandbox`), given that
//! one file to read and nothing else, within limits of memory and time. The
//! file is only read. What was made is kept in `.oikome/previews` until the
//! file or the plugin changes, with a thumbnail of it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use oikome_node::inplace::{self, Place};
use oikome_sandbox::{
    BytesSource, FileSource, Limits, Origin, PreviewPlugin, Sandbox, SandboxError, Source,
};

use crate::in_place::Original;
use crate::{Api, ApiError, Caller, FileRef};

/// The previews kept take up to this much; the oldest go first.
const MAX_KEPT_BYTES: u64 = 1 << 30;

/// The plugin that makes previews of a format.
pub(crate) struct Previewer {
    pub plugin: String,
    pub version: String,
    /// Its code, in the plugin's folder.
    pub wasm: PathBuf,
}

/// A plugin's code, compiled: checked as the sandbox checks it. Kept by
/// what the code is, so each is compiled once.
pub(crate) fn load(wasm: &[u8]) -> Result<PreviewPlugin, SandboxError> {
    static LOADED: OnceLock<Mutex<HashMap<[u8; 32], PreviewPlugin>>> = OnceLock::new();
    let key = *blake3::hash(wasm).as_bytes();
    let loaded = LOADED.get_or_init(Default::default);
    if let Some(plugin) = loaded.lock().ok().and_then(|l| l.get(&key).cloned()) {
        return Ok(plugin);
    }
    let plugin = Sandbox::shared()?.preview_plugin(wasm)?;
    if let Ok(mut loaded) = loaded.lock() {
        loaded.insert(key, plugin.clone());
    }
    Ok(plugin)
}

/// A file on a NAS, a drive or object storage, read a range at a time.
struct PlaceSource {
    place: Place,
    path: String,
    size: u64,
    runtime: tokio::runtime::Handle,
}

impl Source for PlaceSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        if offset >= self.size {
            return Ok(Vec::new());
        }
        let end = offset.saturating_add(u64::from(len)).min(self.size);
        self.runtime
            .block_on(self.place.read(&self.path, offset..end, end - offset))
            .map_err(|e| e.to_string())
    }
}

/// A JPEG that says nothing of how to turn it, with `orientation` (EXIF, 2
/// to 8) put in: an EXIF segment of that one entry, after its start.
fn turned(jpeg: Vec<u8>, orientation: Option<u8>) -> Vec<u8> {
    let Some(orientation) = orientation.filter(|o| (2..=8).contains(o)) else {
        return jpeg;
    };
    if !jpeg.starts_with(&[0xFF, 0xD8]) || has_exif(&jpeg) {
        return jpeg;
    }
    let mut exif = b"Exif\0\0II\x2A\x00\x08\x00\x00\x00\x01\x00".to_vec();
    exif.extend([0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00]);
    exif.extend([orientation, 0, 0, 0]);
    exif.extend([0, 0, 0, 0]);
    let mut out = Vec::with_capacity(jpeg.len() + exif.len() + 4);
    out.extend([0xFF, 0xD8, 0xFF, 0xE1]);
    out.extend(((exif.len() + 2) as u16).to_be_bytes());
    out.extend(exif);
    out.extend(&jpeg[2..]);
    out
}

/// Whether a JPEG has an EXIF segment before its picture.
fn has_exif(jpeg: &[u8]) -> bool {
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if matches!(marker, 0xDA | 0xD9) || len < 2 {
            return false;
        }
        if marker == 0xE1 && jpeg[at + 4..].starts_with(b"Exif\0\0") {
            return true;
        }
        at += 2 + len;
    }
    false
}

pub(crate) fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn type_of(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"RIFF") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

impl Api {
    fn previews_dir(&self) -> PathBuf {
        self.catalog.oikome_dir().join("previews")
    }

    /// Remove the oldest previews kept until they fit again.
    fn trim_previews(&self) {
        let Ok(entries) = std::fs::read_dir(self.previews_dir()) else {
            return;
        };
        let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                Some((meta.modified().ok()?, meta.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = kept.iter().map(|(_, len, _)| len).sum();
        kept.sort();
        for (_, len, path) in kept {
            if total <= MAX_KEPT_BYTES {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= len;
            }
        }
    }

    /// The image an installed plugin makes of a file Oikome does not show
    /// itself, and a thumbnail of it: made once, then kept until the file or
    /// the plugin changes. `None` when no installed plugin previews it.
    async fn plugin_preview(
        &self,
        req: &FileRef,
    ) -> Result<Option<(Vec<u8>, Option<Vec<u8>>)>, ApiError> {
        let saved = self.saved_original(&req.space, &req.path).await?;
        let (name, size, modified_ms, location) = match &saved {
            Some((entry, _)) => (entry.name.clone(), entry.size, entry.modified_ms, None),
            None => {
                let location = self.in_place_of(&req.space)?;
                let file =
                    inplace::file(&self.catalog, &location, &req.path)?.ok_or_else(|| {
                        ApiError::not_found(format!("{} has no file {}", req.space, req.path))
                    })?;
                (file.name, file.size, file.modified_ms, Some(location))
            }
        };
        let format = extension_of(&name);
        let Some(previewer) = self.previewer_of(&format)? else {
            return Ok(None);
        };
        let wasm = std::fs::read(&previewer.wasm).map_err(ApiError::internal)?;
        // Kept by the file as it is now and the code that made it.
        let mut key = blake3::Hasher::new();
        for part in [
            req.space.as_bytes(),
            req.path.as_bytes(),
            &size.to_le_bytes(),
            &modified_ms.to_le_bytes(),
            previewer.plugin.as_bytes(),
            previewer.version.as_bytes(),
            blake3::hash(&wasm).as_bytes(),
        ] {
            key.update(&(part.len() as u64).to_le_bytes());
            key.update(part);
        }
        let key = key.finalize().to_hex().to_string();
        let dir = self.previews_dir();
        let (kept, kept_thumb) = (dir.join(&key), dir.join(format!("{key}.thumb")));
        if let Ok(bytes) = std::fs::read(&kept) {
            return Ok(Some((bytes, std::fs::read(&kept_thumb).ok())));
        }

        let source: Box<dyn Source> = match (saved, location) {
            (Some((_, bytes)), _) => Box::new(BytesSource(bytes)),
            (None, Some(location)) => {
                let place = inplace::place_of(&self.catalog, &self.engines, &location)?;
                let (on, _) = place.route(&req.path);
                if !on.reachable().await {
                    return Err(ApiError::from(inplace::offline(&on.label())));
                }
                match place.local_file(&req.path) {
                    Some(local) => Box::new(FileSource::open(&local).map_err(ApiError::internal)?),
                    None => Box::new(PlaceSource {
                        place,
                        path: req.path.clone(),
                        size,
                        runtime: tokio::runtime::Handle::current(),
                    }),
                }
            }
            (None, None) => return Ok(None),
        };
        let made = tokio::task::spawn_blocking(move || {
            let plugin = load(&wasm)?;
            plugin.render(source, &format, Limits::default())
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| {
            tracing::info!(plugin = %previewer.plugin, file = %req.path, error = %e, "no preview made");
            ApiError::invalid(format!("no preview of {name}: {e}")).because("preview_failed")
        })?;
        let bytes = match (made.content_type, made.origin) {
            ("image/jpeg", Origin::Embedded | Origin::Developed) => {
                turned(made.bytes, made.orientation)
            }
            _ => made.bytes,
        };
        let thumb = {
            let bytes = bytes.clone();
            tokio::task::spawn_blocking(move || inplace::thumbnail(&bytes).map(|(jpeg, _, _)| jpeg))
                .await
                .map_err(ApiError::internal)?
        };
        // Kept for the next time; a failure to keep it only costs that.
        if std::fs::create_dir_all(&dir).is_ok() {
            let tmp = dir.join(format!("{key}.tmp"));
            if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &kept).is_ok() {
                if let Some(thumb) = &thumb {
                    let _ = std::fs::write(&kept_thumb, thumb);
                }
                self.trim_previews();
            }
        }
        Ok(Some((bytes, thumb)))
    }

    /// The image an installed plugin makes of a file (a RAW file's stored
    /// picture), whole.
    pub async fn file_preview(&self, caller: &Caller, req: FileRef) -> Result<Original, ApiError> {
        caller.require_owner()?;
        let (bytes, _) = self
            .plugin_preview(&req)
            .await?
            .ok_or_else(|| ApiError::not_found("no installed plugin previews this file"))?;
        Ok(Original {
            start: 0,
            total: bytes.len() as u64,
            content_type: type_of(&bytes),
            bytes,
        })
    }

    /// The thumbnail of the image a plugin makes of a file, if one does.
    pub(crate) async fn preview_thumbnail(
        &self,
        req: &FileRef,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        Ok(self.plugin_preview(req).await?.and_then(|(_, thumb)| thumb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_is_turned_only_when_it_does_not_say_itself() {
        let plain = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x02, 0xFF, 0xD9];
        assert_eq!(turned(plain.clone(), None), plain);
        assert_eq!(turned(plain.clone(), Some(1)), plain);
        let side = turned(plain.clone(), Some(6));
        assert!(has_exif(&side));
        assert_eq!(&side[side.len() - 6..], &plain[2..]);
        // It says so itself: left as it is.
        assert_eq!(turned(side.clone(), Some(3)), side);
        let decoder = image::ImageReader::new(std::io::Cursor::new(&side))
            .with_guessed_format()
            .unwrap();
        assert_eq!(decoder.format(), Some(image::ImageFormat::Jpeg));
    }
}
