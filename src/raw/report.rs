use crate::config::{OutgoingInterface, PublicationAddressFamily};
use crate::raw::RawPublicationId;
use std::net::IpAddr;

/// Result of one raw multicast send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawSendReport {
    /// Publication used for the send.
    pub publication_id: RawPublicationId,
    /// Parsed IP address family.
    pub family: PublicationAddressFamily,
    /// Source address parsed from the supplied datagram.
    pub source_ip: Option<IpAddr>,
    /// Destination address parsed from the supplied datagram.
    pub destination_ip: Option<IpAddr>,
    /// IPv4 protocol or IPv6 next-header value from the supplied datagram.
    pub ip_protocol: Option<u8>,
    /// Complete IP datagram length accepted by the backend.
    pub bytes_sent: usize,
    /// Local address used to select the egress interface, if configured.
    pub local_bind_addr: Option<IpAddr>,
    /// Caller-provided outgoing-interface selector.
    pub outgoing_interface: Option<OutgoingInterface>,
    /// Resolved outgoing interface index, if known.
    pub outgoing_interface_index: Option<u32>,
}
