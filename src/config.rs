use crate::error::MctxError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// The address family used by one publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicationAddressFamily {
    Ipv4,
    Ipv6,
}

/// Explicit outgoing multicast interface selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutgoingInterface {
    /// Select the IPv4 multicast egress interface by local IPv4 address.
    Ipv4Addr(Ipv4Addr),
    /// Select the IPv6 multicast egress interface by local IPv6 address.
    ///
    /// On the IPv6 send path this also provides the exact local address to bind
    /// when no explicit source address was configured.
    Ipv6Addr(Ipv6Addr),
    /// Select the IPv6 multicast egress interface by interface index.
    Ipv6Index(u32),
}

impl From<Ipv4Addr> for OutgoingInterface {
    fn from(value: Ipv4Addr) -> Self {
        Self::Ipv4Addr(value)
    }
}

impl From<Ipv6Addr> for OutgoingInterface {
    fn from(value: Ipv6Addr) -> Self {
        Self::Ipv6Addr(value)
    }
}

/// The multicast scope encoded in an IPv6 group address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv6MulticastScope {
    InterfaceLocal,
    LinkLocal,
    RealmLocal,
    AdminLocal,
    SiteLocal,
    OrganizationLocal,
    Global,
    Other(u8),
}

/// Configuration for one multicast publication socket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicationConfig {
    /// The destination multicast group.
    pub group: IpAddr,
    /// The destination UDP port.
    pub dst_port: u16,
    /// The explicit multicast egress interface, if set.
    pub outgoing_interface: Option<OutgoingInterface>,
    /// The source UDP port to bind before sending, if explicitly set.
    pub source_port: Option<u16>,
    /// The source IP address to bind before sending, if explicitly set.
    pub source_addr: Option<IpAddr>,
    /// The multicast TTL (IPv4) or hop limit (IPv6) for transmitted packets.
    pub ttl: u32,
    /// Whether outbound multicast packets should be looped back to the local host.
    pub loopback: bool,
}

impl PublicationConfig {
    /// Creates a basic multicast publication configuration.
    pub fn new(group: impl Into<IpAddr>, port: u16) -> Self {
        Self {
            group: group.into(),
            dst_port: port,
            outgoing_interface: None,
            source_port: None,
            source_addr: None,
            ttl: 1,
            loopback: true,
        }
    }

    /// Returns the address family for this publication.
    pub fn family(&self) -> PublicationAddressFamily {
        match self.group {
            IpAddr::V4(_) => PublicationAddressFamily::Ipv4,
            IpAddr::V6(_) => PublicationAddressFamily::Ipv6,
        }
    }

    /// Returns `true` when the publication targets an IPv4 group.
    pub fn is_ipv4(&self) -> bool {
        matches!(self.family(), PublicationAddressFamily::Ipv4)
    }

    /// Returns `true` when the publication targets an IPv6 group.
    pub fn is_ipv6(&self) -> bool {
        matches!(self.family(), PublicationAddressFamily::Ipv6)
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

        if let Some(source_addr) = self.source_addr {
            if source_addr.is_multicast() || source_addr.is_unspecified() {
                return Err(MctxError::InvalidSourceAddress);
            }

            if !same_family_ip(self.group, source_addr) {
                return Err(MctxError::SourceAddressFamilyMismatch);
            }
        }

        if let Some(interface) = self.outgoing_interface {
            match (self.family(), interface) {
                (PublicationAddressFamily::Ipv4, OutgoingInterface::Ipv4Addr(interface)) => {
                    if interface.is_multicast() || interface.is_unspecified() {
                        return Err(MctxError::InvalidInterfaceAddress);
                    }
                }
                (PublicationAddressFamily::Ipv4, OutgoingInterface::Ipv6Addr(_))
                | (PublicationAddressFamily::Ipv4, OutgoingInterface::Ipv6Index(_)) => {
                    return Err(MctxError::OutgoingInterfaceFamilyMismatch);
                }
                (PublicationAddressFamily::Ipv6, OutgoingInterface::Ipv4Addr(_)) => {
                    return Err(MctxError::OutgoingInterfaceFamilyMismatch);
                }
                (PublicationAddressFamily::Ipv6, OutgoingInterface::Ipv6Addr(interface)) => {
                    if interface.is_multicast() || interface.is_unspecified() {
                        return Err(MctxError::InvalidInterfaceAddress);
                    }
                }
                (PublicationAddressFamily::Ipv6, OutgoingInterface::Ipv6Index(index)) => {
                    if index == 0 {
                        return Err(MctxError::InvalidIpv6InterfaceIndex);
                    }
                }
            }
        }

        Ok(())
    }

