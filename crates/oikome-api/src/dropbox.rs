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

//! Connecting the owner's Dropbox (3.4c): OAuth + PKCE through an app the
//! owner registered in Dropbox's App Console. Dropbox shows an authorization
//! code, which the owner pastes into Oikome; no callback needs registering.
//! The node keeps the verifier and pending sign-in in memory, trades the code
//! for a refresh token, and adds the drive encrypted only after it answers.
//! The app's content access is chosen in the console.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oikome_catalog::StorageKind;
use oikome_node::StorageCredentials;
use oikome_vfs::dropbox::Endpoints;
use oikome_vfs::google::{pkce, random_text};
use oikome_vfs::DropboxClient;
use zeroize::Zeroizing;

use crate::places;
use crate::{
    Api, ApiError, Caller, DropboxApp, DropboxSignIn, Empty, FinishDropbox, StartDropbox,
    StorageSummary,
};

const SIGN_IN_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING: usize = 16;
const DRIVE_LABEL: &str = "Dropbox";

struct Pending {
    label: String,
    client_id: String,
    verifier: Zeroizing<String>,
    make_default: bool,
    read_all: bool,
    expires: Instant,
}

#[derive(Clone, Default)]
pub(crate) struct SignIns(Arc<Mutex<HashMap<String, Pending>>>);

impl SignIns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn insert(&self, id: String, pending: Pending) {
        let mut pending_ones = self.lock();
        let now = Instant::now();
        pending_ones.retain(|_, p| p.expires > now);
        while pending_ones.len() >= MAX_PENDING {
            let oldest = pending_ones
                .iter()
                .min_by_key(|(_, p)| p.expires)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => pending_ones.remove(&id),
                None => break,
            };
        }
        pending_ones.insert(id, pending);
    }

    fn take(&self, id: &str) -> Option<Pending> {
        self.lock()
            .remove(id)
            .filter(|p| p.expires > Instant::now())
    }
}

impl Api {
    fn dropbox_client(&self, client_id: &str) -> DropboxClient {
        DropboxClient::new(client_id).with_endpoints(self.dropbox_endpoints.clone())
    }

    /// Sign in somewhere else than Dropbox's own addresses: for tests.
    pub fn with_dropbox_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.dropbox_endpoints = endpoints;
        self
    }

    /// The owner's Dropbox app this node remembers, by its App key.
    pub async fn dropbox_app(&self, caller: &Caller, _: Empty) -> Result<DropboxApp, ApiError> {
        caller.require_owner()?;
        Ok(DropboxApp {
            client_id: self.session(None).dropbox_app()?,
        })
    }

    /// Start connecting Dropbox. The authorization page shows a code for the
    /// owner to paste into Oikome, so no callback address is needed.
    pub async fn start_dropbox(
        &self,
        caller: &Caller,
        req: StartDropbox,
    ) -> Result<DropboxSignIn, ApiError> {
        caller.require_owner()?;
        let label = match &req.label {
            Some(label) => crate::storage_label(label)?.to_string(),
            None => DRIVE_LABEL.to_string(),
        };
        let incomplete = || {
            ApiError::invalid("Dropbox needs your app's App key")
                .because("dropbox_client_incomplete")
        };
        let given = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
        let client_id = match given(&req.client_id) {
            id if !id.is_empty() => id,
            _ => self.session(None).dropbox_app()?.ok_or_else(incomplete)?,
        };
        let client = self.dropbox_client(&client_id);
        let pkce = pkce().map_err(ApiError::internal)?;
        let id = random_text().map_err(ApiError::internal)?;
        let url = client.sign_in_url(&pkce.challenge);
        self.dropbox_sign_ins.insert(
            id.clone(),
            Pending {
                label,
                client_id,
                verifier: pkce.verifier,
                make_default: req.make_default.unwrap_or(false),
                read_all: req.read_all.unwrap_or(false),
                expires: Instant::now() + SIGN_IN_TTL,
            },
        );
        Ok(DropboxSignIn { url, id })
    }

    /// Finish a sign-in with the authorization code Dropbox showed. Its ID
    /// works once, so an old or reused request cannot add or change a storage.
    pub async fn finish_dropbox(
        &self,
        caller: &Caller,
        req: FinishDropbox,
    ) -> Result<StorageSummary, ApiError> {
        caller.require_owner()?;
        let code = req.code.trim();
        if code.is_empty() {
            return Err(ApiError::invalid("Dropbox needs its authorization code")
                .because("dropbox_code_required"));
        }
        let pending = self.dropbox_sign_ins.take(&req.id).ok_or_else(|| {
            ApiError::invalid("This Dropbox sign-in is unknown, old or already used")
                .because("dropbox_signin_unknown")
        })?;
        self.add_dropbox(caller, pending, code).await
    }

    async fn add_dropbox(
        &self,
        caller: &Caller,
        pending: Pending,
        code: &str,
    ) -> Result<StorageSummary, ApiError> {
        let client = self.dropbox_client(&pending.client_id);
        let signed = client
            .exchange(code, &pending.verifier)
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?;
        self.refresh()?;
        let taken: Vec<String> = self
            .catalog
            .list_storages()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let name = places::storage_name(&pending.label, &taken);
        let credentials = StorageCredentials {
            password: None,
            token: Some(signed.refresh_token),
            key: None,
        };
        let kind = StorageKind::Dropbox {
            client_id: pending.client_id.clone(),
            reads_all: pending.read_all,
        };
        self.session(None)
            .add_storage(&name, kind, true, credentials)
            .await?;
        self.session(None).set_dropbox_app(&pending.client_id)?;
        self.update_settings(|settings| {
            settings
                .storage_labels
                .insert(name.clone(), pending.label.clone());
            if pending.make_default {
                settings.default_storage = Some(name.clone());
            }
        })?;
        self.list_storages(caller, Empty {})
            .await?
            .into_iter()
            .find(|s| s.name.as_deref() == Some(name.as_str()))
            .ok_or_else(|| ApiError::internal("the added storage is missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(expires: Instant) -> Pending {
        Pending {
            label: DRIVE_LABEL.into(),
            client_id: "id".into(),
            verifier: Zeroizing::new("v".into()),
            make_default: false,
            read_all: false,
            expires,
        }
    }

    #[test]
    fn a_sign_in_works_once_and_not_once_old() {
        let sign_ins = SignIns::default();
        let later = Instant::now() + SIGN_IN_TTL;
        sign_ins.insert("a".into(), pending(later));
        assert!(sign_ins.take("b").is_none());
        assert!(sign_ins.take("a").is_some());
        assert!(sign_ins.take("a").is_none());
        sign_ins.insert("old".into(), pending(Instant::now()));
        assert!(sign_ins.take("old").is_none());
    }

    #[test]
    fn many_sign_ins_push_out_the_oldest() {
        let sign_ins = SignIns::default();
        let now = Instant::now();
        for i in 0..MAX_PENDING + 2 {
            sign_ins.insert(
                format!("s{i}"),
                pending(now + SIGN_IN_TTL + Duration::from_secs(i as u64)),
            );
        }
        assert_eq!(sign_ins.lock().len(), MAX_PENDING);
        assert!(sign_ins.take("s0").is_none() && sign_ins.take("s1").is_none());
        assert!(sign_ins.take(&format!("s{}", MAX_PENDING + 1)).is_some());
    }
}
