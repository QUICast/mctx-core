use crate::config::PublicationAddressFamily;

/// Route-selected raw egress support for one address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawRouteEgressCapability {
    /// The platform cannot faithfully provide route-selected raw egress.
    Unsupported,
    /// The platform supports unbound, unconnected raw IPv4 socket egress.
    KernelRouteSelected,
}

impl RawRouteEgressCapability {
    /// Returns whether route-selected raw egress is available.
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::KernelRouteSelected)
    }
}

/// Route-selected raw egress capabilities compiled for this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRouteEgressCapabilities {
    /// Route-selected IPv4 support.
    pub ipv4: RawRouteEgressCapability,
    /// Route-selected IPv6 support.
    pub ipv6: RawRouteEgressCapability,
}

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
pub const fn raw_route_egress_capabilities() -> RawRouteEgressCapabilities {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
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
        let capabilities = raw_route_egress_capabilities();

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(
            capabilities.ipv4,
            RawRouteEgressCapability::KernelRouteSelected
        );
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        assert_eq!(capabilities.ipv4, RawRouteEgressCapability::Unsupported);
        assert_eq!(capabilities.ipv6, RawRouteEgressCapability::Unsupported);
        assert_eq!(
            capabilities.for_family(PublicationAddressFamily::Ipv6),
            RawRouteEgressCapability::Unsupported
        );
    }
}
