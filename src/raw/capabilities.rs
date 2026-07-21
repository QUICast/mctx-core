#[cfg(feature = "raw-route-egress")]
use crate::config::PublicationAddressFamily;

/// Source-preservation level for one IPv6 raw multicast egress mode.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawIpv6EgressCapability {
    /// The platform cannot faithfully transmit IPv6 in this egress mode.
    Unsupported,
    /// The platform can transmit only when the supplied source is local.
    LocalSourceOnly,
    /// An explicit Ethernet-like interface preserves the complete IPv6 packet.
    ExplicitInterfaceFullHeader,
    /// Routing selects an Ethernet-like interface while preserving the packet.
    RouteSelectedFullHeader,
}

impl RawIpv6EgressCapability {
    /// Returns whether some IPv6 transmission is available in this mode.
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }

    /// Returns whether an arbitrary supplied source and complete header survive.
    pub const fn preserves_full_header(self) -> bool {
        matches!(
            self,
            Self::ExplicitInterfaceFullHeader | Self::RouteSelectedFullHeader
        )
    }
}

/// IPv6 raw multicast capabilities compiled for this platform.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawIpv6EgressCapabilities {
    /// Behavior when the caller provides an explicit interface selector.
    pub explicit_interface: RawIpv6EgressCapability,
    /// Behavior when the routing table selects the interface.
    pub route_selected: RawIpv6EgressCapability,
}

/// Reports source-preserving IPv6 raw multicast support compiled for this
/// platform.
///
/// Linux route-selected support is reported only when `raw-route-egress` is
/// enabled. Runtime permission and link-type checks can still reject setup or
/// send operations.
pub const fn raw_ipv6_egress_capabilities() -> RawIpv6EgressCapabilities {
    #[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
    {
        RawIpv6EgressCapabilities {
            explicit_interface: RawIpv6EgressCapability::ExplicitInterfaceFullHeader,
            route_selected: RawIpv6EgressCapability::RouteSelectedFullHeader,
        }
    }

    #[cfg(all(target_os = "linux", not(feature = "raw-route-egress")))]
    {
        RawIpv6EgressCapabilities {
            explicit_interface: RawIpv6EgressCapability::ExplicitInterfaceFullHeader,
            route_selected: RawIpv6EgressCapability::Unsupported,
        }
    }

    #[cfg(target_os = "macos")]
    {
        RawIpv6EgressCapabilities {
            explicit_interface: RawIpv6EgressCapability::ExplicitInterfaceFullHeader,
            route_selected: RawIpv6EgressCapability::Unsupported,
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        RawIpv6EgressCapabilities {
            explicit_interface: RawIpv6EgressCapability::Unsupported,
            route_selected: RawIpv6EgressCapability::Unsupported,
        }
    }
}

/// Route-selected raw egress support for one address family.
#[cfg(feature = "raw-route-egress")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawRouteEgressCapability {
    /// The platform cannot faithfully provide route-selected raw egress.
    Unsupported,
    /// The platform has a faithful native route-selected backend for this family.
    KernelRouteSelected,
}

#[cfg(feature = "raw-route-egress")]
impl RawRouteEgressCapability {
    /// Returns whether route-selected raw egress is available.
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::KernelRouteSelected)
    }
}

/// Route-selected raw egress capabilities compiled for this platform.
#[cfg(feature = "raw-route-egress")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRouteEgressCapabilities {
    /// Route-selected IPv4 support.
    pub ipv4: RawRouteEgressCapability,
    /// Route-selected IPv6 support.
    pub ipv6: RawRouteEgressCapability,
}

#[cfg(feature = "raw-route-egress")]
impl RawRouteEgressCapabilities {
    /// Returns the capability for one IP address family.
    pub const fn for_family(self, family: PublicationAddressFamily) -> RawRouteEgressCapability {
        match family {
            PublicationAddressFamily::Ipv4 => self.ipv4,
            PublicationAddressFamily::Ipv6 => self.ipv6,
        }
    }
}

/// Reports route-selected raw egress support for the current platform.
#[cfg(feature = "raw-route-egress")]
pub const fn raw_route_egress_capabilities() -> RawRouteEgressCapabilities {
    #[cfg(target_os = "linux")]
    {
        RawRouteEgressCapabilities {
            ipv4: RawRouteEgressCapability::KernelRouteSelected,
            ipv6: RawRouteEgressCapability::KernelRouteSelected,
        }
    }

    #[cfg(target_os = "macos")]
    {
        RawRouteEgressCapabilities {
            ipv4: RawRouteEgressCapability::KernelRouteSelected,
            ipv6: RawRouteEgressCapability::Unsupported,
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        RawRouteEgressCapabilities {
            ipv4: RawRouteEgressCapability::Unsupported,
            ipv6: RawRouteEgressCapability::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_report_matches_platform_support() {
        let ipv6 = raw_ipv6_egress_capabilities();

        #[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
        assert_eq!(
            ipv6.route_selected,
            RawIpv6EgressCapability::RouteSelectedFullHeader
        );
        #[cfg(all(target_os = "linux", not(feature = "raw-route-egress")))]
        assert_eq!(ipv6.route_selected, RawIpv6EgressCapability::Unsupported);
        #[cfg(target_os = "macos")]
        assert_eq!(
            ipv6.explicit_interface,
            RawIpv6EgressCapability::ExplicitInterfaceFullHeader
        );
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        assert_eq!(
            ipv6.explicit_interface,
            RawIpv6EgressCapability::Unsupported
        );

        #[cfg(feature = "raw-route-egress")]
        let capabilities = raw_route_egress_capabilities();

        #[cfg(all(
            feature = "raw-route-egress",
            any(target_os = "linux", target_os = "macos")
        ))]
        assert_eq!(
            capabilities.ipv4,
            RawRouteEgressCapability::KernelRouteSelected
        );
        #[cfg(all(
            feature = "raw-route-egress",
            not(any(target_os = "linux", target_os = "macos"))
        ))]
        assert_eq!(capabilities.ipv4, RawRouteEgressCapability::Unsupported);
        #[cfg(all(feature = "raw-route-egress", target_os = "linux"))]
        assert_eq!(
            capabilities.for_family(PublicationAddressFamily::Ipv6),
            RawRouteEgressCapability::KernelRouteSelected
        );
    }
}
