use crate::config::PublicationAddressFamily;
use crate::error::MctxError;
use std::net::IpAddr;

/// Configuration for one generic raw-IP transmit socket.
///
/// An egress selector is mandatory. Supplying a local bind address or an
/// interface address resolves a concrete interface index; supplying only an
/// interface index requires an explicit address family.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawIpSocketConfig {
    /// Expected IP family. It is inferred from an IP-address selector when
    /// omitted, but is required for an index-only configuration.
    pub family: Option<PublicationAddressFamily>,
    /// Optional exact local address used to bind the raw socket.
    pub bind_addr: Option<IpAddr>,
    /// Optional local address identifying the required egress interface.
    pub interface_addr: Option<IpAddr>,
    /// Optional required egress interface index.
    pub interface_index: Option<u32>,
}

/// Compatibility name for callers that use the publication terminology.
pub type RawIpPublicationConfig = RawIpSocketConfig;

impl Default for RawIpSocketConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl RawIpSocketConfig {
    /// Creates a generic raw-IP socket configuration with inferred family.
    pub fn new() -> Self {
        Self {
            family: None,
            bind_addr: None,
            interface_addr: None,
            interface_index: None,
        }
    }

    /// Creates an IPv4 raw-IP socket configuration.
    pub fn ipv4() -> Self {
        Self::new().with_family(PublicationAddressFamily::Ipv4)
    }

    /// Creates an IPv6 raw-IP socket configuration.
    pub fn ipv6() -> Self {
        Self::new().with_family(PublicationAddressFamily::Ipv6)
    }

    /// Pins the expected family for supplied datagrams.
    pub fn with_family(mut self, family: PublicationAddressFamily) -> Self {
        self.family = Some(family);
        self
    }

    /// Binds the raw socket to this exact local IP address when the platform
    /// supports it.
    pub fn with_bind_addr(mut self, bind_addr: impl Into<IpAddr>) -> Self {
        self.bind_addr = Some(bind_addr.into());
        self
    }

    /// Selects the required egress interface by one of its local addresses.
    pub fn with_interface_addr(mut self, interface_addr: impl Into<IpAddr>) -> Self {
        self.interface_addr = Some(interface_addr.into());
        self
    }

    /// Selects the required egress interface by its operating-system index.
    pub fn with_interface_index(mut self, interface_index: u32) -> Self {
        self.interface_index = Some(interface_index);
        self
    }

    /// Validates the structural configuration before privileged socket setup.
    pub fn validate(&self) -> Result<(), MctxError> {
        if self.bind_addr.is_none()
            && self.interface_addr.is_none()
            && self.interface_index.is_none()
        {
            return Err(MctxError::RawInterfaceRequired);
        }

        if self.interface_index == Some(0) {
            return Err(MctxError::RawInterfaceRequired);
        }

        if let Some(bind_addr) = self.bind_addr {
            validate_unicast_selector(bind_addr, MctxError::InvalidRawBindAddress)?;
            if let Some(family) = self.family
                && !family_matches_ip(family, bind_addr)
            {
                return Err(MctxError::RawBindAddressFamilyMismatch);
            }
        }

        if let Some(interface_addr) = self.interface_addr {
            validate_unicast_selector(interface_addr, MctxError::InvalidInterfaceAddress)?;
            if let Some(family) = self.family
                && !family_matches_ip(family, interface_addr)
            {
                return Err(MctxError::OutgoingInterfaceFamilyMismatch);
            }
        }

        if let (Some(bind_addr), Some(interface_addr)) = (self.bind_addr, self.interface_addr)
            && ip_family(bind_addr) != ip_family(interface_addr)
        {
            return Err(MctxError::OutgoingInterfaceFamilyMismatch);
        }

        if self.family.is_none() && self.bind_addr.is_none() && self.interface_addr.is_none() {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "raw IP interface-index selection requires an explicit address family".to_string(),
            ));
        }

        Ok(())
    }

    pub(crate) fn resolved_family(&self) -> Result<PublicationAddressFamily, MctxError> {
        self.validate()?;
        self.family
            .or_else(|| self.bind_addr.map(ip_family))
            .or_else(|| self.interface_addr.map(ip_family))
            .ok_or_else(|| {
                MctxError::RawPacketTransmitUnsupported(
                    "raw IP socket address family could not be inferred".to_string(),
                )
            })
    }
}

fn validate_unicast_selector(ip: IpAddr, error: MctxError) -> Result<(), MctxError> {
    if ip.is_multicast() || ip.is_unspecified() {
        return Err(error);
    }

    Ok(())
}

pub(crate) fn ip_family(ip: IpAddr) -> PublicationAddressFamily {
    match ip {
        IpAddr::V4(_) => PublicationAddressFamily::Ipv4,
        IpAddr::V6(_) => PublicationAddressFamily::Ipv6,
    }
}

pub(crate) fn family_matches_ip(family: PublicationAddressFamily, ip: IpAddr) -> bool {
    matches!(
        (family, ip),
        (PublicationAddressFamily::Ipv4, IpAddr::V4(_))
            | (PublicationAddressFamily::Ipv6, IpAddr::V6(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn address_selector_infers_family() {
        let config = RawIpSocketConfig::new().with_interface_addr(Ipv4Addr::new(192, 0, 2, 10));

        assert_eq!(
            config.resolved_family().unwrap(),
            PublicationAddressFamily::Ipv4
        );
    }

    #[test]
    fn index_only_selector_requires_family() {
        let config = RawIpSocketConfig::new().with_interface_index(7);

        assert!(matches!(
            config.validate(),
            Err(MctxError::RawPacketTransmitUnsupported(_))
        ));
    }

    #[test]
    fn zero_interface_index_is_rejected_explicitly() {
        let config = RawIpSocketConfig::ipv4().with_interface_index(0);

        assert!(matches!(
            config.validate(),
            Err(MctxError::RawInterfaceRequired)
        ));
    }

    #[test]
    fn mismatched_selectors_are_rejected() {
        let config = RawIpSocketConfig::ipv6()
            .with_bind_addr(Ipv6Addr::LOCALHOST)
            .with_interface_addr(Ipv4Addr::LOCALHOST);

        assert!(matches!(
            config.validate(),
            Err(MctxError::OutgoingInterfaceFamilyMismatch)
        ));
    }

    #[test]
    fn configuration_requires_a_concrete_egress_selector() {
        assert!(matches!(
            RawIpSocketConfig::ipv4().validate(),
            Err(MctxError::RawInterfaceRequired)
        ));
    }
}
