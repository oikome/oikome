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

//! Being reached from outside this computer's network (roadmap 3.12), in
//! the owner's API.
//!
//! Nothing is opened unless the owner allowed it. Sharing a collection or
//! lending this computer asks first (the web app's prompt: Deny / Allow,
//! and "Don't ask again"):
//!
//! * an answer without "Don't ask again" is for that one share or lending:
//!   allowed, it is kept as `share:<id>` or `lend:<did>` and ends with it
//!   (the share revoked or over, the person no longer lent to);
//! * with it, the answer becomes the rule (always or never), which
//!   Settings › Network can change back to asking.
//!
//! The router is asked to open the port while anything allowed is still
//! going, and the port closes when nothing is. Whether contacts may meet
//! here is the owner's too (on unless turned off); the peer-to-peer layer
//! only offers it while this computer is reached directly.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use oikome_network::{Reach, ReachWanted};

use crate::invites::sender_name;
use crate::{
    Api, ApiError, Caller, Empty, Network, NodeSettings, OpenFor, OpenForKind, ReachChoice,
    ReachPolicy, Reachable, SetNetwork, ShareStatus,
};

/// How long a share or lending waits for the router's answer before its
/// invite is made, so the invite carries the address it opened.
const SETTLE_FOR: Duration = Duration::from_secs(8);

pub(crate) fn share_key(id: &str) -> String {
    format!("share:{id}")
}

pub(crate) fn lend_key(did: &str) -> String {
    format!("lend:{did}")
}

/// Whether the share or lending `key` may be reached from outside.
pub(crate) fn allowed(settings: &NodeSettings, key: &str) -> bool {
    match settings.reach_policy {
        ReachPolicy::Always => true,
        ReachPolicy::Never => false,
        ReachPolicy::Ask => settings.reach_for.iter().any(|k| k == key),
    }
}

/// Keep the owner's answer for the shares or lending `keys`.
pub(crate) fn apply_choice(settings: &mut NodeSettings, choice: ReachChoice, keys: &[String]) {
    if choice.remember == Some(true) {
        settings.reach_policy = if choice.allow {
            ReachPolicy::Always
        } else {
            settings.reach_for.clear();
            ReachPolicy::Never
        };
        return;
    }
    for key in keys {
        let at = settings.reach_for.iter().position(|k| k == key);
        match (choice.allow, at) {
            (true, None) => settings.reach_for.push(key.clone()),
            (false, Some(at)) => {
                settings.reach_for.remove(at);
            }
            _ => {}
        }
    }
}

fn reachable(reach: &Reach) -> Reachable {
    match reach {
        Reach::LocalOnly => Reachable::LocalOnly,
        Reach::Opening => Reachable::Opening,
        Reach::Direct => Reachable::Direct,
        Reach::ThroughContact(_) => Reachable::ThroughContact,
        Reach::RouterRefused => Reachable::RouterRefused,
        Reach::SharedAddress => Reachable::SharedAddress,
    }
}

impl Api {
    /// Shares and lending going on now that the owner allowed to be reached
    /// from outside; and the keys of everything going on, allowed or not.
    fn reach_in_use(
        &self,
        settings: &NodeSettings,
    ) -> Result<(Vec<OpenFor>, HashSet<String>), ApiError> {
        let mut open = Vec::new();
        let mut live = HashSet::new();
        for share in self.shares()? {
            if share.status != ShareStatus::Active {
                continue;
            }
            let key = share_key(&share.id);
            live.insert(key.clone());
            if allowed(settings, &key) {
                open.push(OpenFor {
                    kind: OpenForKind::Share,
                    name: share
                        .contact_name
                        .clone()
                        .unwrap_or_else(|| short(&share.did)),
                    id: share.id,
                    space: Some(share.space),
                });
            }
        }
        if self.lending_on() {
            let mut seen = HashSet::new();
            for grant in self.lent()? {
                if !seen.insert(grant.did.clone()) {
                    continue;
                }
                let key = lend_key(&grant.did);
                live.insert(key.clone());
                if allowed(settings, &key) {
                    open.push(OpenFor {
                        kind: OpenForKind::Lend,
                        name: self
                            .catalog
                            .get_contact(&grant.did)
                            .map(|c| c.name)
                            .unwrap_or_else(|| short(&grant.did)),
                        id: grant.did,
                        space: None,
                    });
                }
            }
        }
        Ok((open, live))
    }

    /// What the owner's choices ask of the network now. Answers kept for a
    /// share or lending that ended are let go.
    pub(crate) fn reach_wanted(&self) -> Result<ReachWanted, ApiError> {
        let settings = self.settings()?;
        let (open, live) = self.reach_in_use(&settings)?;
        if settings.reach_for.iter().any(|k| !live.contains(k)) {
            self.update_settings(|s| s.reach_for.retain(|k| live.contains(k)))?;
        }
        Ok(ReachWanted {
            open: !open.is_empty(),
            help_contacts: !settings.help_contacts_off,
        })
    }

