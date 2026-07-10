use crate::config::PublicationAddressFamily;
use crate::raw_ip::RawIpPublicationId;
use std::net::IpAddr;

/// Result of one generic raw-IP send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawIpSendReport {
    /// Publication that transmitted the datagram.
    pub publication_id: RawIpPublicationId,
    /// Parsed IP address family.
    pub family: PublicationAddressFamily,
    /// Parsed source address from the supplied IP header.
    pub source_ip: IpAddr,
    /// Parsed destination address from the supplied IP header.
    pub destination_ip: IpAddr,
    /// IPv4 protocol or IPv6 next-header value from the supplied header.
    pub ip_protocol: u8,
    /// Complete caller-provided IP datagram length.
    pub bytes_sent: usize,
    /// Exact local bind address, if configured.
    pub local_bind_addr: Option<IpAddr>,
    /// Local address used to identify the egress interface, if configured.
    pub interface_addr: Option<IpAddr>,
    /// Resolved operating-system egress interface index.
    pub interface_index: u32,
}
