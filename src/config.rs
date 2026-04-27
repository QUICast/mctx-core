use crate::error::MctxError;
use std::net::{Ipv4Addr, SocketAddrV4};

/// Configuration for one multicast publication socket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicationConfig {
    /// The destination multicast group.
    pub group: Ipv4Addr,
    /// The destination UDP port.
    pub dst_port: u16,
    /// The local interface to use for multicast egress, if explicitly set.
    pub interface: Option<Ipv4Addr>,
    /// The source UDP port to bind before sending, if explicitly set.
    pub source_port: Option<u16>,
    /// The multicast TTL for transmitted packets.
    pub ttl: u32,
    /// Whether outbound multicast packets should be looped back to the local host.
    pub loopback: bool,
}

impl PublicationConfig {
    /// Creates a basic multicast publication configuration.
    pub fn new(group: Ipv4Addr, port: u16) -> Self {
        Self {
            group,
            dst_port: port,
            interface: None,
            source_port: None,
            ttl: 1,
            loopback: true,
        }
    }

    /// Validates the configuration and returns an error if it is not usable.
    pub fn validate(&self) -> Result<(), MctxError> {
        if self.dst_port == 0 {
            return Err(MctxError::InvalidDestinationPort);
        }

        if !self.group.is_multicast() {
            return Err(MctxError::InvalidMulticastGroup);
        }

        if matches!(self.source_port, Some(0)) {
            return Err(MctxError::InvalidSourcePort);
        }

        if let Some(interface) = self.interface
            && interface.is_multicast()
        {
            return Err(MctxError::InvalidInterfaceAddress);
        }

        Ok(())
    }

    /// Sets the multicast egress interface.
    pub fn with_interface(mut self, interface: Ipv4Addr) -> Self {
        self.interface = Some(interface);
        self
    }

    /// Sets the source UDP port.
    pub fn with_source_port(mut self, source_port: u16) -> Self {
        self.source_port = Some(source_port);
        self
    }

    /// Sets the multicast TTL.
    pub fn with_ttl(mut self, ttl: u32) -> Self {
        self.ttl = ttl;
        self
    }

    /// Enables or disables multicast loopback.
    pub fn with_loopback(mut self, loopback: bool) -> Self {
        self.loopback = loopback;
        self
    }

    /// Returns the configured destination socket address.
    pub fn destination(&self) -> SocketAddrV4 {
        SocketAddrV4::new(self.group, self.dst_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_multicast_config_passes_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
            .with_source_port(5001)
            .with_ttl(8)
            .with_loopback(false);

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn port_zero_fails_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 0);

        let result = cfg.validate();

        assert!(matches!(result, Err(MctxError::InvalidDestinationPort)));
    }

    #[test]
    fn non_multicast_group_fails_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(192, 168, 1, 10), 5000);

        let result = cfg.validate();

        assert!(matches!(result, Err(MctxError::InvalidMulticastGroup)));
    }

    #[test]
    fn multicast_interface_fails_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
            .with_interface(Ipv4Addr::new(239, 9, 9, 9));

        let result = cfg.validate();

        assert!(matches!(result, Err(MctxError::InvalidInterfaceAddress)));
    }
}
