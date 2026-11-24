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
//! misbehave (`tests/guests/misbehaving`), and a small well-behaved one
//! (`tests/guests/stored-picture`), whose built component is kept in the
//! repository as `tests/plugins/stored-picture/preview.wasm` for the API's
//! and the app's tests to install. Real preview plugins (RAW Preview) are in
//! the oikome-plugins repository. Building them needs the
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
    let package = dir.file_name().unwrap().to_string_lossy();
    let name = format!("{package}-{}", features.first().copied().unwrap_or("plain"));
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

// ------------------------------------------------- the tests' own plugin

fn stored_picture_built() -> &'static Vec<u8> {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        build(
            &repo().join("crates/oikome-sandbox/tests/guests/stored-picture"),
            &[],
        )
    })
}

/// A file as the tests' plugin reads one: a TIFF header whose directory
/// starts with the orientation, anything, then the picture.
fn holding(picture: &[u8], orientation: u8) -> Vec<u8> {
    let mut file = b"II\x2A\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00".to_vec();
    file.extend([orientation, 0, 0, 0, 0, 0, 0, 0]);
    file.extend(vec![0xAB; 70_000]);
    file.extend(picture);
    file
}

fn stored_picture_checks(wasm: &[u8]) {
    let plugin = Sandbox::shared().unwrap().preview_plugin(wasm).unwrap();
    let render =
        |file: Vec<u8>| plugin.render(Box::new(BytesSource(file)), "dng", Limits::default());
    let mut picture = vec![0xFF, 0xD8, 0xFF, 0xE0];
    picture.extend(vec![0x55; 300_000]);

    let made = render(holding(&picture, 6)).unwrap();
    assert_eq!(made.bytes, picture);
    assert_eq!(made.origin, Origin::Embedded);
    assert_eq!(made.orientation, Some(6));
    assert_eq!(made.content_type, "image/jpeg");
    // No orientation said, or none that is one.
    assert_eq!(render(picture.clone()).unwrap().orientation, None);
    assert_eq!(render(holding(&picture, 9)).unwrap().orientation, None);
    // Nothing to show: said, not failed.
    for file in [Vec::new(), holding(&[], 1)] {
        let none = render(file);
        assert!(matches!(none, Err(SandboxError::Refused(_))), "{none:?}");
    }
}

#[test]
fn a_preview_plugin_takes_out_the_picture_a_file_holds() {
    stored_picture_checks(stored_picture_built());
}

/// The component in the repository is the one the API's and the app's tests
/// install: it passes what the source built here passes. `UPDATE_PLUGINS=1`
/// writes it.
#[test]
fn the_tests_plugin_in_the_repository_works() {
    let kept = repo().join("crates/oikome-sandbox/tests/plugins/stored-picture/preview.wasm");
    if std::env::var_os("UPDATE_PLUGINS").is_some() {
        std::fs::write(&kept, stored_picture_built()).unwrap();
    }
    let wasm = std::fs::read(&kept).expect(
        "tests/plugins/stored-picture/preview.wasm; write it with UPDATE_PLUGINS=1 cargo t --test sandbox",
    );
    stored_picture_checks(&wasm);
}
