use crate::error::MctxError;
use crate::raw::{RawPublication, RawPublicationConfig, RawPublicationId, RawSendReport};

/// Owns and manages the set of active raw multicast publications.
#[derive(Debug, Default)]
pub struct RawContext {
    publications: Vec<RawPublication>,
    next_publication_id: u64,
}

impl RawContext {
    /// Creates an empty raw context with no publications.
    pub fn new() -> Self {
        Self {
            publications: Vec::new(),
            next_publication_id: 1,
        }
    }

    /// Returns the number of active raw publications.
    pub fn publication_count(&self) -> usize {
        self.publications.len()
    }

    /// Returns true if a raw publication with the given ID exists.
    pub fn contains_publication(&self, id: RawPublicationId) -> bool {
        self.publications
            .iter()
            .any(|publication| publication.id() == id)
    }

    /// Returns the raw publication with the given ID, if present.
    pub fn get_publication(&self, id: RawPublicationId) -> Option<&RawPublication> {
        self.publications
            .iter()
            .find(|publication| publication.id() == id)
    }

    /// Returns the raw publication mutably with the given ID, if present.
    pub fn get_publication_mut(&mut self, id: RawPublicationId) -> Option<&mut RawPublication> {
        self.publications
            .iter_mut()
            .find(|publication| publication.id() == id)
    }

    fn ensure_publication_config_is_unique(
        &self,
        config: &RawPublicationConfig,
    ) -> Result<(), MctxError> {
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        Ok(())
    }

    /// Adds a new raw publication to the context.
    pub fn add_publication(
        &mut self,
        config: RawPublicationConfig,
    ) -> Result<RawPublicationId, MctxError> {
        self.ensure_publication_config_is_unique(&config)?;

        let id = RawPublicationId(self.next_publication_id);
        self.next_publication_id += 1;

        let publication = RawPublication::new(id, config)?;
        self.publications.push(publication);
        Ok(id)
    }

    /// Removes one raw publication and drops its socket.
    pub fn remove_publication(&mut self, id: RawPublicationId) -> bool {
        self.take_publication(id).is_some()
    }

    /// Extracts one raw publication from the context.
    pub fn take_publication(&mut self, id: RawPublicationId) -> Option<RawPublication> {
        let index = self
            .publications
            .iter()
            .position(|publication| publication.id() == id)?;

        Some(self.publications.swap_remove(index))
    }

