/// Raw-IP support level for one address family on the current platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawIpCapability {
    /// The platform has no supported transmit implementation for this family.
    Unsupported,
    /// The platform accepts a complete caller-provided IPv4 datagram through
    /// an `IP_HDRINCL`-style socket.
    FullIpDatagram,
    /// The platform accepts the caller-provided IPv6 payload, but its network
    /// stack rebuilds the IPv6 base header during transmit.
    ///
    /// The raw-IP API pins the source to the configured local bind address and
    /// applies the supplied traffic class and hop limit. IPv6 flow-label and
    /// transport-checksum behavior remain platform controlled.
    KernelRebuiltIpv6Header,
}

/// Raw-IP transmit support exposed by the current platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawIpCapabilities {
    /// IPv4 raw-IP transmit behavior.
    pub ipv4: RawIpCapability,
    /// IPv6 raw-IP transmit behavior.
    pub ipv6: RawIpCapability,
}

/// Returns the raw-IP capabilities compiled for the current platform.
pub const fn raw_ip_capabilities() -> RawIpCapabilities {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        RawIpCapabilities {
            ipv4: RawIpCapability::FullIpDatagram,
            ipv6: RawIpCapability::KernelRebuiltIpv6Header,
        }
    }

    #[cfg(windows)]
    {
        RawIpCapabilities {
            ipv4: RawIpCapability::FullIpDatagram,
            ipv6: RawIpCapability::Unsupported,
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        RawIpCapabilities {
            ipv4: RawIpCapability::Unsupported,
            ipv6: RawIpCapability::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_report_matches_the_compiled_platform() {
        let capabilities = raw_ip_capabilities();

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(capabilities.ipv4, RawIpCapability::FullIpDatagram);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(capabilities.ipv6, RawIpCapability::KernelRebuiltIpv6Header);

        #[cfg(windows)]
        assert_eq!(capabilities.ipv4, RawIpCapability::FullIpDatagram);
        #[cfg(windows)]
        assert_eq!(capabilities.ipv6, RawIpCapability::Unsupported);
    }
}
