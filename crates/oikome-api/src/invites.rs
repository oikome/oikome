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

//! Invites (architecture v3 §13.4): a share, or a lent computer, sent to
//! another data center as one code. The code holds the grant's token, where
//! the sending data center is reached (its Flight address when it listens
//! on one, and the peer-to-peer addresses it signed, 3.12), and what it is
//! called (and, for a computer, how much it lends). The receiver adds it, sees who invited them
//! to what, and accepts (a collection is added, queried where it is; a
//! computer is kept to borrow; the sender becomes a contact) or declines.
//! Invites waiting here hold tokens, so they are kept with the node's
//! identity, readable by the owner only.

use std::collections::HashSet;
use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use oikome_catalog::LocationFormat;
use oikome_core::{validate_space_name, CreateSpaceRequest, RemoteMode};
use oikome_identity::ucan::{self, token_id, ABILITY_QUERY, ABILITY_READ};
use oikome_identity::{Capability, Did, Ucan, VerifyOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::lending::Held;
use crate::{
    AcceptInvite, AcceptedInvite, Api, ApiError, Caller, Empty, ErrorCode, Invite, InviteCode,
    InviteKind, InviteRef, ShareAccess,
};

/// Every invite code starts so; the version follows the letters.
const PREFIX: &str = "oik1-";
/// A code is a token and a little more; anything longer is not one.
const MAX_CODE_BYTES: usize = 16 * 1024;
/// As many as a contact's name may have.
const MAX_FROM_CHARS: usize = oikome_catalog::contact::MAX_CONTACT_NAME_CHARS;
/// Invites waiting at once, so a pasted flood can't fill the disk.
const MAX_WAITING: usize = 100;
const INVITES_FILE: &str = "invites.json";

/// How much a lent computer lends: tasks at once and per hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lends {
    pub(crate) concurrent: u32,
    pub(crate) per_hour: u32,
}

/// What a code carries.
#[derive(Serialize, Deserialize)]
struct Wire {
    v: u8,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peer: Option<String>,
    #[serde(default)]
    from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lends: Option<Lends>,
    /// Where the sender accepts peer-to-peer connections, signed with its
    /// key (3.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<String>,
}

/// A pasted code, read.
struct Code {
    token: String,
    peer: Option<String>,
    from: String,
    lends: Option<Lends>,
    at: Option<String>,
}

/// An invite waiting here, as kept.
#[derive(Clone, Serialize, Deserialize)]
struct Waiting {
    id: String,
    token: String,
    peer: Option<String>,
    from: String,
    #[serde(default)]
    lends: Option<Lends>,
    received_at_ms: u64,
    /// The sender's signed address record, kept when the invite is
    /// accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<String>,
}

/// What a token grants: a collection, or the use of a computer.
pub(crate) enum What {
    Space { space: String, access: ShareAccess },
    Compute { task_ms: Option<u64> },
}

/// A token's grant, read from it.
pub(crate) struct Grant {
    pub(crate) owner: Did,
    pub(crate) what: What,
    pub(crate) expires_at: u64,
}

