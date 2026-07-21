//! Optional raw multicast transmit support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need to
//! inject complete multicast IP datagrams instead of ordinary UDP payloads.
//!
//! Linux uses AF_PACKET and macOS uses BPF for explicit full-header IPv6
//! forwarding. Windows currently supports raw IPv4 only. Unsupported paths
//! return an explicit error rather than silently degrading to UDP behavior.
//!
//! The additive `raw-route-egress` feature supports route-selected IPv4 on
//! Linux/macOS and full-header IPv6 roaming on Linux. Explicit bind/interface
//! selection remains the default.

mod capabilities;
mod config;
mod context;
mod datagram;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
mod link;
#[cfg(target_os = "linux")]
mod linux_packet;
#[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
mod linux_route;
#[cfg(target_os = "macos")]
mod macos_bpf;
mod platform;
mod publication;
mod report;

pub use capabilities::{
    RawIpv6EgressCapabilities, RawIpv6EgressCapability, raw_ipv6_egress_capabilities,
};
#[cfg(feature = "raw-route-egress")]
pub use capabilities::{
    RawRouteEgressCapabilities, RawRouteEgressCapability, raw_route_egress_capabilities,
};
#[cfg(feature = "raw-route-egress")]
pub use config::RawEgressMode;
pub use config::{RawPublicationConfig, RawValidationMode};
pub use context::RawContext;
pub use publication::{RawPublication, RawPublicationId};
pub use report::RawSendReport;
