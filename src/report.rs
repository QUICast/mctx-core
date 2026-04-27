use crate::PublicationId;
use std::net::{Ipv4Addr, SocketAddrV4};

/// Result of one multicast send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReport {
    pub publication_id: PublicationId,
    pub destination: SocketAddrV4,
    pub local_addr: Option<SocketAddrV4>,
    pub source_addr: Option<Ipv4Addr>,
    pub bytes_sent: usize,
}
