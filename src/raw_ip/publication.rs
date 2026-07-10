use crate::error::MctxError;
use crate::raw_ip::platform::{RawIpTransmitSocket, open_raw_ip_socket, send_ip_datagram};
use crate::raw_ip::{RawIpSendReport, RawIpSocketConfig};

/// Stable identifier for one generic raw-IP publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawIpPublicationId(pub u64);

/// One ready-to-send generic raw-IP socket configuration.
#[derive(Debug)]
pub struct RawIpPublication {
    id: RawIpPublicationId,
    config: RawIpSocketConfig,
    socket: RawIpTransmitSocket,
}

impl RawIpPublication {
    /// Creates a ready-to-send raw-IP publication.
    pub fn new(id: RawIpPublicationId, config: RawIpSocketConfig) -> Result<Self, MctxError> {
        let socket = open_raw_ip_socket(&config)?;
        Ok(Self { id, config, socket })
    }

    /// Returns this publication's stable identifier.
    pub fn id(&self) -> RawIpPublicationId {
        self.id
    }

    /// Returns the immutable socket configuration.
    pub fn config(&self) -> &RawIpSocketConfig {
        &self.config
    }

    /// Sends a complete caller-provided IPv4 or IPv6 datagram.
    pub fn send_ip_datagram(&self, ip_datagram: &[u8]) -> Result<RawIpSendReport, MctxError> {
        send_ip_datagram(&self.socket, self.id, ip_datagram)
    }
}
