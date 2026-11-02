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

//! Compute sharing (architecture v3 §12): which of Oikome's compute methods
//! this node offers to peers, and each peer's quota. Who may use them is
//! decided by `compute/<method>` UCANs; how much, by the quota.

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};

use crate::catalog::OikomeCatalog;

/// How much of this node's compute one peer may use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeQuota {
    /// The peer's DID.
    pub peer: String,
    /// Tasks it may have running at once.
    pub concurrent: u32,
    /// Tasks it may start per hour.
    pub per_hour: u32,
}

/// One entry of `compute.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ComputeItem {
    Offer { method: String },
    Quota(ComputeQuota),
}

fn check_method(method: &str) -> Result<(), CatalogError> {
    let ok = !method.is_empty()
        && method.len() <= 32
        && method
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CatalogError::InvalidProperty(format!(
            "'{method}' is not a compute method name"
        )))
    }
}

impl OikomeCatalog {
    /// Offer the compute method `method` to peers holding a grant for it.
    pub fn offer_compute(&self, method: &str) -> Result<(), CatalogError> {
        check_method(method)?;
        self.mutate_registry(|reg| {
            reg.compute_offers.insert(method.to_string());
            Ok(())
        })
    }

    /// Stop offering `method`; returns whether it was offered.
    pub fn withdraw_compute(&self, method: &str) -> Result<bool, CatalogError> {
        self.mutate_registry(|reg| Ok(reg.compute_offers.remove(method)))
    }

    /// The compute methods this node offers.
    pub fn offered_compute(&self) -> Vec<String> {
        self.with_registry(|reg| reg.compute_offers.iter().cloned().collect())
    }

    /// Set the quota of `quota.peer`.
    pub fn set_compute_quota(&self, quota: ComputeQuota) -> Result<(), CatalogError> {
        if !quota.peer.starts_with("did:") {
            return Err(CatalogError::InvalidProperty(format!(
                "'{}' is not a DID",
                quota.peer
            )));
        }
        if quota.concurrent == 0 || quota.per_hour == 0 {
            return Err(CatalogError::InvalidProperty(
                "a quota needs at least one task at once and one per hour; revoke the peer's \
                 compute grant to stop it"
                    .to_string(),
            ));
        }
        self.mutate_registry(|reg| {
            reg.compute_quotas.insert(quota.peer.clone(), quota);
            Ok(())
        })
    }

    /// The quota set for `peer`, if any.
    pub fn compute_quota(&self, peer: &str) -> Option<ComputeQuota> {
        self.with_registry(|reg| reg.compute_quotas.get(peer).cloned())
    }

    /// Every quota set, ordered by peer.
    pub fn compute_quotas(&self) -> Vec<ComputeQuota> {
        self.with_registry(|reg| reg.compute_quotas.values().cloned().collect())
    }
}
