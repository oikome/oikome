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

//! Lending this computer (architecture v3 §12, roadmap 3.6) in the owner's
//! API: whether Oikome's own computations are offered, to which contacts,
//! and how much each may use (tasks at once, tasks per hour). A contact
//! lent to gets a compute grant (`compute/sql`, for a year) and an invite
//! code to add it with; one left out has theirs revoked. Lending can be
//! paused and resumed without new codes.
//!
//! Computers lent to this one, accepted from invites, are kept with the
//! node's identity (they hold tokens), and asked whether they lend now.

use std::path::PathBuf;
use std::time::Duration;

use oikome_engine::EngineError;
use oikome_node::compute::{DEFAULT_CONCURRENT, DEFAULT_LIMITS, DEFAULT_PER_HOUR, METHOD_SQL};
use oikome_node::SessionError;
use oikome_semantic::{ComputeLimits, IssueComputeRequest};
use serde::{Deserialize, Serialize};

use crate::invites::{grant_of, invite_code, now_secs, read_private, Lends, What};
use crate::network::lend_key;
use crate::{
    Api, ApiError, BorrowStatus, Borrowed, BorrowedRef, Caller, Empty, LendInvite, Lending,
    LendingSaved, LentTo, SetLending, UseBorrowed,
};

/// How long a grant to use this computer lasts.
const LEND_DAYS: u64 = 365;
const MAX_CONCURRENT: u32 = 8;
const MAX_PER_HOUR: u32 = 1000;
const BORROWED_FILE: &str = "borrowed.json";
/// How long a lender has to say whether it lends now.
const ASK_FOR: Duration = Duration::from_secs(3);

/// A computer lent to this one, as kept.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Held {
    pub(crate) id: String,
    pub(crate) token: String,
    pub(crate) peer: Option<String>,
    /// The lender's DID.
    pub(crate) owner: String,
    /// The name its code gave.
    pub(crate) from: String,
    pub(crate) lends: Option<Lends>,
    pub(crate) accepted_at_ms: u64,
    /// Big summaries of collections shared with the lender go there
    /// (`use_borrowed`); off until the owner turns it on.
    #[serde(default)]
    pub(crate) in_use: bool,
}

/// A grant this node made to use it.
pub(crate) struct Lent {
    pub(crate) id: String,
    pub(crate) did: String,
    pub(crate) expires_at: u64,
}

impl Api {
    /// Grants to use this computer that still work: every capability is a
    /// compute one on this node, and the token is neither revoked nor over.
    pub(crate) fn lent(&self) -> Result<Vec<Lent>, ApiError> {
        let node = format!("node://{}", self.node_did()?);
        let revoked = self.catalog.revoked_ucans();
        let now = now_secs();
        Ok(self
            .catalog
            .list_issued_ucans()
            .into_iter()
            .filter(|u| {
                !u.capabilities.is_empty()
                    && u.capabilities
                        .iter()
                        .all(|c| c.with == node && c.can.starts_with("compute/"))
                    && !revoked.contains(&u.id)
                    && u.expires_at > now
            })
            .map(|u| Lent {
                id: u.id,
                did: u.audience,
                expires_at: u.expires_at,
            })
            .collect())
    }

    /// Whether Oikome's computations are offered (to whoever is lent to).
    pub(crate) fn lending_on(&self) -> bool {
        self.catalog
            .offered_compute()
            .iter()
            .any(|m| m == METHOD_SQL)
    }

