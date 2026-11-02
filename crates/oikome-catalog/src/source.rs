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

//! Sources (architecture v3 §11): apps whose data a connector brings into a
//! transactional Space of the same name.
//!
//! The registry keeps a source's connector settings (what `oikome-connect`
//! needs, without credentials) and how often to pull it. Credentials are in
//! the node's secret store; the cursor of the last pull is in the Space
//! itself, committed with the data.

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};

use crate::catalog::OikomeCatalog;
use crate::registry::now_ms;

/// A registered source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDef {
    /// Also the name of the Space the source writes.
    pub name: String,
    /// The connector's settings (`oikome_connect::SourceConfig`).
    pub config: serde_json::Value,
    /// Pull every this many seconds while the node serves; `None` pulls only
    /// on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    pub created_at_ms: u64,
}

/// Check a source name: a letter or `_`, then letters, digits or `_`; it
/// also names the source's Space and environment variables.
pub fn validate_source_name(name: &str) -> Result<(), CatalogError> {
    let mut chars = name.chars();
    let ok = name.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CatalogError::InvalidProperty(format!(
            "source name '{name}': use a letter or '_', then letters, digits or '_'"
        )))
    }
}

impl OikomeCatalog {
    /// Register the source `name`. Fails if the name is taken.
    pub fn add_source(
        &self,
        name: &str,
        config: serde_json::Value,
        every_secs: Option<u64>,
    ) -> Result<SourceDef, CatalogError> {
        validate_source_name(name)?;
        let def = SourceDef {
            name: name.to_string(),
            config,
            every_secs,
            created_at_ms: now_ms(),
        };
        self.mutate_registry(|reg| {
            if reg.sources.contains_key(name) {
                return Err(CatalogError::InvalidProperty(format!(
                    "source '{name}' is already registered"
                )));
            }
            reg.sources.insert(name.to_string(), def.clone());
            Ok(())
        })?;
        Ok(def)
    }

    /// The source `name`.
    pub fn get_source(&self, name: &str) -> Result<SourceDef, CatalogError> {
        self.with_registry(|reg| reg.sources.get(name).cloned())
            .ok_or_else(|| CatalogError::InvalidProperty(format!("no source '{name}'")))
    }

    /// All sources, ordered by name.
    pub fn list_sources(&self) -> Vec<SourceDef> {
        self.with_registry(|reg| reg.sources.values().cloned().collect())
    }

    /// Unregister the source `name`; its Space and data stay.
    pub fn remove_source(&self, name: &str) -> Result<(), CatalogError> {
        self.mutate_registry(|reg| {
            reg.sources
                .remove(name)
                .map(|_| ())
                .ok_or_else(|| CatalogError::InvalidProperty(format!("no source '{name}'")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_are_identifiers() {
        for good in ["flomo", "bank_2026", "_x"] {
            validate_source_name(good).unwrap();
        }
        for bad in ["", "2x", "a-b", "a b", "a.b", &"a".repeat(65)] {
            assert!(validate_source_name(bad).is_err(), "{bad}");
        }
    }
}
