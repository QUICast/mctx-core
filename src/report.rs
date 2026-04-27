use crate::PublicationId;
use std::net::SocketAddrV4;

/// Result of one multicast send call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReport {
    pub publication_id: PublicationId,
    pub destination: SocketAddrV4,
    pub bytes_sent: usize,
}
