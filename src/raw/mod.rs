//! Optional raw multicast transmit support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need to
//! inject complete multicast IP datagrams instead of ordinary UDP payloads.
//!
//! Linux combines raw IP sockets with packet-socket injection for remote-source
//! IPv6 forwarding. macOS supports raw IPv4 and local-source raw IPv6, while
//! Windows currently supports raw IPv4. Unsupported paths return an explicit
//! error rather than silently degrading to UDP behavior.

mod config;
mod context;
mod datagram;
#[cfg(target_os = "linux")]
mod linux_packet;
mod platform;
mod publication;
mod report;

pub use config::{RawPublicationConfig, RawValidationMode};
pub use context::RawContext;
pub use publication::{RawPublication, RawPublicationId};
pub use report::RawSendReport;