    /// Whether this computer is lent, to whom, and how much each may use.
    pub async fn lending(&self, caller: &Caller, _: Empty) -> Result<Lending, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let mut people: Vec<LentTo> = Vec::new();
        for grant in self.lent()? {
            match people.iter_mut().find(|p| p.did == grant.did) {
                Some(person) => person.expires_at = person.expires_at.max(grant.expires_at),
                None => people.push(LentTo {
                    name: self.catalog.get_contact(&grant.did).map(|c| c.name),
                    did: grant.did,
                    expires_at: grant.expires_at,
                }),
            }
        }
        people.sort_by(|a, b| a.name.cmp(&b.name).then(a.did.cmp(&b.did)));
        // Everyone lent to has the same limits; the first one says them.
        let quota = people
            .first()
            .and_then(|p| self.catalog.compute_quota(&p.did));
        let offered = self
            .catalog
            .offered_compute()
            .iter()
            .any(|m| m == METHOD_SQL);
        Ok(Lending {
            on: offered && !people.is_empty(),
            concurrent: quota.as_ref().map_or(DEFAULT_CONCURRENT, |q| q.concurrent),
            per_hour: quota.as_ref().map_or(DEFAULT_PER_HOUR, |q| q.per_hour),
            task_ms: DEFAULT_LIMITS.time_ms.unwrap_or(30_000),
            people,
        })
    }

    /// Lend this computer to these contacts within these limits, or pause
    /// it. Someone newly lent to gets a grant and an invite code (returned
    /// this once); someone left out has theirs revoked.
    pub async fn set_lending(
        &self,
        caller: &Caller,
        req: SetLending,
    ) -> Result<LendingSaved, ApiError> {
        caller.require_owner()?;
        if !(1..=MAX_CONCURRENT).contains(&req.concurrent)
            || !(1..=MAX_PER_HOUR).contains(&req.per_hour)
        {
            return Err(ApiError::invalid(format!(
                "lend 1 to {MAX_CONCURRENT} tasks at once and 1 to {MAX_PER_HOUR} an hour"
            )));
        }
        self.refresh()?;
        let mut people = req.people.clone();
        people.sort();
        people.dedup();
        for did in &people {
            if self.catalog.get_contact(did).is_none() {
                return Err(ApiError::not_found(format!(
                    "{did} is not a contact; add it before lending to it"
                )));
            }
        }
        let session = self.session(None);
        let lent = self.lent()?;
        for grant in &lent {
            if !people.contains(&grant.did) {
                session.revoke_token(&grant.id)?;
            }
        }
        let from = self.settings()?.name;
        let lends = Lends {
            concurrent: req.concurrent,
            per_hour: req.per_hour,
        };
        let mut issued_to = Vec::new();
        for did in &people {
            session.set_compute_quota(did, req.concurrent, req.per_hour)?;
            if lent.iter().any(|g| &g.did == did) {
                continue;
            }
            let issued = session.issue_compute_token(&IssueComputeRequest {
                methods: vec![METHOD_SQL.to_string()],
                audience: did.clone(),
                limits: ComputeLimits {
                    time_ms: None,
                    input_bytes: None,
                    output_bytes: None,
                    memory_bytes: None,
                },
                lifetime_secs: LEND_DAYS * 86_400,
            })?;
            issued_to.push((did.clone(), issued.token));
        }
        if req.on && !people.is_empty() {
            session.offer_compute(METHOD_SQL)?;
        } else {
            session.withdraw_compute(METHOD_SQL)?;
        }
        // Asked only when lending is on: then the people lent to reach it.
        let keys: Vec<String> = people.iter().map(|d| lend_key(d)).collect();
        self.reach_for(req.reach.filter(|_| req.on), &keys).await?;
        let record = self.address_record();
        let invites = issued_to
            .into_iter()
            .map(|(did, token)| LendInvite {
                name: self
                    .catalog
                    .get_contact(&did)
                    .map(|c| c.name)
                    .unwrap_or_default(),
                invite: invite_code(
                    &token,
                    self.flight_url.as_deref(),
                    &from,
                    Some(lends),
                    record.clone(),
                ),
                did,
            })
            .collect();
        Ok(LendingSaved {
            lending: self.lending(caller, Empty {}).await?,
            invites,
        })
    }

    /// Stop lending to `did` (a contact being removed).
    pub(crate) fn stop_lending_to(&self, did: &str) -> Result<(), ApiError> {
        let session = self.session(None);
        for grant in self.lent()? {
            if grant.did == did {
                session.revoke_token(&grant.id)?;
            }
        }
        Ok(())
    }

    fn borrowed_path(&self) -> PathBuf {
        self.catalog
            .oikome_dir()
            .join("identity")
            .join(BORROWED_FILE)
    }

    /// The computer `owner` lends here, if it does.
    pub(crate) fn borrowed_from(&self, owner: &str) -> Option<Held> {
        read_private::<Held>(&self.borrowed_path())
            .ok()?
            .into_iter()
            .find(|h| h.owner == owner)
    }

    /// The names lenders gave with the computers they lend here, by DID.
    pub(crate) fn lender_names(&self) -> Vec<(String, String)> {
        read_private::<Held>(&self.borrowed_path())
            .unwrap_or_default()
            .into_iter()
            .filter(|h| !h.from.trim().is_empty())
            .map(|h| (h.owner, h.from))
            .collect()
    }

    /// The computers lent to this one, as kept.
    pub(crate) fn held(&self) -> Result<Vec<Held>, ApiError> {
        read_private(&self.borrowed_path())
    }

    pub(crate) fn change_borrowed<T>(
        &self,
        change: impl FnOnce(&mut Vec<Held>) -> T,
    ) -> Result<T, ApiError> {
        self.change_private(&self.borrowed_path(), change)
    }

    /// Computers lent to this one, each asked (briefly) whether it lends now.
    pub async fn list_borrowed(
        &self,
        caller: &Caller,
        _: Empty,
    ) -> Result<Vec<Borrowed>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let held = self.held()?;
        let session = self.session(None);
        let now = now_secs();
        let mut out = Vec::new();
        for h in held {
            let grant = grant_of(&h.token);
            let expires_at = grant.as_ref().map_or(0, |g| g.expires_at);
            let task_ms = match grant.map(|g| g.what) {
                Some(What::Compute { task_ms }) => task_ms,
                _ => None,
            };
            let status = if expires_at <= now {
                BorrowStatus::Ended
            } else {
                let asked = tokio::time::timeout(
                    ASK_FOR,
                    session.compute_methods_on_peer(h.peer.as_deref(), &h.owner, &h.token),
                )
                .await;
                match asked {
                    Ok(Ok(methods)) if methods.iter().any(|m| m.method == METHOD_SQL) => {
                        BorrowStatus::Available
                    }
                    Ok(Ok(_))
                    | Ok(Err(SessionError::PermissionDenied(_)))
                    | Ok(Err(SessionError::Engine(EngineError::PermissionDenied(_)))) => {
                        BorrowStatus::Unavailable
                    }
                    _ => BorrowStatus::Unreachable,
                }
            };
            out.push(Borrowed {
                id: h.id,
                from_name: self
                    .catalog
                    .get_contact(&h.owner)
                    .map_or(h.from, |c| c.name),
                from_did: h.owner,
                concurrent: h.lends.map(|l| l.concurrent),
                per_hour: h.lends.map(|l| l.per_hour),
                task_ms,
                expires_at,
                status,
                in_use: h.in_use,
            });
        }
        out.sort_by(|a, b| a.from_name.cmp(&b.from_name));
        Ok(out)
    }

    /// Send big summaries of collections shared with its owner to a
    /// computer lent to this one, or stop.
    pub async fn use_borrowed(
        &self,
        caller: &Caller,
        req: UseBorrowed,
    ) -> Result<Vec<Borrowed>, ApiError> {
        caller.require_owner()?;
        let found = self.change_borrowed(|all| {
            let found = all.iter().any(|h| h.id == req.id);
            for h in all.iter_mut().filter(|h| h.id == req.id) {
                h.in_use = req.in_use;
            }
            found
        })?;
        if !found {
            return Err(ApiError::not_found("no such computer lent to you"));
        }
        self.list_borrowed(caller, Empty {}).await
    }

    /// Stop using a computer lent to this one: its grant is forgotten here.
    pub async fn stop_borrowing(
        &self,
        caller: &Caller,
        req: BorrowedRef,
    ) -> Result<Vec<Borrowed>, ApiError> {
        caller.require_owner()?;
        self.change_borrowed(|all| all.retain(|h| h.id != req.id))?;
        self.list_borrowed(caller, Empty {}).await
    }
}
