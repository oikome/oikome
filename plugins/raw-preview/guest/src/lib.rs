// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

//! RAW Preview: the JPEG a camera stored inside its RAW file, taken out as
//! it is. Nothing is developed from the sensor's data, and no RAW decoder
//! is used.
//!
//! RAW files built on TIFF (CR2, NEF, ARW, DNG, ORF, PEF and their kin) keep
//! their previews in image directories, which say where each one is. When
//! the directories name none (some keep it in the maker's own notes), the
//! file is searched for JPEG images. The largest one that a browser can
//! show is given back: a lossless JPEG, which is how some RAW files hold the
//! sensor's data itself, is not a picture and is passed over.

wit_bindgen::generate!({ world: "preview", path: "../../../crates/oikome-sandbox/wit" });

use oikome::preview::source;
use oikome::preview::types::Origin;

/// The largest preview given back.
const MAX_PREVIEW: u64 = 48 * 1024 * 1024;
/// How much of the file one read asks for.
const BLOCK: u64 = 256 * 1024;
/// Directories read at most: a file that loops is left.
const MAX_DIRECTORIES: usize = 64;
/// JPEG images looked at when the file is searched.
const MAX_FOUND: usize = 48;

/// The file, read through the host a block at a time.
struct File {
    size: u64,
    at: u64,
    block: Vec<u8>,
}

impl File {
    fn open() -> File {
        File { size: source::size(), at: 0, block: Vec::new() }
    }

    /// `len` bytes from `offset`, or `None` past the end.
    fn bytes(&mut self, offset: u64, len: usize) -> Result<Option<&[u8]>, String> {
        let end = match offset.checked_add(len as u64) {
            Some(end) if end <= self.size => end,
            _ => return Ok(None),
        };
        if offset < self.at || end > self.at + self.block.len() as u64 {
            let want = (len as u64).max(BLOCK).min(self.size - offset);
            self.block = read_all(offset, want)?;
            self.at = offset;
            if (self.block.len() as u64) < len as u64 {
                return Ok(None);
            }
        }
        let start = (offset - self.at) as usize;
        Ok(Some(&self.block[start..start + len]))
    }
}

/// `len` bytes from `offset`, asked for in as many reads as the host needs.
fn read_all(offset: u64, len: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(len as usize);
    while (out.len() as u64) < len {
        let rest = len - out.len() as u64;
        let got = source::read(offset + out.len() as u64, rest.min(u64::from(u32::MAX)) as u32)?;
        if got.is_empty() {
            break;
        }
        out.extend(got);
    }
    Ok(out)
}

/// A JPEG image inside the file.
#[derive(Clone, Copy)]
struct Found {
    offset: u64,
    len: u64,
    width: u32,
    height: u32,
}

/// What a JPEG's headers say: its size in pixels, whether a browser shows
/// it (baseline, extended or progressive), and where its headers end.
struct Frame {
    width: u32,
    height: u32,
    shown: bool,
}

/// The frame of the JPEG at `offset`, if one starts there.
fn frame_at(file: &mut File, offset: u64) -> Result<Option<Frame>, String> {
    match file.bytes(offset, 3)? {
        Some([0xFF, 0xD8, 0xFF]) => {}
        _ => return Ok(None),
    }
    let mut at = offset + 2;
    // Segments before the frame: a few at most.
    for _ in 0..64 {
        let Some(head) = file.bytes(at, 4)? else { return Ok(None) };
        if head[0] != 0xFF {
            return Ok(None);
        }
        let marker = head[1];
        let len = u64::from(u16::from_be_bytes([head[2], head[3]]));
        match marker {
            // Fill bytes before a marker.
            0xFF => {
                at += 1;
                continue;
            }
            0xC0..=0xCF if marker != 0xC4 && marker != 0xC8 && marker != 0xCC => {
                let Some(body) = file.bytes(at + 4, 5)? else { return Ok(None) };
                let height = u32::from(u16::from_be_bytes([body[1], body[2]]));
                let width = u32::from(u16::from_be_bytes([body[3], body[4]]));
                return Ok(Some(Frame { width, height, shown: matches!(marker, 0xC0..=0xC2) }));
            }
            // The image's data, or its end, before any frame: not a picture.
            0xDA | 0xD9 => return Ok(None),
            _ if len < 2 => return Ok(None),
            _ => at += 2 + len,
        }
    }
    Ok(None)
}

