//! Optional generic raw-IP transmit support.
//!
//! Enable this module with the `raw-ip` Cargo feature when a caller already
//! owns a complete IP datagram, such as an ICMP Packet Too Big control packet.
//! This API is intentionally separate from [`crate::raw`]: it accepts unicast
//! and multicast destinations and does not apply multicast forwarding policy.

mod capabilities;
mod config;
mod context;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
mod datagram;
mod platform;
mod publication;
mod report;

pub use capabilities::{RawIpCapabilities, RawIpCapability, raw_ip_capabilities};
pub use config::{RawIpPublicationConfig, RawIpSocketConfig};
pub use context::RawIpContext;
pub use publication::{RawIpPublication, RawIpPublicationId};
pub use report::RawIpSendReport;
