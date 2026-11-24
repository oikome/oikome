// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

//! A preview plugin for the tests, as small as one can be: the picture is
//! what the file holds from its first JPEG start to its end. When the file
//! is a little-endian TIFF whose first directory (at byte 8) starts with
//! the orientation, that orientation is given with it.

wit_bindgen::generate!({ world: "preview", path: "../../../wit" });

use oikome::preview::source;
use oikome::preview::types::Origin;

struct Plugin;

impl Guest for Plugin {
    fn render(_format: String) -> Result<Image, String> {
        let size = source::size();
        let mut file = Vec::new();
        while (file.len() as u64) < size {
            let got = source::read(file.len() as u64, u32::MAX)?;
            if got.is_empty() {
                break;
            }
            file.extend(got);
        }
        let start = file
            .windows(3)
            .position(|bytes| bytes == [0xFF, 0xD8, 0xFF])
            .ok_or("this file holds no preview image")?;
        let turned = file.starts_with(b"II\x2A\x00\x08\x00\x00\x00") && file.len() > 18 && file[10..12] == [0x12, 0x01];
        Ok(Image {
            content_type: "image/jpeg".into(),
            bytes: file[start..].to_vec(),
            origin: Origin::Embedded,
            orientation: turned.then(|| file[18]).filter(|o| (1..=8).contains(o)),
        })
    }
}

export!(Plugin);
