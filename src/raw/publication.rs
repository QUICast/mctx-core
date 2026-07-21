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

    /// Sends one complete IP datagram through the selected raw backend.
    ///
    /// Supported IPv6 backends use link-layer injection and preserve the
    /// complete supplied datagram. They do not feed the local IP receive path.
    pub fn send_raw(&self, ip_datagram: &[u8]) -> Result<RawSendReport, MctxError> {
        send_raw_ip_datagram(&self.socket, self.id, &self.config, ip_datagram)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn raw_ipv6_publication_accepts_explicit_loopback_override() {
        let publication = RawPublication::new(
            RawPublicationId(2),
            RawPublicationConfig::ipv6()
                .with_ipv6_interface_index(7)
                .with_loopback(false),
        )
        .unwrap();

        assert_eq!(publication.config().loopback, Some(false));
    }
}