    /// Tell the peer-to-peer layer what the owner's choices ask of it now:
    /// after a share or lending changed, and once in a while (a share runs
    /// out on its own).
    pub fn update_reach(&self) {
        match self.reach_wanted() {
            Ok(wanted) => {
                if let Some(network) = &self.network {
                    network.set_reach(wanted);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not work out whether to be reached from outside")
            }
        }
    }

    /// Keep the owner's answer for `keys`, then act on it. When it opens
    /// the port, wait a moment for the router, so an invite made next
    /// carries the address it opened.
    pub(crate) async fn reach_for(
        &self,
        choice: Option<ReachChoice>,
        keys: &[String],
    ) -> Result<(), ApiError> {
        if let Some(choice) = choice {
            self.update_settings(|s| apply_choice(s, choice, keys))?;
        }
        self.update_reach();
        let Some(network) = &self.network else {
            return Ok(());
        };
        let wanted = self.reach_wanted()?;
        if !wanted.open {
            return Ok(());
        }
        let until = Instant::now() + SETTLE_FOR;
        while Instant::now() < until {
            let status = network.status();
            if status.wanted.open && status.reach != Reach::Opening {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Where this data center is reached, signed, for an invite.
    pub(crate) fn address_record(&self) -> Option<String> {
        self.network.as_ref()?.address_record()
    }

    /// Keep where `did` is reached, from the record its invite carried.
    pub(crate) fn learn_record(&self, did: &str, record: Option<&str>) {
        let (Some(network), Some(record)) = (&self.network, record) else {
            return;
        };
        let Ok(did) = oikome_identity::Did::parse(did) else {
            return;
        };
        if let Err(e) = network.learn_record(&did, record) {
            tracing::info!(%did, error = %e, "an invite's address record was not kept");
        }
    }

    /// How this computer is reached from outside, what for, and what the
    /// owner chose.
    pub async fn network(&self, caller: &Caller, _: Empty) -> Result<Network, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let settings = self.settings()?;
        let (open_for, _) = self.reach_in_use(&settings)?;
        let status = self.network.as_ref().map(|n| n.status());
        let through = match status.as_ref().map(|s| &s.reach) {
            Some(Reach::ThroughContact(did)) => Some(
                self.catalog
                    .get_contact(did.as_str())
                    .map(|c| c.name)
                    .unwrap_or_else(|| short(did.as_str())),
            ),
            _ => None,
        };
        Ok(Network {
            reachable: status
                .as_ref()
                .map_or(Reachable::LocalOnly, |s| reachable(&s.reach)),
            through,
            open_for,
            policy: settings.reach_policy,
            help_contacts: !settings.help_contacts_off,
            helping: status.as_ref().is_some_and(|s| s.helping),
            available: self.network.is_some(),
        })
    }

    /// Change what Oikome does when it needs to be reached from outside,
    /// or whether contacts may meet here.
    pub async fn set_network(&self, caller: &Caller, req: SetNetwork) -> Result<Network, ApiError> {
        caller.require_owner()?;
        self.update_settings(|s| {
            if let Some(policy) = req.policy {
                s.reach_policy = policy;
            }
            if let Some(help) = req.help_contacts {
                s.help_contacts_off = !help;
            }
        })?;
        self.update_reach();
        self.network(caller, Empty {}).await
    }
}

/// The start and end of a DID, for someone who isn't a contact.
fn short(did: &str) -> String {
    match oikome_identity::Did::parse(did) {
        Ok(did) => sender_name("", &did),
        Err(_) => did.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask() -> NodeSettings {
        NodeSettings::default()
    }

    #[test]
    fn an_answer_without_dont_ask_again_is_for_that_share_alone() {
        let mut s = ask();
        let one = share_key("u1");
        let two = share_key("u2");
        apply_choice(
            &mut s,
            ReachChoice {
                allow: true,
                remember: None,
            },
            std::slice::from_ref(&one),
        );
        assert!(allowed(&s, &one));
        assert!(!allowed(&s, &two));
        assert_eq!(s.reach_policy, ReachPolicy::Ask);
        // Denied for another share: that one stays on this network only.
        apply_choice(
            &mut s,
            ReachChoice {
                allow: false,
                remember: Some(false),
            },
            std::slice::from_ref(&two),
        );
        assert!(allowed(&s, &one) && !allowed(&s, &two));
    }

    #[test]
    fn dont_ask_again_makes_the_answer_the_rule() {
        let mut s = ask();
        let one = lend_key("did:key:z6Mkone");
        apply_choice(
            &mut s,
            ReachChoice {
                allow: true,
                remember: Some(true),
            },
            std::slice::from_ref(&one),
        );
        assert_eq!(s.reach_policy, ReachPolicy::Always);
        assert!(allowed(&s, &share_key("any")));

        apply_choice(
            &mut s,
            ReachChoice {
                allow: false,
                remember: Some(true),
            },
            &[],
        );
        assert_eq!(s.reach_policy, ReachPolicy::Never);
        assert!(!allowed(&s, &one));
        assert!(s.reach_for.is_empty());
    }
}