    /// Sends one full IP datagram through the selected raw publication.
    pub fn send_raw(
        &self,
        id: RawPublicationId,
        ip_datagram: &[u8],
    ) -> Result<RawSendReport, MctxError> {
        self.get_publication(id)
            .ok_or(MctxError::PublicationNotFound)?
            .send_raw(ip_datagram)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    use crate::test_support::TEST_GROUP;
    #[cfg(target_os = "macos")]
    use crate::test_support::{recv_payload, test_multicast_receiver};
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    use std::net::{IpAddr, Ipv4Addr};

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn raw_context_requires_explicit_interface_selection_before_socket_setup() {
        let mut ctx = RawContext::new();

        let err = ctx
            .add_publication(RawPublicationConfig::ipv4())
            .unwrap_err();
        assert!(matches!(err, MctxError::RawInterfaceRequired));
    }

    #[cfg(windows)]
    #[test]
    fn windows_raw_ipv6_support_is_explicitly_unsupported() {
        let mut ctx = RawContext::new();

        let err = ctx
            .add_publication(RawPublicationConfig::ipv6().with_ipv6_interface_index(7))
            .unwrap_err();
        assert!(matches!(err, MctxError::RawPacketTransmitUnsupported(_)));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    #[test]
    fn unsupported_platforms_report_explicit_raw_transmit_unsupported_errors() {
        let mut ctx = RawContext::new();

        let err = ctx
            .add_publication(RawPublicationConfig::ipv6().with_ipv6_interface_index(7))
            .unwrap_err();
        assert!(matches!(err, MctxError::RawPacketTransmitUnsupported(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires CAP_NET_RAW and MCTX_RAW_TEST_SOURCE_V4 set to a local Ethernet IPv4 address; validates send success/report only"]
    fn linux_raw_ipv4_send_report_smoke_test() {
        run_raw_ipv4_send_report_smoke_test();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires root and MCTX_RAW_TEST_SOURCE_V4 set to a local IPv4 address"]
    fn macos_raw_ipv4_send_smoke_test() {
        run_raw_ipv4_send_smoke_test();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Administrator privileges and MCTX_RAW_TEST_SOURCE_V4 set to a local IPv4 address; validates send success/report only"]
    fn windows_raw_ipv4_send_report_smoke_test() {
        run_raw_ipv4_send_report_smoke_test();
    }

    #[cfg(target_os = "macos")]
    fn run_raw_ipv4_send_smoke_test() {
        let Some(source) = std::env::var("MCTX_RAW_TEST_SOURCE_V4")
            .ok()
            .and_then(|raw| raw.parse::<Ipv4Addr>().ok())
        else {
            return;
        };

        let (receiver, port) = test_multicast_receiver();
        let mut ctx = RawContext::new();
        let id = ctx
            .add_publication(RawPublicationConfig::ipv4().with_bind_addr(source))
            .unwrap();

        let payload = b"raw-smoke";
        let datagram = build_ipv4_udp_datagram(source, TEST_GROUP, 4000, port, payload);
        let report = ctx.send_raw(id, &datagram).unwrap();

        assert_eq!(report.source_ip, Some(IpAddr::V4(source)));
        assert_eq!(report.destination_ip, Some(IpAddr::V4(TEST_GROUP)));
        assert_eq!(recv_payload(&receiver), payload);
    }

    #[cfg(any(target_os = "linux", windows))]
    fn run_raw_ipv4_send_report_smoke_test() {
        let Some(source) = std::env::var("MCTX_RAW_TEST_SOURCE_V4")
            .ok()
            .and_then(|raw| raw.parse::<Ipv4Addr>().ok())
        else {
            return;
        };

        let mut ctx = RawContext::new();
        let id = ctx
            .add_publication(
                RawPublicationConfig::ipv4()
                    .with_bind_addr(source)
                    .with_outgoing_interface(source),
            )
            .unwrap();

        let payload = b"raw-smoke";
        let datagram = build_ipv4_udp_datagram(source, TEST_GROUP, 4000, 5000, payload);
        let report = ctx.send_raw(id, &datagram).unwrap();

        assert_eq!(report.source_ip, Some(IpAddr::V4(source)));
        assert_eq!(report.destination_ip, Some(IpAddr::V4(TEST_GROUP)));
        assert_eq!(report.ip_protocol, Some(17));
        assert_eq!(report.bytes_sent, datagram.len());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn build_ipv4_udp_datagram(
        source: Ipv4Addr,
        destination: Ipv4Addr,
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let total_len = 20 + 8 + payload.len();
        let mut datagram = vec![0u8; total_len];

        datagram[0] = 0x45;
        datagram[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        datagram[8] = 1;
        datagram[9] = 17;
        datagram[12..16].copy_from_slice(&source.octets());
        datagram[16..20].copy_from_slice(&destination.octets());

        let udp_len = (8 + payload.len()) as u16;
        datagram[20..22].copy_from_slice(&source_port.to_be_bytes());
        datagram[22..24].copy_from_slice(&destination_port.to_be_bytes());
        datagram[24..26].copy_from_slice(&udp_len.to_be_bytes());
        datagram[26..28].copy_from_slice(&0u16.to_be_bytes());
        datagram[28..].copy_from_slice(payload);

        let checksum = ipv4_header_checksum(&datagram[..20]);
        datagram[10..12].copy_from_slice(&checksum.to_be_bytes());
        datagram
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn ipv4_header_checksum(header: &[u8]) -> u16 {
        let mut sum = 0u32;

        for chunk in header.chunks_exact(2) {
            sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
        }

        while (sum >> 16) != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }

        !(sum as u16)
    }
}
