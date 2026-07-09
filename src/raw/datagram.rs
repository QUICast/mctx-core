#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::config::PublicationAddressFamily;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::error::MctxError;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedRawIpDatagram {
    pub(crate) family: PublicationAddressFamily,
    pub(crate) source_ip: IpAddr,
    pub(crate) destination_ip: IpAddr,
    pub(crate) protocol: u8,
    pub(crate) header_len: usize,
    pub(crate) ttl_or_hop_limit: u8,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
pub(crate) fn parse_raw_ip_datagram(datagram: &[u8]) -> Result<ParsedRawIpDatagram, MctxError> {
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

#[cfg(any(target_os = "linux", windows, test))]
pub(crate) fn apply_ttl_or_hop_limit_override(
    datagram: &[u8],
    parsed: ParsedRawIpDatagram,
    ttl_or_hop_limit: u8,
) -> Vec<u8> {
    let mut patched = datagram.to_vec();

    match parsed.family {
        PublicationAddressFamily::Ipv4 => {
            patched[8] = ttl_or_hop_limit;
            patched[10] = 0;
            patched[11] = 0;

            let checksum = ipv4_header_checksum(&patched[..parsed.header_len]);
            let checksum_bytes = checksum.to_be_bytes();
            patched[10] = checksum_bytes[0];
            patched[11] = checksum_bytes[1];
        }
        PublicationAddressFamily::Ipv6 => {
            patched[7] = ttl_or_hop_limit;
        }
    }

    patched
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn parse_ipv4_datagram(datagram: &[u8]) -> Result<ParsedRawIpDatagram, MctxError> {
    if datagram.len() < 20 {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let ihl = ((datagram[0] & 0x0f) as usize) * 4;
    if ihl < 20 || datagram.len() < ihl {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let total_len = u16::from_be_bytes([datagram[2], datagram[3]]) as usize;
    if total_len < ihl || total_len != datagram.len() {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    Ok(ParsedRawIpDatagram {
        family: PublicationAddressFamily::Ipv4,
        source_ip: IpAddr::V4(Ipv4Addr::new(
            datagram[12],
            datagram[13],
            datagram[14],
            datagram[15],
        )),
        destination_ip: IpAddr::V4(Ipv4Addr::new(
            datagram[16],
            datagram[17],
            datagram[18],
            datagram[19],
        )),
        protocol: datagram[9],
        header_len: ihl,
        ttl_or_hop_limit: datagram[8],
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn parse_ipv6_datagram(datagram: &[u8]) -> Result<ParsedRawIpDatagram, MctxError> {
    if datagram.len() < 40 {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let payload_len = u16::from_be_bytes([datagram[4], datagram[5]]) as usize;
    let total_len = 40usize
        .checked_add(payload_len)
        .ok_or(MctxError::InvalidRawIpDatagram)?;

    if total_len != datagram.len() {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let source =
        <[u8; 16]>::try_from(&datagram[8..24]).map_err(|_| MctxError::InvalidRawIpDatagram)?;
    let destination =
        <[u8; 16]>::try_from(&datagram[24..40]).map_err(|_| MctxError::InvalidRawIpDatagram)?;

    Ok(ParsedRawIpDatagram {
        family: PublicationAddressFamily::Ipv6,
        source_ip: IpAddr::V6(Ipv6Addr::from(source)),
        destination_ip: IpAddr::V6(Ipv6Addr::from(destination)),
        protocol: datagram[6],
        header_len: 40,
        ttl_or_hop_limit: datagram[7],
    })
}

#[cfg(any(target_os = "linux", windows, test))]
fn ipv4_header_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;

    for chunk in header.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    if !header.len().is_multiple_of(2) {
        sum += (header[header.len() - 1] as u32) << 8;
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_raw_datagram_fields() {
        let datagram = [
            0x45, 0x00, 0x00, 0x1c, 0x12, 0x34, 0x00, 0x00, 0x01, 0x11, 0x00, 0x00, 10, 1, 2, 3,
            239, 1, 2, 3, 1, 2, 3, 4, 5, 6, 7, 8,
        ];

        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.family, PublicationAddressFamily::Ipv4);
        assert_eq!(parsed.source_ip, IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3))
        );
        assert_eq!(parsed.protocol, 17);
        assert_eq!(parsed.ttl_or_hop_limit, 1);
    }

    #[test]
    fn parses_ipv6_raw_datagram_fields() {
        let mut datagram = [0u8; 40];
        datagram[0] = 0x60;
        datagram[5] = 0;
        datagram[6] = 17;
        datagram[8..24].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        datagram[24..40].copy_from_slice(&"ff3e::8000:1234".parse::<Ipv6Addr>().unwrap().octets());

        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        assert_eq!(parsed.family, PublicationAddressFamily::Ipv6);
        assert_eq!(parsed.source_ip, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(
            parsed.destination_ip,
            IpAddr::V6("ff3e::8000:1234".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(parsed.protocol, 17);
        assert_eq!(parsed.ttl_or_hop_limit, 0);
    }

    #[test]
    fn malformed_raw_datagram_is_rejected() {
        assert!(matches!(
            parse_raw_ip_datagram(&[0x45, 0x00, 0x00]),
            Err(MctxError::InvalidRawIpDatagram)
        ));
        assert!(matches!(
            parse_raw_ip_datagram(&[0x70; 8]),
            Err(MctxError::InvalidRawIpDatagram)
        ));
    }

    #[test]
    fn ipv4_total_length_must_match_buffer_length() {
        let datagram = [
            0x45, 0x00, 0x00, 0x20, 0x12, 0x34, 0x00, 0x00, 0x01, 0x11, 0x00, 0x00, 10, 1, 2, 3,
            239, 1, 2, 3, 1, 2, 3, 4, 5, 6, 7, 8,
        ];

        assert!(matches!(
            parse_raw_ip_datagram(&datagram),
            Err(MctxError::InvalidRawIpDatagram)
        ));
    }

    #[test]
    fn ttl_override_updates_ipv4_header_checksum() {
        let datagram = [
            0x45, 0x00, 0x00, 0x1c, 0x12, 0x34, 0x00, 0x00, 0x01, 0x11, 0x00, 0x00, 10, 1, 2, 3,
            239, 1, 2, 3, 1, 2, 3, 4, 5, 6, 7, 8,
        ];

        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        let patched = apply_ttl_or_hop_limit_override(&datagram, parsed, 64);

        assert_eq!(patched[8], 64);
        assert_eq!(ipv4_header_checksum(&patched[..parsed.header_len]), 0);
    }
}
