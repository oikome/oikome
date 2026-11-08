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

//! Which addresses are on this node's own network and which are outside
//! it (3.12): only contacts may connect from outside, a Flight endpoint is
//! only recorded for a peer on the same network, and a node with an
//! address on the internet can help its contacts meet.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

/// The IP of a multiaddr, if it has one.
pub fn ip_of(addr: &Multiaddr) -> Option<IpAddr> {
    addr.iter().find_map(|p| match p {
        Protocol::Ip4(ip) => Some(IpAddr::V4(ip)),
        Protocol::Ip6(ip) => Some(IpAddr::V6(ip)),
        _ => None,
    })
}

/// The TCP port of a multiaddr, if it has one.
pub fn tcp_port_of(addr: &Multiaddr) -> Option<u16> {
    addr.iter().find_map(|p| match p {
        Protocol::Tcp(port) => Some(port),
        _ => None,
    })
}

/// Whether a connection at `addr` goes through another node.
pub fn is_relayed(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| matches!(p, Protocol::P2pCircuit))
}

/// 100.64.0.0/10: addresses a provider shares among many homes
/// (carrier-grade NAT, RFC 6598).
pub fn is_shared(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            a == 100 && (64..128).contains(&b)
        }
        IpAddr::V6(_) => false,
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local (fc00::/7) and link-local (fe80::/10).
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || ip.to_ipv4_mapped().is_some_and(is_local_v4)
}

/// Whether `ip` belongs to a home or office network (or this computer)
/// rather than the internet. Shared provider addresses are neither: they
/// reach other homes of the same provider, so they count as outside.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_local_v4(ip),
        IpAddr::V6(ip) => is_local_v6(ip),
    }
}

/// Whether `ip` is an address on the internet that others can connect to.
pub fn is_public(ip: IpAddr) -> bool {
    !is_local(ip) && !is_shared(ip)
}

/// How a node sorts addresses. Tests run every node on this computer, so
/// they can have loopback addresses count as outside the network.
#[derive(Debug, Clone, Copy, Default)]
pub struct Classifier {
    /// Treat this computer's and the local network's addresses as the
    /// internet (tests only).
    pub local_is_outside: bool,
}

impl Classifier {
    /// Whether a connection from `addr` comes from outside this node's
    /// network: from the internet, or through another node.
    pub fn is_outside(&self, addr: &Multiaddr) -> bool {
        if is_relayed(addr) {
            return true;
        }
        match ip_of(addr) {
            Some(ip) => self.local_is_outside || !is_local(ip),
            // A name, or no address: not one of ours.
            None => true,
        }
    }

    /// Whether others on the internet could connect to `addr`.
    pub fn is_public(&self, addr: &Multiaddr) -> bool {
        if is_relayed(addr) {
            return false;
        }
        match ip_of(addr) {
            Some(ip) if ip.is_unspecified() => false,
            Some(ip) => self.local_is_outside || is_public(ip),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> Multiaddr {
        text.parse().unwrap()
    }

    #[test]
    fn home_networks_are_local_and_the_internet_is_not() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.3.4",
            "::1",
            "fe80::1",
            "fd12:3456::1",
        ] {
            let ip: IpAddr = local.parse().unwrap();
            assert!(is_local(ip), "{local}");
            assert!(!is_public(ip), "{local}");
        }
        for public in ["8.8.8.8", "198.51.100.7", "2001:db8::5", "2a00:1450::1"] {
            let ip: IpAddr = public.parse().unwrap();
            assert!(!is_local(ip), "{public}");
            assert!(is_public(ip), "{public}");
        }
        // A provider's shared addresses are neither home nor internet.
        let shared: IpAddr = "100.72.1.2".parse().unwrap();
        assert!(is_shared(shared) && !is_local(shared) && !is_public(shared));
        assert!(!is_shared("100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn connections_through_another_node_come_from_outside() {
        let home = Classifier::default();
        assert!(!home.is_outside(&addr("/ip4/192.168.1.5/tcp/4001")));
        assert!(home.is_outside(&addr("/ip4/203.0.113.9/tcp/4001")));
        assert!(home.is_outside(&addr("/ip4/100.70.0.1/tcp/4001")));
        let relayed = addr(
            "/ip4/192.168.1.5/tcp/4001/p2p/12D3KooWD3eckifWpRn9wQpMG9R9hX3sD158z7EqHWmweQAJU5SA/p2p-circuit",
        );
        assert!(home.is_outside(&relayed));
        assert!(!home.is_public(&relayed));
        assert!(home.is_public(&addr("/ip4/198.51.100.2/tcp/4001")));
        assert!(!home.is_public(&addr("/ip4/0.0.0.0/tcp/4001")));

        let test = Classifier {
            local_is_outside: true,
        };
        assert!(test.is_outside(&addr("/ip4/127.0.0.1/tcp/4001")));
        assert!(test.is_public(&addr("/ip4/127.0.0.1/tcp/4001")));
        assert_eq!(tcp_port_of(&addr("/ip4/127.0.0.1/tcp/4001")), Some(4001));
    }
}
