// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

#[cfg(not(feature = "reaches-out"))]
wit_bindgen::generate!({ world: "preview", path: "../../../wit" });
#[cfg(feature = "reaches-out")]
wit_bindgen::generate!({ world: "oikome:reaching/reaching", path: ["../../../wit", "wit"], generate_all });

use oikome::preview::source;
use oikome::preview::types::Origin;

struct Plugin;

fn jpeg(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len.max(4)];
    bytes[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
    bytes
}

fn image(content_type: &str, bytes: Vec<u8>) -> Result<Image, String> {
    Ok(Image { content_type: content_type.into(), bytes, origin: Origin::Developed, orientation: None })
}

impl Guest for Plugin {
    fn render(format: String) -> Result<Image, String> {
        #[cfg(feature = "reaches-out")]
        let _ = oikome::reaching::files::open("/etc/passwd");
        match format.as_str() {
            // The file as it is, to show what a call can read.
            "echo" => {
                let size = source::size();
                let mut out = jpeg(0)[..3].to_vec();
                out.extend(source::read(0, size as u32)?);
                image("image/jpeg", out)
            }
            "loop" => loop {
                std::hint::black_box(0);
            },
            "hog" => {
                let mut kept = Vec::new();
                loop {
                    kept.push(vec![1u8; 16 * 1024 * 1024]);
                    std::hint::black_box(&kept);
                }
            }
            "big" => image("image/jpeg", jpeg(3 * 1024 * 1024)),
            "lie" => image("image/png", jpeg(16)),
            "text" => image("image/jpeg", b"<html>".to_vec()),
            "read-much" => {
                let got = source::read(0, u32::MAX)?;
                image("image/jpeg", jpeg(got.len()))
            }
            "panic" => panic!("a bug in the plugin"),
            other => Err(format!("no {other} here")),
        }
    }
}

export!(Plugin);
