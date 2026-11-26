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

//! A development build of `oikome` finds DuckDB's library where
//! libduckdb-sys downloaded it (`<target>/duckdb-download/<triple>/<version>`,
//! with `DUCKDB_DOWNLOAD_LIB`), so `target/debug/oikome` runs as it is,
//! without `DYLD_LIBRARY_PATH` or `LD_LIBRARY_PATH`. libduckdb-sys's own
//! run path reaches its crate only, not the binaries that use it. A release
//! build is packaged with the library beside it and gets none.

use std::path::{Path, PathBuf};

/// `1.5.6` before `1.5.10`: a version's numbers, compared as numbers.
fn version_of(path: &Path) -> Vec<u64> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.split('.').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default()
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DUCKDB_DOWNLOAD_LIB");
    if std::env::var("PROFILE").as_deref() != Ok("debug")
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        return;
    }
    let (Ok(out), Ok(triple)) = (std::env::var("OUT_DIR"), std::env::var("TARGET")) else {
        return;
    };
    let Some(target) = Path::new(&out)
        .ancestors()
        .find(|dir| dir.join("duckdb-download").is_dir())
    else {
        return;
    };
    let downloads = target.join("duckdb-download").join(triple);
    println!("cargo:rerun-if-changed={}", downloads.display());
    // The newest: a version left from before an upgrade is not the one linked.
    let newest: Option<PathBuf> = std::fs::read_dir(&downloads)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .max_by_key(|path| version_of(path));
    if let Some(dir) = newest {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{}", dir.display());
    }
}
