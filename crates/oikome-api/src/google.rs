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

//! Connecting the owner's Google Drive (3.4c, architecture v3 §7.5),
//! through an app the owner registered with Google themselves: its client
//! ID and secret are entered in the apps with the drive. Oikome carries no
//! app of its own.
//!
//! The owner starts it in the app ([`Api::start_google_drive`]) and is sent
//! to Google's page; Google sends them back to the node's callback address
//! with a code, which the node trades for a refresh token
//! ([`Api::finish_google_drive`]). Google's redirect carries no cookie of
//! the app's, so the way back is let in by its `state` alone: made for the
//! owner's own start, 32 random bytes, good once and for ten minutes, and
//! held in memory only. The PKCE verifier stays here too, so a code is no
//! use to anyone else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oikome_catalog::StorageKind;
use oikome_node::StorageCredentials;
use oikome_vfs::google::{self, Endpoints};
use oikome_vfs::GoogleClient;
use zeroize::Zeroizing;

use crate::places;
use crate::{
    Api, ApiError, Caller, Empty, GoogleApp, GoogleCallback, GoogleOutcome, GoogleReturn,
    GoogleSignIn, StartGoogleDrive,
};

/// How long a sign-in may take.
const SIGN_IN_TTL: Duration = Duration::from_secs(10 * 60);
/// Sign-ins started and not finished, at most: a page that keeps starting
/// them pushes out the oldest.
const MAX_PENDING: usize = 16;
/// What the drive is called when the apps don't say.
const DRIVE_LABEL: &str = "Google Drive";

/// A sign-in the owner started and Google hasn't sent back yet.
struct Pending {
    label: String,
    client_id: String,
    client_secret: Zeroizing<String>,
    verifier: Zeroizing<String>,
    redirect_uri: String,
    make_default: bool,
    return_to: GoogleReturn,
    expires: Instant,
}

/// The sign-ins under way, by their state.
#[derive(Clone, Default)]
pub(crate) struct SignIns(Arc<Mutex<HashMap<String, Pending>>>);

impl SignIns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn insert(&self, state: String, pending: Pending) {
        let mut pending_ones = self.lock();
        let now = Instant::now();
        pending_ones.retain(|_, p| p.expires > now);
        while pending_ones.len() >= MAX_PENDING {
            let oldest = pending_ones
                .iter()
                .min_by_key(|(_, p)| p.expires)
                .map(|(s, _)| s.clone());
            match oldest {
                Some(s) => pending_ones.remove(&s),
                None => break,
            };
        }
        pending_ones.insert(state, pending);
    }

    /// The sign-in `state` names, once: a second use, an old one or one
    /// never started finds nothing.
    fn take(&self, state: &str) -> Option<Pending> {
        self.lock()
            .remove(state)
            .filter(|p| p.expires > Instant::now())
    }
}

impl Api {
    /// The owner's Google app, signing in at Google (or where a test says).
    fn google_client(&self, client_id: &str, client_secret: &str) -> GoogleClient {
        GoogleClient::new(client_id, client_secret).with_endpoints(self.google_endpoints.clone())
    }

