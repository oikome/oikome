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

//! Being reached from outside the node's network (roadmap 3.12), between
//! the owner's API, which decides when it is wanted (the owner allowed it
//! for a share or a lent computer), and the peer-to-peer layer, which asks
//! the router to open a port or meets contacts at another contact.

use oikome_identity::Did;

/// What the owner's choices ask of the network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReachWanted {
    /// Something the owner allowed needs this node reached from outside:
    /// the router is asked to open a port, or else a contact helps.
    pub open: bool,
    /// Help contacts reach each other here, when this node itself can be
    /// reached from outside.
    pub help_contacts: bool,
}

/// How this node can be reached from outside its network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Only from its own network: nothing asked for more, or not yet.
    LocalOnly,
    /// The router is being asked.
    Opening,
    /// Directly: the router opened a port, or the node has an address of
    /// its own on the internet.
    Direct,
    /// Through a contact both sides reach, who introduces them so they
    /// connect directly.
    ThroughContact(Did),
    /// The router didn't open a port, and no contact could help.
    RouterRefused,
    /// The router's own address is shared with others (the provider's
    /// carrier-grade NAT, or a second router), so no port it opens is
    /// reachable.
    SharedAddress,
}

/// The network as the owner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkStatus {
    pub wanted: ReachWanted,
    pub reach: Reach,
    /// Contacts can meet here now.
    pub helping: bool,
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self {
            wanted: ReachWanted::default(),
            reach: Reach::LocalOnly,
            helping: false,
        }
    }
}

/// The node's peer-to-peer layer, as the owner's API uses it.
pub trait NodeNetwork: Send + Sync + 'static {
    fn status(&self) -> NetworkStatus;
    /// Say what the owner's choices ask of the network now.
    fn set_reach(&self, wanted: ReachWanted);
    /// Where this node is reached, signed with its key, for an invite.
    fn address_record(&self) -> Option<String>;
    /// Keep where `did` is reached, from a record it signed. A record
    /// signed by anyone else is refused.
    fn learn_record(&self, did: &Did, record: &str) -> Result<(), String>;
}
