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

pub mod file_read;
pub mod file_write;
pub mod remote;
pub mod seal;
pub mod storage;

/// Before the first opendal operator: TLS with ring, the provider the rest
/// of the node uses, and opendal's HTTP transport over it (opendal's
/// default features would bring aws-lc instead).
pub(crate) fn ready() {
    static READY: std::sync::Once = std::sync::Once::new();
    READY.call_once(|| {
        // Already installed by the node's own TLS, or by an earlier call.
        let _ = rustls::crypto::ring::default_provider().install_default();
        opendal_http_transport_reqwest::install_default();
    });
}

pub use file_read::OpenDalFileRead;
pub use file_write::OpenDalFileWrite;
pub use remote::{
    check_owner_path, check_relative_path, file_chunks, Bucket, Credentials, OwnerEntry,
    RemoteStore, StoreEntry, StoreError, S3_REGION,
};
pub use seal::{sealed_size, SealError, SealKey};
pub use storage::{OpenDalStorage, OpenDalStorageConfig, OpenDalStorageFactory};
