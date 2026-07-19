//! Optional raw multicast transmit support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need to
//! inject complete multicast IP datagrams instead of ordinary UDP payloads.
//!
//! Linux combines raw IP sockets with packet-socket injection for remote-source
//! IPv6 forwarding. macOS supports raw IPv4 and local-source raw IPv6, while
//! Windows currently supports raw IPv4. Unsupported paths return an explicit
//! error rather than silently degrading to UDP behavior.
//!
//! The additive `raw-route-egress` feature allows IPv4 publications on Linux
//! and macOS to leave egress selection to the kernel routing table. Explicit
//! bind/interface selection remains the default.

#[cfg(feature = "raw-route-egress")]
mod capabilities;
mod config;
mod context;
mod datagram;
#[cfg(target_os = "linux")]
mod linux_packet;
mod platform;
mod publication;
mod report;

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
