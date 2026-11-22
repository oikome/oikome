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

//! The sandbox, tried with plugins built here from their sources: ones that
//! misbehave (`tests/guests/misbehaving`), and the RAW Preview plugin
//! (`plugins/raw-preview/guest`), whose built component is kept in the
//! repository as `plugins/raw-preview/preview.wasm`. Building them needs the
//! `wasm32-unknown-unknown` target (`rustup target add`).
//! `UPDATE_PLUGINS=1 cargo t --test sandbox` writes the built component.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use oikome_sandbox::{BytesSource, Limits, Origin, PreviewPlugin, Sandbox, SandboxError, MAX_READ};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The guest package in `dir`, built and made a component.
fn build(dir: &Path, features: &[&str]) -> Vec<u8> {
    // A folder per package and feature: each holds one .wasm file.
    let parent = dir
        .parent()
        .and_then(Path::file_name)
        .unwrap()
        .to_string_lossy();
    let name = format!("{parent}-{}", features.first().copied().unwrap_or("plain"));
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("guests")
        .join(name);
    let mut cargo = Command::new(env!("CARGO"));
    // The guest is built as itself, not as this test run is.
    for (key, _) in std::env::vars() {
        if (key.starts_with("CARGO_") && key != "CARGO_HOME") || key == "RUSTFLAGS" {
            cargo.env_remove(key);
        }
    }
    cargo
        .current_dir(dir)
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .arg("--target-dir")
        .arg(&target);
    if !features.is_empty() {
        cargo.args(["--features", &features.join(",")]);
    }
    let out = cargo.output().expect("cargo runs");
    assert!(
        out.status.success(),
        "building {} failed (is the wasm32-unknown-unknown target installed?):\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let built = std::fs::read_dir(target.join("wasm32-unknown-unknown/release"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "wasm"))
        .expect("a .wasm file is built");
    wit_component::ComponentEncoder::default()
        .module(&std::fs::read(built).unwrap())
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

fn misbehaving() -> &'static PreviewPlugin {
    static PLUGIN: OnceLock<PreviewPlugin> = OnceLock::new();
    PLUGIN.get_or_init(|| {
        let wasm = build(
            &repo().join("crates/oikome-sandbox/tests/guests/misbehaving"),
            &[],
        );
        Sandbox::shared().unwrap().preview_plugin(&wasm).unwrap()
    })
}

fn raw_built() -> &'static Vec<u8> {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| build(&repo().join("plugins/raw-preview/guest"), &[]))
}

fn small() -> Limits {
    Limits {
        memory: 64 * 1024 * 1024,
        time: Duration::from_millis(400),
        output: 1024 * 1024,
    }
}

fn run(format: &str, file: Vec<u8>) -> Result<oikome_sandbox::Preview, SandboxError> {
    misbehaving().render(Box::new(BytesSource(file)), format, small())
}

#[test]
fn a_call_reads_the_one_file_it_is_given() {
    let made = run("echo", b"only this file".to_vec()).unwrap();
    assert_eq!(&made.bytes[3..], b"only this file");
    assert_eq!(made.content_type, "image/jpeg");
    assert_eq!(made.origin, Origin::Developed);
    // No more than the most one read gives, whatever is asked for.
    let much = run("read-much", vec![0; MAX_READ as usize + 4096]);
    assert!(matches!(much, Err(SandboxError::Output(_))), "{much:?}");
    let big = Limits {
        output: 64 * 1024 * 1024,
        ..small()
    };
    let read = misbehaving()
        .render(
            Box::new(BytesSource(vec![0; MAX_READ as usize + 4096])),
            "read-much",
            big,
        )
        .unwrap();
    assert_eq!(read.bytes.len(), MAX_READ as usize);
}

