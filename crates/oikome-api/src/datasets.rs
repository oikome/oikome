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

//! Datasets in a collection's folders (PR 31): the DuckDB extensions that
//! read them. Reading them is in `file_analysis` (their tables) and the
//! node's `datasets` module. A dataset is a folder of the collection like
//! any other: it is copied as files are, never through a path of its own.

use oikome_engine::Capabilities;
use oikome_node::datasets::DatasetKind as NodeKind;

use crate::{Api, ApiError, Caller, DatasetKind, ExtensionDownload, ExtensionRef};

pub(crate) fn kind_of(kind: NodeKind) -> DatasetKind {
    match kind {
        NodeKind::Lance => DatasetKind::Lance,
        NodeKind::Iceberg => DatasetKind::Iceberg,
    }
}

/// The capability that loads the extension reading `kind`.
pub(crate) fn capability_of(kind: NodeKind) -> Capabilities {
    match kind {
        NodeKind::Lance => Capabilities::READ_LANCE,
        NodeKind::Iceberg => Capabilities::READ_ICEBERG,
    }
}

impl Api {
    /// Where an extension's download is, for `name` (`lance`, `iceberg`).
    pub(crate) fn extension_download(&self, name: &str) -> ExtensionDownload {
        crate::knowledge::extension_state(&self.session(None), name)
    }

    /// Start downloading a DuckDB extension Oikome reads datasets with,
    /// from DuckDB's repository, and installing it; DuckDB checks its
    /// signature whenever it loads it. Owner only.
    pub async fn install_extension(
        &self,
        caller: &Caller,
        req: ExtensionRef,
    ) -> Result<ExtensionDownload, ApiError> {
        caller.require_owner()?;
        let Some(kind) = NodeKind::from_name(&req.name) else {
            return Err(ApiError::invalid(format!(
                "{} is not an extension Oikome downloads",
                req.name
            )));
        };
        let session = self.session(None);
        session.install_extension_support(kind.extension())?;
        Ok(self.extension_download(kind.extension()))
    }
}
