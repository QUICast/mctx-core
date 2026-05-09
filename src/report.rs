use crate::PublicationId;
use std::net::{IpAddr, SocketAddr};

/// Result of one multicast send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReport {
    pub publication_id: PublicationId,
    pub destination: SocketAddr,
    pub local_addr: Option<SocketAddr>,
    pub source_addr: Option<IpAddr>,
    pub bytes_sent: usize,
}