#[test]
fn a_plugin_that_runs_on_is_stopped() {
    // Built before the clock starts.
    misbehaving();
    let started = std::time::Instant::now();
    let stopped = run("loop", Vec::new());
    assert!(matches!(stopped, Err(SandboxError::Time(_))), "{stopped:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_plugin_that_takes_too_much_memory_is_stopped() {
    let stopped = run("hog", Vec::new());
    assert!(
        matches!(stopped, Err(SandboxError::Memory(64))),
        "{stopped:?}"
    );
}

#[test]
fn what_a_plugin_gives_back_is_checked() {
    let big = run("big", Vec::new());
    assert!(matches!(big, Err(SandboxError::Output(1))), "{big:?}");
    // The type is read from the bytes, not taken on the plugin's word.
    for format in ["lie", "text"] {
        let wrong = run(format, Vec::new());
        assert!(matches!(wrong, Err(SandboxError::Result(_))), "{wrong:?}");
    }
    let refused = run("unknown", Vec::new());
    assert!(
        matches!(refused, Err(SandboxError::Refused(_))),
        "{refused:?}"
    );
}

#[test]
fn a_plugin_that_fails_takes_nothing_with_it() {
    let failed = run("panic", Vec::new());
    assert!(matches!(failed, Err(SandboxError::Trap(_))), "{failed:?}");
    // Each call has an instance of its own: the next one runs.
    assert!(run("echo", b"after".to_vec()).is_ok());
}

#[test]
fn a_plugin_that_asks_for_more_than_one_file_is_not_loaded() {
    let wasm = build(
        &repo().join("crates/oikome-sandbox/tests/guests/misbehaving"),
        &["reaches-out"],
    );
    let refused = Sandbox::shared().unwrap().preview_plugin(&wasm);
    let Err(SandboxError::Load(why)) = refused else {
        panic!("it was loaded");
    };
    assert!(why.contains("oikome:reaching/files"), "{why}");
    // Nor is anything that is not a component.
    for bytes in [&b"not wasm"[..], &b"\0asm\x01\0\0\0"[..]] {
        let refused = Sandbox::shared().unwrap().preview_plugin(bytes);
        assert!(matches!(refused, Err(SandboxError::Load(_))));
    }
}

// ------------------------------------------------------------ RAW Preview

/// A JPEG's structure: headers a reader walks, `data` bytes of scan, its
/// end. `frame` is its kind (0xC0 baseline, 0xC3 lossless); `inside` goes
/// into an application segment, as a preview's own thumbnail does.
fn jpeg(frame: u8, width: u16, height: u16, data: usize, inside: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    if !inside.is_empty() {
        out.extend([0xFF, 0xE1]);
        out.extend(((inside.len() + 2) as u16).to_be_bytes());
        out.extend(inside);
    }
    out.extend([0xFF, frame, 0x00, 0x0B, 8]);
    out.extend(height.to_be_bytes());
    out.extend(width.to_be_bytes());
    out.extend([1, 1, 0x11, 0]);
    out.extend([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 63, 0]);
    // Scan data with a stuffed byte and a restart marker in it.
    out.extend([0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0]);
    out.extend(std::iter::repeat_n(0x55, data));
    out.extend([0xFF, 0xD9]);
    out
}

/// One directory entry holding a single number.
fn entry(tag: u16, long: bool, value: u32) -> Vec<u8> {
    let mut out = tag.to_le_bytes().to_vec();
    out.extend(if long { 4u16 } else { 3u16 }.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(value.to_le_bytes());
    out
}

fn directory(entries: &[Vec<u8>], next: u32) -> Vec<u8> {
    let mut out = (entries.len() as u16).to_le_bytes().to_vec();
    for e in entries {
        out.extend(e);
    }
    out.extend(next.to_le_bytes());
    out
}

/// A RAW file as cameras write them on TIFF: a small preview in the first
/// directory, the sensor's data as a lossless JPEG in a directory under it,
/// and the full-size preview in another, turned on its side.
fn tiff_raw() -> (Vec<u8>, Vec<u8>) {
    let small = jpeg(0xC0, 160, 120, 64, &[]);
    let sensor = jpeg(0xC3, 6000, 4000, 4096, &[]);
    let full = jpeg(0xC2, 3000, 2000, 512, &jpeg(0xC0, 160, 120, 16, &[]));
    // Header (8), first directory (4 entries: 2 + 48 + 4 = 54), two more
    // (3 entries each: 42), then the images.
    let (first, second, third) = (8u32, 62u32, 104u32);
    let small_at = 146u32;
    let sensor_at = small_at + small.len() as u32;
    let full_at = sensor_at + sensor.len() as u32;
    let mut file = b"II\x2A\x00".to_vec();
    file.extend(first.to_le_bytes());
    file.extend(directory(
        &[
            entry(0x0112, false, 6),
            entry(0x014A, true, second),
            entry(0x0201, true, small_at),
            entry(0x0202, true, small.len() as u32),
        ],
        third,
    ));
    file.extend(directory(
        &[
            entry(0x0103, false, 7),
            entry(0x0111, true, sensor_at),
            entry(0x0117, true, sensor.len() as u32),
        ],
        0,
    ));
    file.extend(directory(
        &[
            entry(0x0103, false, 6),
            entry(0x0111, true, full_at),
            entry(0x0117, true, full.len() as u32),
        ],
        // A directory that points back at the first: a loop to be left.
        first,
    ));
    assert_eq!(file.len() as u32, small_at);
    file.extend(&small);
    file.extend(&sensor);
    file.extend(&full);
    (file, full)
}

fn raw_checks(wasm: &[u8]) {
    let plugin = Sandbox::shared().unwrap().preview_plugin(wasm).unwrap();
    let render =
        |file: Vec<u8>| plugin.render(Box::new(BytesSource(file)), "dng", Limits::default());

    // The largest picture a browser shows, not the sensor's data.
    let (file, full) = tiff_raw();
    let made = render(file).unwrap();
    assert_eq!(made.bytes, full);
    assert_eq!(made.origin, Origin::Embedded);
    assert_eq!(made.orientation, Some(6));
    assert_eq!(made.content_type, "image/jpeg");

    // Kept where no directory says (a maker's notes): found by reading the
    // file through, the preview's own thumbnail not taken for it.
    let inner = jpeg(0xC0, 160, 120, 16, &[]);
    let preview = jpeg(0xC0, 1600, 1200, 300_000, &inner);
    let mut noted = b"IIRO\x08\x00\x00\x00".to_vec();
    noted.extend(directory(&[entry(0x0112, false, 1)], 0));
    noted.extend(vec![0xAB; 70_000]);
    noted.extend(&preview);
    noted.extend(vec![0xCD; 1000]);
    let made = render(noted).unwrap();
    assert_eq!(made.bytes, preview);
    assert_eq!(made.orientation, Some(1));

    // Nothing to show: said, not failed.
    for file in [
        Vec::new(),
        b"II\x2A\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec(),
        jpeg(0xC3, 6000, 4000, 4096, &[]),
        // Directories that point outside the file.
        [&b"II\x2A\x00"[..], &0xFFFF_FFF0u32.to_le_bytes()].concat(),
    ] {
        let none = render(file);
        assert!(matches!(none, Err(SandboxError::Refused(_))), "{none:?}");
    }
}

#[test]
fn raw_preview_takes_out_the_picture_a_camera_stored() {
    raw_checks(raw_built());
}

/// The component in the repository is the one the node ships: it passes
/// what the source built here passes. `UPDATE_PLUGINS=1` writes it.
#[test]
fn the_raw_preview_component_in_the_repository_works() {
    let kept = repo().join("plugins/raw-preview/preview.wasm");
    if std::env::var_os("UPDATE_PLUGINS").is_some() {
        std::fs::write(&kept, raw_built()).unwrap();
    }
    let wasm = std::fs::read(&kept).expect(
        "plugins/raw-preview/preview.wasm; write it with UPDATE_PLUGINS=1 cargo t --test sandbox",
    );
    raw_checks(&wasm);
}