    /// Sets the multicast egress interface.
    pub fn with_outgoing_interface(
        mut self,
        outgoing_interface: impl Into<OutgoingInterface>,
    ) -> Self {
        self.outgoing_interface = Some(outgoing_interface.into());
        self
    }

    /// Sets the multicast egress interface using the existing IPv4-oriented
    /// convenience builder.
    pub fn with_interface(self, interface: Ipv4Addr) -> Self {
        self.with_outgoing_interface(interface)
    }

    /// Sets the IPv6 multicast egress interface by interface index.
    pub fn with_ipv6_interface_index(mut self, interface_index: u32) -> Self {
        self.outgoing_interface = Some(OutgoingInterface::Ipv6Index(interface_index));
        self
    }

    /// Sets the source UDP port.
    pub fn with_source_port(mut self, source_port: u16) -> Self {
        self.source_port = Some(source_port);
        self
    }

    /// Sets the exact local source address to bind before sending.
    pub fn with_source_addr(mut self, source_addr: impl Into<IpAddr>) -> Self {
        self.source_addr = Some(source_addr.into());
        self
    }

    /// Sets the exact local address and UDP port to bind before sending.
    pub fn with_bind_addr(mut self, bind_addr: impl Into<SocketAddr>) -> Self {
        let bind_addr = bind_addr.into();
        self.source_addr = Some(bind_addr.ip());
        self.source_port = Some(bind_addr.port());

        // Preserve an explicit IPv6 scope ID from scoped bind addresses so
        // link-local senders keep their interface identity through socket setup.
        if let SocketAddr::V6(bind_addr_v6) = bind_addr
            && bind_addr_v6.scope_id() != 0
        {
            self.outgoing_interface = Some(OutgoingInterface::Ipv6Index(bind_addr_v6.scope_id()));
        }

        self
    }

    /// Sets the multicast TTL (IPv4) or hop limit (IPv6).
    pub fn with_ttl(mut self, ttl: u32) -> Self {
        self.ttl = ttl;
        self
    }

    /// Enables or disables multicast loopback.
    pub fn with_loopback(mut self, loopback: bool) -> Self {
        self.loopback = loopback;
        self
    }

    /// Returns the multicast scope for the configured IPv6 group, if applicable.
    pub fn ipv6_scope(&self) -> Option<Ipv6MulticastScope> {
        match self.group {
            IpAddr::V6(group) => ipv6_multicast_scope(group),
            IpAddr::V4(_) => None,
        }
    }
}

