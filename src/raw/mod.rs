//! Optional raw multicast transmit support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need to
//! inject complete multicast IP datagrams instead of ordinary UDP payloads.
//!
//! The first implementation targets Linux packet sockets. Other platforms
//! currently return a clear unsupported error rather than silently degrading to
//! UDP behavior.

mod config;
mod context;
mod datagram;
mod platform;
mod publication;
mod report;

pub use config::{RawPublicationConfig, RawValidationMode};
pub use context::RawContext;
pub use publication::{RawPublication, RawPublicationId};
pub use report::RawSendReport;
