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

//! Asking the home router to open the node's port (3.12), only while the
//! owner allowed something that needs it: UPnP IGD first (most home
//! routers), then PCP and NAT-PMP at the default gateway. One TCP port is
//! mapped, the node's peer-to-peer port; the web pages and assistants'
//! endpoint stay on this computer.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use crab_nat::{InternetProtocol, PortMapping, PortMappingOptions, PortMappingType, TimeoutConfig};
use futures::StreamExt;
use rupnp::http::Uri;
use rupnp::ssdp::{SearchTarget, URN};

/// How long a mapping is asked for; it is renewed at half of that.
const LEASE_SECS: u32 = 3600;
/// How long to wait for routers to answer a UPnP search.
const SEARCH_FOR: Duration = Duration::from_secs(2);
/// What the router lists the mapping as.
const DESCRIPTION: &str = "Oikome";
/// UPnP error: the router only keeps mappings without an end.
const ONLY_PERMANENT_LEASES: u16 = 725;

const GATEWAYS: [URN; 2] = [
    URN::device("schemas-upnp-org", "InternetGatewayDevice", 2),
    URN::device("schemas-upnp-org", "InternetGatewayDevice", 1),
];
const CONNECTIONS: [URN; 3] = [
    URN::service("schemas-upnp-org", "WANIPConnection", 2),
    URN::service("schemas-upnp-org", "WANIPConnection", 1),
    URN::service("schemas-upnp-org", "WANPPPConnection", 1),
];

/// PCP and NAT-PMP retry briefly: a router that supports them answers at
/// once.
const QUICK: TimeoutConfig = TimeoutConfig {
    initial_timeout: Duration::from_millis(250),
    max_retries: 2,
    max_retry_timeout: Some(Duration::from_secs(1)),
};

/// What asking the router came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapped {
    /// The router forwards `external` to this node's port.
    Open { external: SocketAddrV4 },
    /// No router answered, or it refused.
    Refused(String),
}

enum Current {
    Upnp {
        device: Uri,
        service: rupnp::Service,
        local: Ipv4Addr,
        external: SocketAddrV4,
        renew_at: Instant,
    },
    Pmp {
        mapping: PortMapping,
        external: SocketAddrV4,
        renew_at: Instant,
    },
}

impl Current {
    fn external(&self) -> SocketAddrV4 {
        match self {
            Current::Upnp { external, .. } | Current::Pmp { external, .. } => *external,
        }
    }
}

/// Keeps the node's port open on the router while asked to.
pub struct PortMapper {
    port: u16,
    current: Option<Current>,
}

/// The default gateway, and this computer's address towards it.
fn gateway() -> Option<(Ipv4Addr, Ipv4Addr)> {
    let gateway = *netdev::get_default_gateway().ok()?.ipv4.first()?;
    // No packet is sent: connecting a UDP socket only picks the route.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((gateway, 5351)).ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(local) => Some((gateway, *local.ip())),
        SocketAddr::V6(_) => None,
    }
}

fn renew_at(lease_secs: u32) -> Instant {
    let half = if lease_secs == 0 {
        LEASE_SECS / 2
    } else {
        lease_secs / 2
    };
    Instant::now() + Duration::from_secs(u64::from(half.max(60)))
}

impl PortMapper {
    /// A mapper for TCP `port` (the same port outside as inside, when the
    /// router allows).
    pub fn new(port: u16) -> Self {
        Self {
            port,
            current: None,
        }
    }

    /// Open the port, or keep it open.
    pub async fn ensure(&mut self) -> Mapped {
        if let Some(current) = &mut self.current {
            if Instant::now() < renew_at_of(current) {
                return Mapped::Open {
                    external: current.external(),
                };
            }
            match renew(current, self.port).await {
                Ok(()) => {
                    return Mapped::Open {
                        external: current.external(),
                    }
                }
                Err(e) => {
                    tracing::info!(error = %e, "the router stopped keeping the port open");
                    self.current = None;
                }
            }
        }
        let Some((gateway, local)) = gateway() else {
            return Mapped::Refused("this computer has no router to ask".to_string());
        };
        let mut why = Vec::new();
        match upnp_map(self.port, local).await {
            Ok(current) => {
                let external = current.external();
                tracing::info!(%external, "the router opened the port (UPnP)");
                self.current = Some(current);
                return Mapped::Open { external };
            }
            Err(e) => why.push(format!("UPnP: {e}")),
        }
        match pmp_map(self.port, gateway, local).await {
            Ok(current) => {
                let external = current.external();
                tracing::info!(%external, "the router opened the port (PCP / NAT-PMP)");
                self.current = Some(current);
                return Mapped::Open { external };
            }
            Err(e) => why.push(format!("PCP / NAT-PMP: {e}")),
        }
        Mapped::Refused(why.join("; "))
    }

