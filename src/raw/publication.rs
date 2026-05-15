use crate::error::MctxError;
use crate::raw::platform::{RawTransmitSocket, open_raw_transmit_socket, send_raw_ip_datagram};
use crate::raw::{RawPublicationConfig, RawSendReport};

/// Stable ID for one configured raw publication socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawPublicationId(pub u64);

/// One ready-to-send raw multicast publication.
#[derive(Debug)]
pub struct RawPublication {
    id: RawPublicationId,
    config: RawPublicationConfig,
    socket: RawTransmitSocket,
}

impl RawPublication {
    /// Creates and configures a new raw publication socket.
    pub fn new(id: RawPublicationId, config: RawPublicationConfig) -> Result<Self, MctxError> {
        config.validate()?;
        let socket = open_raw_transmit_socket(&config)?;

        Ok(Self { id, config, socket })
    }

    /// Returns the raw publication ID.
    pub fn id(&self) -> RawPublicationId {
        self.id
    }

    /// Returns the configured raw publication parameters.
    pub fn config(&self) -> &RawPublicationConfig {
        &self.config
    }

    /// Sends one complete IP datagram without rewriting its IP header.
    pub fn send_raw(&self, ip_datagram: &[u8]) -> Result<RawSendReport, MctxError> {
        send_raw_ip_datagram(&self.socket, self.id, &self.config, ip_datagram)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn raw_publication_rejects_loopback_override_before_privileged_socket_setup() {
        let err = RawPublication::new(
            RawPublicationId(1),
            RawPublicationConfig::ipv4()
                .with_bind_addr(Ipv4Addr::new(192, 168, 1, 20))
                .with_loopback(false),
        )
        .unwrap_err();

        assert!(matches!(err, MctxError::RawPacketTransmitUnsupported(_)));
    }
}