fn same_family_ip(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

/// Returns `true` if the group is in `ff3x::/32`.
pub fn is_ipv6_ssm_group(group: Ipv6Addr) -> bool {
    group.is_multicast() && (group.octets()[1] & 0xf0) == 0x30
}

pub(crate) fn ipv6_multicast_scope(group: Ipv6Addr) -> Option<Ipv6MulticastScope> {
    if !group.is_multicast() {
        return None;
    }

    let scope = group.octets()[1] & 0x0f;
    Some(match scope {
        0x1 => Ipv6MulticastScope::InterfaceLocal,
        0x2 => Ipv6MulticastScope::LinkLocal,
        0x3 => Ipv6MulticastScope::RealmLocal,
        0x4 => Ipv6MulticastScope::AdminLocal,
        0x5 => Ipv6MulticastScope::SiteLocal,
        0x8 => Ipv6MulticastScope::OrganizationLocal,
        0xe => Ipv6MulticastScope::Global,
        other => Ipv6MulticastScope::Other(other),
    })
}

pub(crate) fn ipv6_destination_scope_id(group: Ipv6Addr, interface_index: u32) -> u32 {
    match ipv6_multicast_scope(group) {
        Some(Ipv6MulticastScope::InterfaceLocal | Ipv6MulticastScope::LinkLocal) => interface_index,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddrV4, SocketAddrV6};

    #[test]
    fn valid_ipv4_multicast_config_passes_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
            .with_source_port(5001)
            .with_source_addr(Ipv4Addr::new(192, 168, 10, 5))
            .with_ttl(8)
            .with_loopback(false);

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn valid_ipv6_multicast_config_passes_validation() {
        let cfg = PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>().unwrap(), 5000)
            .with_source_addr("::1".parse::<Ipv6Addr>().unwrap())
            .with_outgoing_interface("::1".parse::<Ipv6Addr>().unwrap())
            .with_ttl(4);

        assert!(cfg.validate().is_ok());
        assert!(cfg.is_ipv6());
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
    fn family_mismatched_source_fails_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
            .with_source_addr("::1".parse::<Ipv6Addr>().unwrap());

        let result = cfg.validate();

        assert!(matches!(
            result,
            Err(MctxError::SourceAddressFamilyMismatch)
        ));
    }

    #[test]
    fn family_mismatched_interface_fails_validation() {
        let cfg = PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>().unwrap(), 5000)
            .with_interface(Ipv4Addr::new(192, 168, 1, 10));

        let result = cfg.validate();

        assert!(matches!(
            result,
            Err(MctxError::OutgoingInterfaceFamilyMismatch)
        ));
    }

    #[test]
    fn unspecified_source_addr_fails_validation() {
        let cfg = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
            .with_source_addr(Ipv4Addr::UNSPECIFIED);

        let result = cfg.validate();

        assert!(matches!(result, Err(MctxError::InvalidSourceAddress)));
    }

    #[test]
    fn zero_ipv6_interface_index_fails_validation() {
        let cfg = PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>().unwrap(), 5000)
            .with_ipv6_interface_index(0);

        let result = cfg.validate();

        assert!(matches!(result, Err(MctxError::InvalidIpv6InterfaceIndex)));
    }

    #[test]
    fn bind_addr_builder_sets_source_fields_for_ipv4() {
        let bind_addr = SocketAddrV4::new(Ipv4Addr::new(10, 1, 2, 3), 5001);
        let cfg =
            PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000).with_bind_addr(bind_addr);

        assert_eq!(
            cfg.source_addr,
            Some(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)))
        );
        assert_eq!(cfg.source_port, Some(5001));
    }

    #[test]
    fn bind_addr_builder_sets_source_fields_for_ipv6() {
        let bind_addr = SocketAddrV6::new("fd00::10".parse().unwrap(), 5001, 0, 0);
        let cfg = PublicationConfig::new("ff3e::8000:1234".parse::<Ipv6Addr>().unwrap(), 5000)
            .with_bind_addr(bind_addr);

        assert_eq!(
            cfg.source_addr,
            Some(IpAddr::V6("fd00::10".parse::<Ipv6Addr>().unwrap()))
        );
        assert_eq!(cfg.source_port, Some(5001));
    }

    #[test]
    fn bind_addr_builder_preserves_ipv6_scope_as_interface_index() {
        let bind_addr = SocketAddrV6::new("fe80::1234".parse().unwrap(), 5001, 0, 7);
        let cfg = PublicationConfig::new("ff32::8000:1234".parse::<Ipv6Addr>().unwrap(), 5000)
            .with_bind_addr(bind_addr);

        assert_eq!(
            cfg.outgoing_interface,
            Some(OutgoingInterface::Ipv6Index(7))
        );
    }

    #[test]
    fn ipv6_ssm_detection_only_matches_ff3x_groups() {
        assert!(is_ipv6_ssm_group("ff31::8000:1234".parse().unwrap()));
        assert!(is_ipv6_ssm_group("ff3e::8000:1234".parse().unwrap()));
        assert!(!is_ipv6_ssm_group("ff12::1234".parse().unwrap()));
    }

    #[test]
    fn link_local_ipv6_group_keeps_interface_index_in_destination_scope() {
        let group = "ff32::8000:1234".parse::<Ipv6Addr>().unwrap();

        assert_eq!(ipv6_destination_scope_id(group, 7), 7);
    }

    #[test]
    fn wider_scope_ipv6_group_clears_destination_scope() {
        let group = "ff3e::8000:1234".parse::<Ipv6Addr>().unwrap();

        assert_eq!(ipv6_destination_scope_id(group, 7), 0);
    }
}
