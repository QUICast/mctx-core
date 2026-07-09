use crate::config::{OutgoingInterface, PublicationAddressFamily};
use crate::error::MctxError;
use std::net::IpAddr;

/// Validation behavior applied to outbound raw IP datagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RawValidationMode {
    /// Require the parsed destination IP address to be multicast.
    #[default]
    StrictMulticastDestination,
    /// Allow non-multicast destinations through validation.
    ///
    /// Individual platform backends can still return an explicit unsupported
    /// error when they cannot route a non-multicast raw datagram faithfully.
    AllowAnyDestination,
}

/// Configuration for one raw multicast transmit publication.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawPublicationConfig {
    /// The expected IP family for outbound datagrams, if fixed in advance.
    /// Otherwise it is inferred from the local bind or interface selector when
    /// the publication is created.
    pub family: Option<PublicationAddressFamily>,
    /// The explicit egress interface selector, if set.
    pub outgoing_interface: Option<OutgoingInterface>,
    /// The local IP address used to select and validate the egress interface.
    ///
    /// The source IP seen by receivers comes from the supplied datagram. This
    /// field only identifies a local egress address. A matching IPv6 source can
    /// use the host IP stack; a distinct source requires a link-layer backend.
    pub bind_addr: Option<IpAddr>,
    /// Optional TTL or hop-limit override applied during transmit.
    pub ttl: Option<u8>,
    /// Optional loopback preference.
    pub loopback: Option<bool>,
    /// Validation behavior for outbound datagrams.
    pub validation_mode: RawValidationMode,
}

impl Default for RawPublicationConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl RawPublicationConfig {
    /// Creates a raw publication config with family inferred from its local
    /// bind or outgoing-interface selector.
    pub fn new() -> Self {
        Self {
            family: None,
            outgoing_interface: None,
            bind_addr: None,
            ttl: None,
            loopback: None,
            validation_mode: RawValidationMode::StrictMulticastDestination,
        }
    }

    /// Creates a config fixed to IPv4 datagrams.
    pub fn ipv4() -> Self {
        Self::new().with_family(PublicationAddressFamily::Ipv4)
    }

    /// Creates a config fixed to IPv6 datagrams.
    pub fn ipv6() -> Self {
        Self::new().with_family(PublicationAddressFamily::Ipv6)
    }

    /// Validates the configuration and returns an error if it is not usable.
    pub fn validate(&self) -> Result<(), MctxError> {
        if self.bind_addr.is_none() && self.outgoing_interface.is_none() {
            return Err(MctxError::RawInterfaceRequired);
        }

        if let Some(bind_addr) = self.bind_addr {
            if bind_addr.is_multicast() || bind_addr.is_unspecified() {
                return Err(MctxError::InvalidRawBindAddress);
            }

            if let Some(family) = self.family
                && !family_matches_ip(family, bind_addr)
            {
                return Err(MctxError::RawBindAddressFamilyMismatch);
            }
        }

        if let Some(outgoing_interface) = self.outgoing_interface {
            match outgoing_interface {
                OutgoingInterface::Ipv4Addr(interface) => {
                    if interface.is_multicast() || interface.is_unspecified() {
                        return Err(MctxError::InvalidInterfaceAddress);
                    }

                    if matches!(self.family, Some(PublicationAddressFamily::Ipv6)) {
                        return Err(MctxError::OutgoingInterfaceFamilyMismatch);
                    }
                }
                OutgoingInterface::Ipv6Addr(interface) => {
                    if interface.is_multicast() || interface.is_unspecified() {
                        return Err(MctxError::InvalidInterfaceAddress);
                    }

                    if matches!(self.family, Some(PublicationAddressFamily::Ipv4)) {
                        return Err(MctxError::OutgoingInterfaceFamilyMismatch);
                    }
                }
                OutgoingInterface::Ipv6Index(index) => {
                    if index == 0 {
                        return Err(MctxError::InvalidIpv6InterfaceIndex);
                    }

                    if matches!(self.family, Some(PublicationAddressFamily::Ipv4)) {
                        return Err(MctxError::OutgoingInterfaceFamilyMismatch);
                    }
                }
            }

            if let Some(bind_addr) = self.bind_addr
                && !interface_matches_ip(outgoing_interface, bind_addr)
            {
                return Err(MctxError::OutgoingInterfaceFamilyMismatch);
            }
        }

        Ok(())
    }

