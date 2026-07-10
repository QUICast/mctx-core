use crate::config::PublicationAddressFamily;
use crate::error::MctxError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Parsed metadata from a strictly validated complete IP datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedRawIpDatagram {
    pub(crate) family: PublicationAddressFamily,
    pub(crate) source_ip: IpAddr,
    pub(crate) destination_ip: IpAddr,
    pub(crate) protocol: u8,
    pub(crate) header_len: usize,
    pub(crate) ttl_or_hop_limit: u8,
    pub(crate) traffic_class: u8,
}

/// Parses a complete IP datagram without allocating.
///
/// IPv4 requires a valid header checksum, internally consistent IHL/total
/// length, and concrete source/destination addresses. IPv6 requires an exact
/// base-header payload length and concrete source/destination addresses.
pub(crate) fn parse_complete_ip_datagram(
    datagram: &[u8],
) -> Result<ParsedRawIpDatagram, MctxError> {
    let version = datagram
        .first()
        .map(|byte| byte >> 4)
        .ok_or(MctxError::InvalidRawIpDatagram)?;

    match version {
        4 => parse_ipv4_datagram(datagram),
        6 => parse_ipv6_datagram(datagram),
        _ => Err(MctxError::InvalidRawIpDatagram),
    }
}

fn parse_ipv4_datagram(datagram: &[u8]) -> Result<ParsedRawIpDatagram, MctxError> {
    if datagram.len() < 20 || datagram[0] >> 4 != 4 {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let header_len = usize::from(datagram[0] & 0x0f) * 4;
    if header_len < 20 || datagram.len() < header_len {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let total_len = usize::from(u16::from_be_bytes([datagram[2], datagram[3]]));
    if total_len != datagram.len() || total_len < header_len {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    if ipv4_header_checksum(&datagram[..header_len]) != 0 {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let source_ip = Ipv4Addr::new(datagram[12], datagram[13], datagram[14], datagram[15]);
    let destination_ip = Ipv4Addr::new(datagram[16], datagram[17], datagram[18], datagram[19]);
    if source_ip.is_unspecified() || destination_ip.is_unspecified() {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    Ok(ParsedRawIpDatagram {
        family: PublicationAddressFamily::Ipv4,
        source_ip: IpAddr::V4(source_ip),
        destination_ip: IpAddr::V4(destination_ip),
        protocol: datagram[9],
        header_len,
        ttl_or_hop_limit: datagram[8],
        traffic_class: datagram[1],
    })
}

fn parse_ipv6_datagram(datagram: &[u8]) -> Result<ParsedRawIpDatagram, MctxError> {
    if datagram.len() < 40 || datagram[0] >> 4 != 6 {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let payload_len = usize::from(u16::from_be_bytes([datagram[4], datagram[5]]));
    let total_len = 40usize
        .checked_add(payload_len)
        .ok_or(MctxError::InvalidRawIpDatagram)?;
    if total_len != datagram.len() {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let source_ip = Ipv6Addr::from(
        <[u8; 16]>::try_from(&datagram[8..24]).map_err(|_| MctxError::InvalidRawIpDatagram)?,
    );
    let destination_ip = Ipv6Addr::from(
        <[u8; 16]>::try_from(&datagram[24..40]).map_err(|_| MctxError::InvalidRawIpDatagram)?,
    );
    if source_ip.is_unspecified() || destination_ip.is_unspecified() {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    Ok(ParsedRawIpDatagram {
        family: PublicationAddressFamily::Ipv6,
        source_ip: IpAddr::V6(source_ip),
        destination_ip: IpAddr::V6(destination_ip),
        protocol: datagram[6],
        header_len: 40,
        ttl_or_hop_limit: datagram[7],
        traffic_class: ((datagram[0] & 0x0f) << 4) | (datagram[1] >> 4),
    })
}

fn ipv4_header_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in header.chunks_exact(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_ipv4_datagram_with_a_valid_header_checksum() {
        let datagram = ipv4_icmp_datagram();

        let parsed = parse_complete_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.family, PublicationAddressFamily::Ipv4);
        assert_eq!(parsed.source_ip, "192.0.2.10".parse::<IpAddr>().unwrap());
        assert_eq!(
            parsed.destination_ip,
            "198.51.100.20".parse::<IpAddr>().unwrap()
        );
        assert_eq!(parsed.protocol, 1);
        assert_eq!(parsed.header_len, 20);
        assert_eq!(parsed.ttl_or_hop_limit, 64);
    }

    #[test]
    fn rejects_ipv4_with_an_invalid_header_checksum() {
        let mut datagram = ipv4_icmp_datagram();
        datagram[8] = 1;

        assert!(matches!(
            parse_complete_ip_datagram(&datagram),
            Err(MctxError::InvalidRawIpDatagram)
        ));
    }

    #[test]
    fn rejects_declared_length_mismatches() {
        let mut ipv4 = ipv4_icmp_datagram();
        ipv4[3] -= 1;
        assert!(matches!(
            parse_complete_ip_datagram(&ipv4),
            Err(MctxError::InvalidRawIpDatagram)
        ));

        let mut ipv6 = ipv6_icmp_datagram();
        ipv6[5] -= 1;
        assert!(matches!(
            parse_complete_ip_datagram(&ipv6),
            Err(MctxError::InvalidRawIpDatagram)
        ));
    }

    #[test]
    fn parses_complete_ipv6_datagram_and_extracts_traffic_class() {
        let datagram = ipv6_icmp_datagram();

        let parsed = parse_complete_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.family, PublicationAddressFamily::Ipv6);
        assert_eq!(parsed.protocol, 58);
        assert_eq!(parsed.ttl_or_hop_limit, 32);
        assert_eq!(parsed.traffic_class, 0xab);
    }

    #[test]
    fn rejects_unspecified_header_addresses() {
        let mut datagram = ipv6_icmp_datagram();
        datagram[8..24].fill(0);

        assert!(matches!(
            parse_complete_ip_datagram(&datagram),
            Err(MctxError::InvalidRawIpDatagram)
        ));
    }

    fn ipv4_icmp_datagram() -> [u8; 28] {
        let mut datagram = [0u8; 28];
        datagram[0] = 0x45;
        datagram[2..4].copy_from_slice(&28u16.to_be_bytes());
        datagram[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        datagram[8] = 64;
        datagram[9] = 1;
        datagram[12..16].copy_from_slice(&[192, 0, 2, 10]);
        datagram[16..20].copy_from_slice(&[198, 51, 100, 20]);
        datagram[20] = 3;
        datagram[21] = 4;
        let checksum = ipv4_header_checksum(&datagram[..20]);
        datagram[10..12].copy_from_slice(&checksum.to_be_bytes());
        datagram
    }

    fn ipv6_icmp_datagram() -> [u8; 48] {
        let mut datagram = [0u8; 48];
        datagram[0] = 0x6a;
        datagram[1] = 0xb0;
        datagram[4..6].copy_from_slice(&8u16.to_be_bytes());
        datagram[6] = 58;
        datagram[7] = 32;
        datagram[8..24].copy_from_slice(&"2001:db8::10".parse::<Ipv6Addr>().unwrap().octets());
        datagram[24..40].copy_from_slice(&"2001:db8::20".parse::<Ipv6Addr>().unwrap().octets());
        datagram[40] = 2;
        datagram
    }
}