/// Where the JPEG at `offset` ends, reading no further than `limit`.
fn end_of(file: &mut File, offset: u64, limit: u64) -> Result<Option<u64>, String> {
    let mut at = offset + 2;
    while at < limit {
        let Some(head) = file.bytes(at, 2)? else { return Ok(None) };
        let (first, marker) = (head[0], head[1]);
        if first != 0xFF {
            return Ok(None);
        }
        match marker {
            0xFF => at += 1,
            0xD9 => return Ok(Some(at + 2)),
            0x01 | 0xD0..=0xD7 => at += 2,
            _ => {
                let Some(len) = file.bytes(at + 2, 2)? else { return Ok(None) };
                let len = u64::from(u16::from_be_bytes([len[0], len[1]]));
                if len < 2 {
                    return Ok(None);
                }
                at += 2 + len;
                if marker == 0xDA {
                    // Compressed data: up to the next marker that is not
                    // a stuffed byte or a restart.
                    at = match next_marker(file, at, limit)? {
                        Some(next) => next,
                        None => return Ok(None),
                    };
                }
            }
        }
    }
    Ok(None)
}

/// The first marker from `from` that ends a scan's data.
fn next_marker(file: &mut File, from: u64, limit: u64) -> Result<Option<u64>, String> {
    let mut at = from;
    while at < limit {
        let len = BLOCK.min(file.size - at).min(limit - at) as usize;
        if len < 2 {
            return Ok(None);
        }
        let Some(block) = file.bytes(at, len)? else { return Ok(None) };
        let mut i = 0;
        while i + 1 < block.len() {
            if block[i] == 0xFF && !matches!(block[i + 1], 0x00 | 0xD0..=0xD7 | 0xFF) {
                return Ok(Some(at + i as u64));
            }
            i += 1;
        }
        // The last byte may start a marker: it is read again.
        at += (block.len() - 1) as u64;
    }
    Ok(None)
}

/// A TIFF file's byte order.
#[derive(Clone, Copy)]
struct Order {
    little: bool,
}

impl Order {
    fn u16(self, b: &[u8]) -> u16 {
        if self.little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }
    }

    fn u32(self, b: &[u8]) -> u32 {
        let b = [b[0], b[1], b[2], b[3]];
        if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
    }
}

/// One entry of an image directory: its numbers, up to sixteen.
fn values(file: &mut File, order: Order, entry: &[u8]) -> Result<Vec<u32>, String> {
    let kind = order.u16(&entry[2..4]);
    let count = order.u32(&entry[4..8]) as usize;
    let width = match kind {
        1 | 7 => 1,
        3 => 2,
        4 | 13 => 4,
        _ => return Ok(Vec::new()),
    };
    let count = count.min(16);
    let held;
    let data: &[u8] = if count * width <= 4 {
        &entry[8..12]
    } else {
        let at = u64::from(order.u32(&entry[8..12]));
        match file.bytes(at, count * width)? {
            Some(bytes) => {
                held = bytes.to_vec();
                &held
            }
            None => return Ok(Vec::new()),
        }
    };
    Ok((0..count)
        .map(|i| match width {
            1 => u32::from(data[i]),
            2 => u32::from(order.u16(&data[i * 2..])),
            _ => order.u32(&data[i * 4..]),
        })
        .collect())
}

