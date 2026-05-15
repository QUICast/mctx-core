use crate::config::{OutgoingInterface, PublicationAddressFamily};
use crate::raw::RawPublicationId;
use std::net::IpAddr;

/// Result of one raw multicast send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawSendReport {
    pub publication_id: RawPublicationId,
    pub family: PublicationAddressFamily,
    pub source_ip: Option<IpAddr>,
    pub destination_ip: Option<IpAddr>,
    pub ip_protocol: Option<u8>,
    pub bytes_sent: usize,
    pub local_bind_addr: Option<IpAddr>,
    pub outgoing_interface: Option<OutgoingInterface>,
    pub outgoing_interface_index: Option<u32>,
}
