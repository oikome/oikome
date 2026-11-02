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

//! The owner's contacts: the people (their nodes' DIDs) the owner shares
//! with, under a name the owner chose (architecture v3 §13.4).
//!
//! A contact is only a name for a DID. It grants nothing: access still comes
//! from grants and UCANs issued to that DID.

use oikome_core::CatalogError;
use serde::{Deserialize, Serialize};

use crate::catalog::OikomeCatalog;
use crate::registry::now_ms;

/// Longest contact name, in characters.
pub const MAX_CONTACT_NAME_CHARS: usize = 64;
/// Longest DID accepted for a contact (an Ed25519 `did:key` is 56 bytes).
const MAX_DID_BYTES: usize = 256;

/// Someone the owner shares with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// Their node's DID (`did:key:z…`).
    pub did: String,
    /// The name the owner gave them.
    pub name: String,
    pub added_at_ms: u64,
}

/// A trimmed, non-empty name of at most [`MAX_CONTACT_NAME_CHARS`]
/// characters, without control characters.
pub fn validate_contact_name(name: &str) -> Result<String, CatalogError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_CONTACT_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(CatalogError::InvalidProperty(format!(
            "a contact name has 1 to {MAX_CONTACT_NAME_CHARS} characters and no control characters"
        )));
    }
    Ok(name.to_string())
}

fn check_did(did: &str) -> Result<(), CatalogError> {
    let ok = did.len() <= MAX_DID_BYTES
        && did.starts_with("did:")
        && did.chars().all(|c| c.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(CatalogError::InvalidProperty(format!(
            "'{did}' is not a DID"
        )))
    }
}

impl OikomeCatalog {
    /// Add a contact. A DID can be a contact once; adding it again is an error.
    pub fn add_contact(&self, did: &str, name: &str) -> Result<Contact, CatalogError> {
        check_did(did)?;
        let contact = Contact {
            did: did.to_string(),
            name: validate_contact_name(name)?,
            added_at_ms: now_ms(),
        };
        self.mutate_registry(|reg| {
            if reg.contacts.contains_key(did) {
                return Err(CatalogError::InvalidProperty(format!(
                    "{did} is already a contact"
                )));
            }
            reg.contacts.insert(did.to_string(), contact.clone());
            Ok(contact)
        })
    }

    /// Give a contact another name.
    pub fn rename_contact(&self, did: &str, name: &str) -> Result<Contact, CatalogError> {
        let name = validate_contact_name(name)?;
        self.mutate_registry(|reg| {
            let contact = reg
                .contacts
                .get_mut(did)
                .ok_or_else(|| CatalogError::ContactNotFound(did.to_string()))?;
            contact.name = name;
            Ok(contact.clone())
        })
    }

    /// Remove a contact. Grants and UCANs issued to the DID are not touched
    /// here; revoking them is the caller's decision.
    pub fn remove_contact(&self, did: &str) -> Result<Contact, CatalogError> {
        self.mutate_registry(|reg| {
            reg.contacts
                .remove(did)
                .ok_or_else(|| CatalogError::ContactNotFound(did.to_string()))
        })
    }

    /// The contact with this DID, if any.
    pub fn get_contact(&self, did: &str) -> Option<Contact> {
        self.with_registry(|reg| reg.contacts.get(did).cloned())
    }

    /// Every contact, ordered by DID.
    pub fn list_contacts(&self) -> Vec<Contact> {
        self.with_registry(|reg| reg.contacts.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(validate_contact_name("  Alice ").unwrap(), "Alice");
        assert!(validate_contact_name("   ").is_err());
        assert!(validate_contact_name("a\nb").is_err());
        assert!(validate_contact_name(&"x".repeat(MAX_CONTACT_NAME_CHARS + 1)).is_err());
        assert!(validate_contact_name(&"界".repeat(MAX_CONTACT_NAME_CHARS)).is_ok());
    }

    #[test]
    fn dids_are_checked() {
        assert!(check_did("did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK").is_ok());
        assert!(check_did("alice").is_err());
        assert!(check_did("did:key:z6Mk with space").is_err());
    }
}