/// The JPEG images a TIFF file's directories name, and the orientation its
/// first directory gives.
fn from_directories(file: &mut File) -> Result<(Vec<(u64, u64)>, Option<u8>), String> {
    let Some(head) = file.bytes(0, 8)? else { return Ok((Vec::new(), None)) };
    let order = match &head[..2] {
        b"II" => Order { little: true },
        b"MM" => Order { little: false },
        _ => return Ok((Vec::new(), None)),
    };
    let first = u64::from(order.u32(&head[4..8]));
    let mut todo = vec![first];
    let mut seen: Vec<u64> = Vec::new();
    let mut named = Vec::new();
    let mut orientation = None;
    while let Some(at) = todo.pop() {
        if at == 0 || seen.contains(&at) || seen.len() >= MAX_DIRECTORIES {
            continue;
        }
        seen.push(at);
        let Some(count) = file.bytes(at, 2)? else { continue };
        let count = usize::from(order.u16(count)).min(512);
        let Some(entries) = file.bytes(at + 2, count * 12 + 4)?.map(<[u8]>::to_vec) else {
            continue;
        };
        let (mut jpeg_at, mut jpeg_len) = (None, None);
        let (mut strip_at, mut strip_len, mut compression) = (None, None, None);
        for entry in entries[..count * 12].chunks_exact(12) {
            let tag = order.u16(&entry[..2]);
            match tag {
                0x0103 => compression = values(file, order, entry)?.first().copied(),
                0x0111 | 0x0117 => {
                    let all = values(file, order, entry)?;
                    // A preview is one strip; the sensor's data is many.
                    let one = (order.u32(&entry[4..8]) == 1).then(|| all.first().copied()).flatten();
                    if tag == 0x0111 { strip_at = one } else { strip_len = one }
                }
                0x0112 if seen.len() == 1 => {
                    orientation = values(file, order, entry)?.first().map(|v| *v as u8);
                }
                0x0201 => jpeg_at = values(file, order, entry)?.first().copied(),
                0x0202 => jpeg_len = values(file, order, entry)?.first().copied(),
                // Panasonic's JpgFromRaw: the image is the entry's own data.
                0x002E if order.u32(&entry[4..8]) > 4 => {
                    named.push((u64::from(order.u32(&entry[8..12])), u64::from(order.u32(&entry[4..8]))));
                }
                // Directories under this one, and the EXIF directory.
                0x014A | 0x8769 => {
                    todo.extend(values(file, order, entry)?.into_iter().map(u64::from));
                }
                _ => {}
            }
        }
        if let (Some(at), Some(len)) = (jpeg_at, jpeg_len) {
            named.push((u64::from(at), u64::from(len)));
        }
        if let (Some(at), Some(len), Some(6 | 7)) = (strip_at, strip_len, compression) {
            named.push((u64::from(at), u64::from(len)));
        }
        todo.push(u64::from(order.u32(&entries[count * 12..])));
    }
    Ok((named, orientation))
}

/// The JPEG images found by reading the file through.
fn by_searching(file: &mut File) -> Result<Vec<(u64, u64)>, String> {
    let mut found = Vec::new();
    let mut at = 0u64;
    while at + 3 <= file.size && found.len() < MAX_FOUND {
        let len = BLOCK.min(file.size - at) as usize;
        let Some(block) = file.bytes(at, len)? else { break };
        let hit = block.windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]);
        let Some(hit) = hit else {
            if len < 3 {
                break;
            }
            at += (len - 2) as u64;
            continue;
        };
        let start = at + hit as u64;
        let limit = file.size.min(start + MAX_PREVIEW);
        match end_of(file, start, limit)? {
            // The images inside it (its own thumbnail) are passed with it.
            Some(end) => {
                found.push((start, end - start));
                at = end;
            }
            None => at = start + 3,
        }
    }
    Ok(found)
}

struct Plugin;

impl Guest for Plugin {
    fn render(_format: String) -> Result<Image, String> {
        let mut file = File::open();
        let (mut named, orientation) = from_directories(&mut file)?;
        if named.is_empty() {
            named = by_searching(&mut file)?;
        }
        let mut best: Option<Found> = None;
        for (offset, len) in named {
            if len < 4 || len > MAX_PREVIEW || offset.checked_add(len).is_none_or(|end| end > file.size) {
                continue;
            }
            let Some(frame) = frame_at(&mut file, offset)? else { continue };
            if !frame.shown {
                continue;
            }
            let found = Found { offset, len, width: frame.width, height: frame.height };
            let area = |f: &Found| (u64::from(f.width) * u64::from(f.height), f.len);
            if best.as_ref().is_none_or(|b| area(&found) > area(b)) {
                best = Some(found);
            }
        }
        let best = best.ok_or("this file holds no preview image")?;
        let bytes = read_all(best.offset, best.len)?;
        if bytes.len() as u64 != best.len {
            return Err("the preview image is cut short".into());
        }
        Ok(Image {
            content_type: "image/jpeg".into(),
            bytes,
            origin: Origin::Embedded,
            orientation: orientation.filter(|o| (1..=8).contains(o)),
        })
    }
}

export!(Plugin);