/// The invite code for a grant: its token, where this data center is
/// reached (its Flight address, left out when it only says the node
/// listens everywhere, so the receiver finds it on the network; and its
/// signed peer-to-peer addresses), the name it goes by, and for a lent
/// computer how much it lends.
pub(crate) fn invite_code(
    token: &str,
    peer: Option<&str>,
    from: &str,
    lends: Option<Lends>,
    at: Option<String>,
) -> String {
    let wire = Wire {
        v: 1,
        token: token.to_string(),
        peer: peer.filter(|p| reachable(p)).map(str::to_string),
        from: from.to_string(),
        lends,
        at,
    };
    let json = serde_json::to_vec(&wire).unwrap_or_default();
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

/// Whether others could reach `url`; `0.0.0.0` and `[::]` only say where
/// the node listens.
fn reachable(url: &str) -> bool {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    authority
        .trim_end_matches('/')
        .parse::<std::net::SocketAddr>()
        .map_or(true, |addr| !addr.ip().is_unspecified())
}

fn incomplete() -> ApiError {
    ApiError::invalid("that isn't a whole invite code; copy all of it and paste it again")
        .because("invite_incomplete")
}

/// The token, address and sender's name in a pasted code. Line breaks and
/// spaces a message added are ignored; a token alone (as the command line
/// prints one) is taken too.
fn decode(code: &str) -> Result<Code, ApiError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.is_empty() {
        return Err(
            ApiError::invalid("paste the invite code you were sent").because("invite_empty")
        );
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(incomplete());
    }
    if let Some(rest) = code.strip_prefix(PREFIX) {
        let bytes = URL_SAFE_NO_PAD.decode(rest).map_err(|_| incomplete())?;
        let wire: Wire = serde_json::from_slice(&bytes).map_err(|_| incomplete())?;
        if wire.v != 1 {
            return Err(ApiError::invalid(
                "this invite comes from a newer Oikome; update Oikome to add it",
            )
            .because("invite_newer"));
        }
        let peer = wire
            .peer
            .map(|p| oikome_catalog::validate_peer_url(&p).map_err(|_| incomplete()))
            .transpose()?;
        Ok(Code {
            token: wire.token,
            peer,
            from: wire.from,
            lends: wire.lends,
            // Read when accepted; one far too long isn't a record.
            at: wire.at.filter(|a| a.len() <= 8192),
        })
    } else if code.split('.').count() == 3 {
        Ok(Code {
            token: code,
            peer: None,
            from: String::new(),
            lends: None,
            at: None,
        })
    } else {
        Err(incomplete())
    }
}

/// A name to show for the sender: the one its code gave, cleaned up, or
/// else the start and end of its ID.
pub(crate) fn sender_name(from: &str, owner: &Did) -> String {
    let name: String = from
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_FROM_CHARS)
        .collect();
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    let id = owner.as_str().trim_start_matches("did:key:");
    let head: String = id.chars().take(8).collect();
    let tail: String = id
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// What a token grants, read without checking its signature: a
/// collection of its issuer's, or else the use of its issuer's computer.
pub(crate) fn grant_of(token: &str) -> Option<Grant> {
    let ucan = Ucan::decode(token).ok()?;
    let owner = ucan.root_issuer().ok()?;
    let prefix = format!("space://{owner}/");
    let node = format!("node://{owner}");
    let att = &ucan.payload().att;
    let space = att.iter().find_map(|c| {
        let space = c.with.strip_prefix(&prefix)?;
        (c.can == ABILITY_QUERY || c.can == ABILITY_READ).then(|| (space.to_string(), &c.can))
    });
    let what = match space {
        Some((space, can)) => {
            validate_space_name(&space).ok()?;
            What::Space {
                access: if can == ABILITY_READ {
                    ShareAccess::Copy
                } else {
                    ShareAccess::Query
                },
                space,
            }
        }
        None => {
            let compute = att
                .iter()
                .find(|c| c.with == node && c.can.starts_with("compute/"))?;
            What::Compute {
                task_ms: compute.nb.and_then(|nb| nb.time_ms),
            }
        }
    };
    Some(Grant {
        owner,
        what,
        expires_at: ucan.payload().exp,
    })
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A list kept in a file of the owner's alone; none yet is empty.
pub(crate) fn read_private<I: DeserializeOwned>(
    path: &std::path::Path,
) -> Result<Vec<I>, ApiError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(ApiError::internal),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(ApiError::internal(e)),
    }
}

impl Api {
    fn invites_path(&self) -> PathBuf {
        self.catalog
            .oikome_dir()
            .join("identity")
            .join(INVITES_FILE)
    }

    fn waiting(&self) -> Result<Vec<Waiting>, ApiError> {
        read_private(&self.invites_path())
    }

    /// The names people gave in the invites waiting here, by their DID.
    pub(crate) fn invite_names(&self) -> Vec<(String, String)> {
        self.waiting()
            .unwrap_or_default()
            .iter()
            .filter_map(|w| {
                let grant = grant_of(&w.token)?;
                Some((grant.owner.to_string(), sender_name(&w.from, &grant.owner)))
            })
            .collect()
    }

    fn change_waiting<T>(
        &self,
        change: impl FnOnce(&mut Vec<Waiting>) -> T,
    ) -> Result<T, ApiError> {
        self.change_private(&self.invites_path(), change)
    }