    /// Close the port again.
    pub async fn release(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        let closed = match current {
            Current::Upnp {
                device, service, ..
            } => {
                let payload = format!(
                    "<NewRemoteHost></NewRemoteHost><NewExternalPort>{}</NewExternalPort>\
                     <NewProtocol>TCP</NewProtocol>",
                    self.port
                );
                service
                    .action(&device, "DeletePortMapping", &payload)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            Current::Pmp { mapping, .. } => {
                mapping.try_drop().await.map_err(|(e, _)| e.to_string())
            }
        };
        match closed {
            Ok(()) => tracing::info!("the router closed the port"),
            Err(e) => tracing::info!(error = %e, "the router didn't close the port; it lapses"),
        }
    }
}

fn renew_at_of(current: &Current) -> Instant {
    match current {
        Current::Upnp { renew_at, .. } | Current::Pmp { renew_at, .. } => *renew_at,
    }
}

async fn renew(current: &mut Current, port: u16) -> Result<(), String> {
    match current {
        Current::Upnp {
            device,
            service,
            local,
            renew_at: at,
            ..
        } => {
            let lease = add_mapping(service, device, port, *local).await?;
            *at = renew_at(lease);
            Ok(())
        }
        Current::Pmp {
            mapping,
            renew_at: at,
            ..
        } => {
            mapping.renew().await.map_err(|e| e.to_string())?;
            *at = renew_at(mapping.lifetime());
            Ok(())
        }
    }
}

/// Ask for the mapping; the lease the router took.
async fn add_mapping(
    service: &rupnp::Service,
    device: &Uri,
    port: u16,
    local: Ipv4Addr,
) -> Result<u32, String> {
    // Every value is a number or this computer's own address: nothing
    // from outside goes into the request.
    let payload = |lease: u32| {
        format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort>\
             <NewProtocol>TCP</NewProtocol><NewInternalPort>{port}</NewInternalPort>\
             <NewInternalClient>{local}</NewInternalClient><NewEnabled>1</NewEnabled>\
             <NewPortMappingDescription>{DESCRIPTION}</NewPortMappingDescription>\
             <NewLeaseDuration>{lease}</NewLeaseDuration>"
        )
    };
    match service
        .action(device, "AddPortMapping", &payload(LEASE_SECS))
        .await
    {
        Ok(_) => Ok(LEASE_SECS),
        Err(rupnp::Error::UPnPError(e)) if e.err_code() == ONLY_PERMANENT_LEASES => service
            .action(device, "AddPortMapping", &payload(0))
            .await
            .map(|_| 0)
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

async fn upnp_map(port: u16, local: Ipv4Addr) -> Result<Current, String> {
    for gateway in &GATEWAYS {
        let devices = rupnp::discover(&SearchTarget::URN(gateway.clone()), SEARCH_FOR, None)
            .await
            .map_err(|e| e.to_string())?;
        let mut devices = std::pin::pin!(devices);
        while let Some(device) = devices.next().await {
            let Ok(device) = device else {
                continue;
            };
            let Some(service) = CONNECTIONS.iter().find_map(|urn| device.find_service(urn)) else {
                continue;
            };
            let url = device.url().clone();
            let lease = add_mapping(service, &url, port, local).await?;
            let answer = service
                .action(&url, "GetExternalIPAddress", "")
                .await
                .map_err(|e| e.to_string())?;
            let ip: Ipv4Addr = answer
                .get("NewExternalIPAddress")
                .and_then(|ip| ip.trim().parse().ok())
                .ok_or("the router didn't say its address")?;
            return Ok(Current::Upnp {
                device: url,
                service: service.clone(),
                local,
                external: SocketAddrV4::new(ip, port),
                renew_at: renew_at(lease),
            });
        }
    }
    Err("no router answered".to_string())
}

async fn pmp_map(port: u16, gateway: Ipv4Addr, local: Ipv4Addr) -> Result<Current, String> {
    let internal = NonZeroU16::new(port).ok_or("no port")?;
    let mapping = PortMapping::new(
        gateway.into(),
        IpAddr::V4(local),
        InternetProtocol::Tcp,
        internal,
        PortMappingOptions {
            external_port: Some(internal),
            lifetime_seconds: Some(LEASE_SECS),
            timeout_config: Some(QUICK),
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    let ip = match mapping.mapping_type() {
        PortMappingType::Pcp {
            external_ip: IpAddr::V4(ip),
            ..
        } => ip,
        _ => crab_nat::natpmp::external_address(gateway.into(), Some(QUICK))
            .await
            .map_err(|e| e.to_string())?,
    };
    let external = SocketAddrV4::new(ip, mapping.external_port().get());
    let lease = mapping.lifetime();
    Ok(Current::Pmp {
        mapping,
        external,
        renew_at: renew_at(lease),
    })
}
