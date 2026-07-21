use std::net::Ipv6Addr;

#[cfg(any(target_os = "macos", test))]
pub(crate) const ETHERNET_IPV6_HEADER_LEN: usize = 14;

pub(crate) fn ipv6_multicast_mac(group: Ipv6Addr) -> [u8; 6] {
    debug_assert!(group.is_multicast());
    let octets = group.octets();
    [0x33, 0x33, octets[12], octets[13], octets[14], octets[15]]
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn ethernet_ipv6_header(group: Ipv6Addr, source_mac: [u8; 6]) -> [u8; 14] {
    let mut header = [0u8; ETHERNET_IPV6_HEADER_LEN];
    header[..6].copy_from_slice(&ipv6_multicast_mac(group));
    header[6..12].copy_from_slice(&source_mac);
    header[12..14].copy_from_slice(&0x86ddu16.to_be_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_ipv6_multicast_destination_mac() {
        assert_eq!(
            ipv6_multicast_mac("ff3e::8000:1234".parse().unwrap()),
            [0x33, 0x33, 0x80, 0x00, 0x12, 0x34]
        );
    }

    #[test]
    fn builds_complete_ethernet_ipv6_header() {
        let header = ethernet_ipv6_header(
            "ff3e::8000:1234".parse().unwrap(),
            [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
        );

        assert_eq!(&header[..6], &[0x33, 0x33, 0x80, 0x00, 0x12, 0x34]);
        assert_eq!(&header[6..12], &[0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
        assert_eq!(&header[12..], &[0x86, 0xdd]);
    }
}