    /// Pins the expected IP family for datagrams sent through this publication.
    pub fn with_family(mut self, family: PublicationAddressFamily) -> Self {
        self.family = Some(family);
        self
    }

    /// Sets the outgoing interface selector.
    pub fn with_outgoing_interface(
        mut self,
        outgoing_interface: impl Into<OutgoingInterface>,
    ) -> Self {
        self.outgoing_interface = Some(outgoing_interface.into());
        self
    }

    /// Sets the IPv4-oriented interface convenience selector.
    pub fn with_interface(self, interface: std::net::Ipv4Addr) -> Self {
        self.with_outgoing_interface(interface)
    }

    /// Sets the IPv6 interface selector by interface index.
    pub fn with_ipv6_interface_index(mut self, interface_index: u32) -> Self {
        self.outgoing_interface = Some(OutgoingInterface::Ipv6Index(interface_index));
        self
    }

    /// Sets the local IP address used to select and validate the egress interface.
    pub fn with_bind_addr(mut self, bind_addr: impl Into<IpAddr>) -> Self {
        self.bind_addr = Some(bind_addr.into());
        self
    }

    /// Sets an optional TTL or hop-limit override.
    pub fn with_ttl(mut self, ttl: u8) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Requests an explicit loopback preference.
    pub fn with_loopback(mut self, loopback: bool) -> Self {
        self.loopback = Some(loopback);
        self
    }

    /// Adjusts outbound datagram validation behavior.
    pub fn with_validation_mode(mut self, validation_mode: RawValidationMode) -> Self {
        self.validation_mode = validation_mode;
        self
    }
}

fn family_matches_ip(family: PublicationAddressFamily, ip: IpAddr) -> bool {
    matches!(
        (family, ip),
        (PublicationAddressFamily::Ipv4, IpAddr::V4(_))
            | (PublicationAddressFamily::Ipv6, IpAddr::V6(_))
    )
}

fn interface_matches_ip(interface: OutgoingInterface, ip: IpAddr) -> bool {
    matches!(
        (interface, ip),
        (OutgoingInterface::Ipv4Addr(_), IpAddr::V4(_))
            | (
                OutgoingInterface::Ipv6Addr(_) | OutgoingInterface::Ipv6Index(_),
                IpAddr::V6(_)
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn valid_ipv4_raw_config_passes_validation() {
        let cfg = RawPublicationConfig::ipv4()
            .with_bind_addr(Ipv4Addr::new(192, 168, 1, 20))
            .with_outgoing_interface(Ipv4Addr::new(192, 168, 1, 20))
            .with_ttl(8);

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn valid_ipv6_raw_config_passes_validation() {
        let cfg = RawPublicationConfig::ipv6()
            .with_bind_addr("2001:db8::10".parse::<Ipv6Addr>().unwrap())
            .with_ipv6_interface_index(7)
            .with_validation_mode(RawValidationMode::AllowAnyDestination);

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn raw_bind_address_must_be_unicast() {
        let cfg = RawPublicationConfig::new().with_bind_addr(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        assert!(matches!(
            cfg.validate(),
            Err(MctxError::InvalidRawBindAddress)
        ));
    }

    #[test]
    fn raw_bind_address_family_must_match_config_family() {
        let cfg = RawPublicationConfig::ipv6().with_bind_addr(Ipv4Addr::new(10, 0, 0, 1));

        assert!(matches!(
            cfg.validate(),
            Err(MctxError::RawBindAddressFamilyMismatch)
        ));
    }

    #[test]
    fn ipv4_raw_config_rejects_ipv6_interface_index() {
        let cfg = RawPublicationConfig::ipv4().with_ipv6_interface_index(7);

        assert!(matches!(
            cfg.validate(),
            Err(MctxError::OutgoingInterfaceFamilyMismatch)
        ));
    }

    #[test]
    fn inferred_family_rejects_mismatched_bind_and_interface() {
        let cfg = RawPublicationConfig::new()
            .with_bind_addr(Ipv4Addr::new(10, 0, 0, 1))
            .with_ipv6_interface_index(7);

        assert!(matches!(
            cfg.validate(),
            Err(MctxError::OutgoingInterfaceFamilyMismatch)
        ));
    }

    #[test]
    fn raw_config_requires_an_egress_selector() {
        assert!(matches!(
            RawPublicationConfig::ipv4().validate(),
            Err(MctxError::RawInterfaceRequired)
        ));
    }
}