    /// Change a list kept in a file of the owner's alone, as one step.
    pub(crate) fn change_private<I: Serialize + DeserializeOwned, T>(
        &self,
        path: &std::path::Path,
        change: impl FnOnce(&mut Vec<I>) -> T,
    ) -> Result<T, ApiError> {
        let _one = self
            .settings_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut items: Vec<I> = read_private(path)?;
        let out = change(&mut items);
        let dir = path.parent().unwrap_or(path);
        std::fs::create_dir_all(dir).map_err(ApiError::internal)?;
        let bytes = serde_json::to_vec_pretty(&items).map_err(ApiError::internal)?;
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(ApiError::internal)?;
            file.write_all(&bytes).map_err(ApiError::internal)?;
            file.sync_all().map_err(ApiError::internal)?;
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ApiError::internal(e)
        })?;
        Ok(out)
    }

    /// Check a token as what it grants would be added with: made for this
    /// data center by another, signed, and not ended.
    fn check_grant(&self, token: &str) -> Result<Grant, ApiError> {
        let identity = self.engines.identity()?;
        let me = identity.did();
        let ucan = Ucan::decode(token).map_err(|_| incomplete())?;
        if ucan.root_issuer().is_ok_and(|owner| &owner == me) {
            return Err(ApiError::invalid(
                "this invite is one you made; send it to the person you shared with",
            )
            .because("invite_own"));
        }
        if &ucan.payload().aud != me {
            return Err(ApiError::invalid(
                "this invite is for another data center; ask them to share with your ID, from Settings",
            )
            .because("invite_other"));
        }
        let grant = grant_of(token).ok_or_else(|| {
            ApiError::invalid("this invite doesn't share a collection or lend a computer")
                .because("invite_nothing")
        })?;
        if grant.expires_at <= now_secs() {
            return Err(
                ApiError::invalid("this invite has ended; ask them to share again")
                    .because("invite_ended"),
            );
        }
        ucan::verify(
            token,
            &VerifyOptions {
                root: &grant.owner,
                audience: me,
                required: &match &grant.what {
                    What::Space { space, .. } => {
                        Capability::space(&grant.owner, space, ABILITY_QUERY)
                    }
                    What::Compute { .. } => {
                        Capability::compute(&grant.owner, oikome_node::compute::METHOD_SQL, None)
                    }
                },
                now: now_secs(),
                revoked: &HashSet::new(),
            },
        )
        .map_err(|_| {
            ApiError::invalid("this invite code was changed after it was made; ask for it again")
                .because("invite_changed")
        })?;
        Ok(grant)
    }

    /// The collection here that shows `space` of `owner` live, if one does.
    fn added_from(&self, owner: &str, space: &str) -> Option<String> {
        self.catalog.list_spaces().into_iter().find_map(|s| {
            let uri = s.uri.as_ref()?;
            let live = matches!(
                self.catalog.get_location(&s.location).map(|l| l.format),
                Ok(LocationFormat::Remote { .. })
            );
            (live && uri.owner_did == owner && uri.name == space).then(|| s.name.clone())
        })
    }

    /// `space`, or `space-2`, `space-3`… when the name is taken here.
    fn free_name(&self, space: &str) -> String {
        let taken = |name: &str| {
            self.catalog.get_space(name).is_ok()
                || self.catalog.get_location(name).is_ok()
                || self.catalog.get_source(name).is_ok()
        };
        std::iter::once(space.to_string())
            .chain((2..).map(|i| format!("{space}-{i}")))
            .find(|n| !taken(n))
            .unwrap_or_else(|| space.to_string())
    }

    fn summary(&self, waiting: &Waiting, now: u64) -> Option<Invite> {
        let grant = grant_of(&waiting.token)?;
        let owner = grant.owner.as_str();
        let contact = self.catalog.get_contact(owner);
        let (kind, space, access, name, renews) = match &grant.what {
            What::Space { space, access } => {
                let renews = self.added_from(owner, space);
                (
                    InviteKind::Collection,
                    space.clone(),
                    *access,
                    renews.clone().unwrap_or_else(|| self.free_name(space)),
                    renews.is_some(),
                )
            }
            What::Compute { .. } => (
                InviteKind::Compute,
                String::new(),
                ShareAccess::Query,
                String::new(),
                self.borrowed_from(owner).is_some(),
            ),
        };
        Some(Invite {
            id: waiting.id.clone(),
            kind,
            from_did: owner.to_string(),
            from_name: contact
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| sender_name(&waiting.from, &grant.owner)),
            from_contact: contact.is_some(),
            space,
            access,
            concurrent: waiting.lends.map(|l| l.concurrent),
            per_hour: waiting.lends.map(|l| l.per_hour),
            name,
            renews,
            expires_at: grant.expires_at,
            ended: grant.expires_at <= now,
            received_at_ms: waiting.received_at_ms,
        })
    }

    /// Invites waiting here, newest first.
    pub async fn list_invites(&self, caller: &Caller, _: Empty) -> Result<Vec<Invite>, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let now = now_secs();
        let mut out: Vec<Invite> = self
            .waiting()?
            .iter()
            .filter_map(|w| self.summary(w, now))
            .collect();
        out.sort_by_key(|i| std::cmp::Reverse(i.received_at_ms));
        Ok(out)
    }

    /// Add an invite code someone sent; it waits here to be accepted or
    /// declined. The same code twice is one invite.
    pub async fn add_invite(&self, caller: &Caller, req: InviteCode) -> Result<Invite, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let Code {
            token,
            peer,
            from,
            lends,
            at,
        } = decode(&req.invite)?;
        let grant = self.check_grant(&token)?;
        let owner = grant.owner.as_str();
        let already = match &grant.what {
            What::Space { space, .. } => self.added_from(owner, space).and_then(|name| {
                let held = self.catalog.remote_token(&name).ok()?;
                (held.expose() == token).then(|| {
                    ApiError::new(
                        ErrorCode::Conflict,
                        format!(
                            "you added this invite already: it's {name}, under Shared With You"
                        ),
                    )
                    .because("invite_added_space")
                    .with("name", name)
                })
            }),
            What::Compute { .. } => self
                .borrowed_from(owner)
                .filter(|held| held.token == token)
                .map(|_| {
                    ApiError::new(
                        ErrorCode::Conflict,
                        "you added this invite already: it's under Compute, on Sharing",
                    )
                    .because("invite_added_compute")
                }),
        };
        if let Some(error) = already {
            return Err(error);
        }
        let id = token_id(&token);
        let waiting = Waiting {
            id: id.clone(),
            token,
            peer,
            from,
            lends,
            received_at_ms: crate::now_ms(),
            at,
        };
        let kept = self.change_waiting(|all| {
            if let Some(same) = all.iter().find(|w| w.id == id) {
                return Ok(same.clone());
            }
            if all.len() >= MAX_WAITING {
                return Err(ApiError::invalid(
                    "too many invites are waiting; accept or decline some first",
                )
                .because("invites_too_many"));
            }
            all.push(waiting.clone());
            Ok(waiting)
        })??;
        self.summary(&kept, now_secs())
            .ok_or_else(|| ApiError::internal("the added invite can't be read"))
    }

    /// Accept an invite: its collection is added here, queried where it is
    /// kept, or its computer is kept to borrow; its sender becomes a
    /// contact. Accepting a share of a collection already here, or a
    /// computer already lent here (a renewal), replaces it.
    pub async fn accept_invite(
        &self,
        caller: &Caller,
        req: AcceptInvite,
    ) -> Result<AcceptedInvite, ApiError> {
        caller.require_owner()?;
        self.refresh()?;
        let waiting = self
            .waiting()?
            .into_iter()
            .find(|w| w.id == req.id)
            .ok_or_else(|| {
                ApiError::not_found(
                    "this invite isn't waiting any more; it was accepted or declined",
                )
                .because("invite_not_waiting")
            })?;
        let grant = self.check_grant(&waiting.token)?;
        let owner = grant.owner.as_str().to_string();
        let from = sender_name(&waiting.from, &grant.owner);
        // Where the sender is reached from another network.
        self.learn_record(&owner, waiting.at.as_deref());
        let space = match &grant.what {
            What::Space { space, .. } => space.clone(),
            What::Compute { .. } => {
                let held = Held {
                    id: waiting.id.clone(),
                    token: waiting.token.clone(),
                    peer: waiting.peer.clone(),
                    owner: owner.clone(),
                    from: from.clone(),
                    lends: waiting.lends,
                    accepted_at_ms: crate::now_ms(),
                    in_use: false,
                };
                self.change_borrowed(|all| {
                    all.retain(|h| h.owner != held.owner);
                    all.push(held);
                })?;
                self.adopt_sender(&owner, &from);
                self.change_waiting(|all| all.retain(|w| w.id != waiting.id))?;
                return Ok(AcceptedInvite {
                    kind: InviteKind::Compute,
                    name: self.catalog.get_contact(&owner).map_or(from, |c| c.name),
                });
            }
        };
        let renews = self.added_from(&owner, &space);
        let name = match req.name.as_deref().map(str::trim) {
            Some(name) => {
                validate_space_name(name)?;
                name.to_string()
            }
            None => renews.clone().unwrap_or_else(|| self.free_name(&space)),
        };
        let session = self.session(None);
        if renews.as_deref() == Some(name.as_str()) {
            session.drop_space(&name).await?;
        }
        let mut request = CreateSpaceRequest::new(&name);
        request.kind = Some(oikome_core::SpaceKind::Remote);
        request.mode = Some(RemoteMode::Query);
        request.peer = waiting.peer.clone();
        request.remote = Some(space);
        request.token = Some(waiting.token.clone());
        session.create_space(request).await?;
        self.adopt_sender(&owner, &from);
        self.change_waiting(|all| all.retain(|w| w.id != waiting.id))?;
        Ok(AcceptedInvite {
            kind: InviteKind::Collection,
            name,
        })
    }

    /// The sender of an accepted invite becomes a contact, under the name
    /// its code gave (which can always be changed later).
    fn adopt_sender(&self, owner: &str, name: &str) {
        if self.catalog.get_contact(owner).is_none() {
            let _ = self.catalog.add_contact(owner, name);
        }
    }

    /// Decline an invite: it is forgotten here. Nothing is sent back.
    pub async fn decline_invite(
        &self,
        caller: &Caller,
        req: InviteRef,
    ) -> Result<Vec<Invite>, ApiError> {
        caller.require_owner()?;
        self.change_waiting(|all| all.retain(|w| w.id != req.id))?;
        self.list_invites(caller, Empty {}).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_carry_the_token_address_and_name() {
        let lends = Lends {
            concurrent: 2,
            per_hour: 30,
        };
        let code = invite_code(
            "a.b.c",
            Some("https://127.0.0.1:7330"),
            "Rosh's data center",
            Some(lends),
            Some("c2lnbmVk".to_string()),
        );
        assert!(code.starts_with(PREFIX));
        // A message may wrap it.
        let wrapped = format!("  {}\n{}  ", &code[..20], &code[20..]);
        let read = decode(&wrapped).unwrap();
        assert_eq!(read.token, "a.b.c");
        assert_eq!(read.peer.as_deref(), Some("https://127.0.0.1:7330"));
        assert_eq!(read.from, "Rosh's data center");
        assert_eq!(read.lends, Some(lends));
        assert_eq!(read.at.as_deref(), Some("c2lnbmVk"));
    }

    #[test]
    fn codes_from_before_addresses_were_signed_still_read() {
        // As Oikome made them before 3.12: no `at`.
        let json = br#"{"v":1,"token":"a.b.c","from":"Home"}"#;
        let code = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json));
        let read = decode(&code).unwrap();
        assert_eq!(read.token, "a.b.c");
        assert_eq!(read.at, None);
    }

    #[test]
    fn listening_everywhere_is_not_an_address() {
        let read = decode(&invite_code(
            "a.b.c",
            Some("https://0.0.0.0:7330"),
            "",
            None,
            None,
        ))
        .unwrap();
        assert_eq!(read.peer, None);
        assert!(reachable("https://nas.local:7330"));
    }

    #[test]
    fn a_bare_token_is_an_invite_and_anything_else_is_not() {
        assert_eq!(decode("x.y.z").unwrap().token, "x.y.z");
        assert!(decode("").is_err());
        assert!(decode("oik1-!!!").is_err());
        assert!(decode("hello").is_err());
        let changed = invite_code("a.b.c", Some("https://h:1/path?x"), "", None, None);
        assert!(decode(&changed).is_err());
    }
}
