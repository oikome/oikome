# Third-party code

## datafusion-federation 0.5.5

- Source: https://crates.io/crates/datafusion-federation/0.5.5
  (https://github.com/datafusion-contrib/datafusion-federation), Apache License 2.0.
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`.
- Change: `[dependencies.datafusion]` in `Cargo.toml` sets
  `default-features = false`. Version 0.5.5 depends on DataFusion with its
  default features, which forces the C-backed `compression` and `parquet`
  features on for every crate and blocks the wasm32 build (architecture v3
  §10). 0.5.7 makes the same change upstream but requires DataFusion 55 /
  Arrow 59, which the pinned DuckDB and Iceberg crates do not support yet.
  No source file is modified.
- Drop this copy once the workspace moves to DataFusion 55 and
  datafusion-federation ≥ 0.5.7.

## opendal-service-gdrive 0.59.5

- Source: https://crates.io/crates/opendal-service-gdrive/0.59.5
  (Apache OpenDAL, https://github.com/apache/opendal), Apache License 2.0.
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`, by
  oikome-vfs for Google Drive (3.4c).
- Change, to be proposed upstream (apache/opendal#8503): in `src/writer.rs`,
  `src/backend.rs` and `src/core.rs`, a file larger than one chunk (8 MiB)
  is written through a resumable upload session, a chunk at a time,
  instead of being held whole in memory and sent in one request. Smaller
  files are sent as before.
- Change, 2026-11-10: in `src/writer.rs`, a new file's path is looked up
  once; the path cache (the file and every folder above it) is refreshed
  only when the upload answers not found, and then the upload is tried once
  more. opendal refreshed it before every new file, so writing one file
  looked every folder of its path up again: four requests more for a file
  three folders deep.
- Drop this copy once an opendal release has the changes.

## opendal-service-dropbox 0.59.5

- Source: https://crates.io/crates/opendal-service-dropbox/0.59.5
  (Apache OpenDAL, https://github.com/apache/opendal), Apache License 2.0.
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`, by
  oikome-vfs for Dropbox (3.4c).
- Change, to be proposed upstream: `src/backend.rs` allows a refresh token
  without a client secret, and `src/core.rs` includes `client_secret` in the
  refresh request only when one was supplied. Dropbox's PKCE flow for public
  clients needs only the App key; the original builder required a secret and
  always sent it.
- Drop this copy once an opendal release has the change.