    /// Sign in to Google somewhere else than Google's own addresses: for
    /// tests, with a stand-in token endpoint.
    pub fn with_google_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.google_endpoints = endpoints;
        self
    }

    /// The owner's Google app this node remembers, by its client ID.
    pub async fn google_app(&self, caller: &Caller, _: Empty) -> Result<GoogleApp, ApiError> {
        caller.require_owner()?;
        Ok(GoogleApp {
            client_id: self.session(None).google_app()?.map(|(id, _)| id),
        })
    }

    /// Start connecting the owner's Google Drive: the address of Google's
    /// page, which sends the owner back to `redirect_uri` (the node's own
    /// callback address, which the server gives, never the page).
    pub async fn start_google_drive(
        &self,
        caller: &Caller,
        req: StartGoogleDrive,
        redirect_uri: &str,
    ) -> Result<GoogleSignIn, ApiError> {
        caller.require_owner()?;
        let label = match &req.label {
            Some(label) => crate::storage_label(label)?.to_string(),
            None => DRIVE_LABEL.to_string(),
        };
        // The app given, or the one remembered from the last connection.
        let incomplete = || {
            ApiError::invalid("Google Drive needs your Google app's client ID and secret")
                .because("google_client_incomplete")
        };
        let given = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
        let (client_id, client_secret) = match (given(&req.client_id), given(&req.client_secret)) {
            (id, secret) if !id.is_empty() && !secret.is_empty() => (id, Zeroizing::new(secret)),
            (id, secret) if id.is_empty() && secret.is_empty() => {
                self.session(None).google_app()?.ok_or_else(incomplete)?
            }
            _ => return Err(incomplete()),
        };
        let client = self.google_client(&client_id, &client_secret);
        let pkce = google::pkce().map_err(ApiError::internal)?;
        let state = google::random_text().map_err(ApiError::internal)?;
        let url = client.sign_in_url(
            redirect_uri,
            &state,
            &pkce.challenge,
            req.read_all.unwrap_or(false),
        );
        self.sign_ins.insert(
            state,
            Pending {
                label,
                client_id,
                client_secret,
                verifier: pkce.verifier,
                redirect_uri: redirect_uri.to_string(),
                make_default: req.make_default.unwrap_or(false),
                return_to: req.return_to,
                expires: Instant::now() + SIGN_IN_TTL,
            },
        );
        Ok(GoogleSignIn { url })
    }

    /// Finish a sign-in Google sent the owner back from, and say where they
    /// go next and how it went. Its state shows it is the owner's: only
    /// the owner starts one, and each works once. What went wrong goes to
    /// the node's log, without the code or a token.
    pub async fn finish_google_drive(&self, back: GoogleCallback) -> (GoogleReturn, GoogleOutcome) {
        let Some(pending) = back.state.as_deref().and_then(|s| self.sign_ins.take(s)) else {
            tracing::warn!("a Google sign-in came back that is unknown, old or used");
            return (GoogleReturn::Settings, GoogleOutcome::Failed);
        };
        let return_to = pending.return_to;
        let code = match (back.error.as_deref(), back.code.as_deref()) {
            (None, Some(code)) if !code.is_empty() => code,
            (Some("access_denied"), _) => return (return_to, GoogleOutcome::Denied),
            (error, _) => {
                tracing::warn!(error = ?error, "Google sent back no code");
                return (return_to, GoogleOutcome::Failed);
            }
        };
        match self.add_google_drive(pending, code).await {
            Ok(()) => (return_to, GoogleOutcome::Added),
            Err(e) => {
                tracing::warn!(error = %e, "connecting Google Drive failed");
                (return_to, GoogleOutcome::Failed)
            }
        }
    }

    /// Trade `code` for a refresh token, then add the drive, encrypted,
    /// once it answers. If anything fails, nothing is kept.
    async fn add_google_drive(&self, pending: Pending, code: &str) -> Result<(), ApiError> {
        let client = self.google_client(&pending.client_id, &pending.client_secret);
        let signed = client
            .exchange(code, &pending.verifier, &pending.redirect_uri)
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
        // The app's secret is kept like a password, the sign-in as a token.
        let credentials = StorageCredentials {
            password: Some(pending.client_secret.clone()),
            token: Some(signed.refresh_token),
            key: None,
        };
        // A token that didn't work is forgotten with the storage, not
        // revoked: Google would revoke the owner's other computers' too.
        let kind = StorageKind::Gdrive {
            client_id: pending.client_id.clone(),
            reads_all: signed.reads_all,
        };
        self.session(None)
            .add_storage(&name, kind, true, credentials)
            .await?;
        // It worked: the next Google Drive asks for nothing.
        self.session(None)
            .set_google_app(&pending.client_id, &pending.client_secret)?;
        self.update_settings(|settings| {
            settings
                .storage_labels
                .insert(name.clone(), pending.label.clone());
            if pending.make_default {
                settings.default_storage = Some(name.clone());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(expires: Instant) -> Pending {
        Pending {
            label: DRIVE_LABEL.into(),
            client_id: "id".into(),
            client_secret: Zeroizing::new("secret".into()),
            verifier: Zeroizing::new("v".into()),
            redirect_uri: "http://127.0.0.1:1/oauth/google/callback".into(),
            make_default: false,
            return_to: GoogleReturn::Settings,
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

    #[test]
    fn the_way_back_is_one_of_the_apps_pages() {
        assert_eq!(
            GoogleOutcome::Added.page(GoogleReturn::Settings),
            "/settings?google=added"
        );
        assert_eq!(
            GoogleOutcome::Denied.page(GoogleReturn::Onboarding),
            "/onboarding?google=denied"
        );
    }
}
