use crate::error::MctxError;
use crate::raw::{RawPublication, RawPublicationConfig, RawPublicationId, RawSendReport};
use std::collections::HashMap;

/// Owns and manages the set of active raw multicast publications.
#[derive(Debug, Default)]
pub struct RawContext {
    publications: Vec<RawPublication>,
    publication_indices: HashMap<RawPublicationId, usize>,
    next_publication_id: u64,
}

impl RawContext {
    /// Creates an empty raw context with no publications.
    pub fn new() -> Self {
        Self {
            publications: Vec::new(),
            publication_indices: HashMap::new(),
            next_publication_id: 1,
        }
    }

    /// Returns the number of active raw publications.
    pub fn publication_count(&self) -> usize {
        self.publications.len()
    }

    /// Returns true if a raw publication with the given ID exists.
    pub fn contains_publication(&self, id: RawPublicationId) -> bool {
        self.publication_indices.contains_key(&id)
    }

    /// Returns the raw publication with the given ID, if present.
    pub fn get_publication(&self, id: RawPublicationId) -> Option<&RawPublication> {
        let index = *self.publication_indices.get(&id)?;
        self.publications.get(index)
    }

    /// Returns the raw publication mutably with the given ID, if present.
    pub fn get_publication_mut(&mut self, id: RawPublicationId) -> Option<&mut RawPublication> {
        let index = *self.publication_indices.get(&id)?;
        self.publications.get_mut(index)
    }

    fn ensure_publication_config_is_unique(
        &self,
        config: &RawPublicationConfig,
        excluded_id: Option<RawPublicationId>,
    ) -> Result<(), MctxError> {
        if self.publications.iter().any(|publication| {
            Some(publication.id()) != excluded_id && publication.config() == config
        }) {
            return Err(MctxError::DuplicatePublication);
        }

        Ok(())
    }

    /// Adds a new raw publication to the context.
    pub fn add_publication(
        &mut self,
        config: RawPublicationConfig,
    ) -> Result<RawPublicationId, MctxError> {
        self.ensure_publication_config_is_unique(&config, None)?;

        let id = RawPublicationId(self.next_publication_id);
        self.next_publication_id += 1;

        let publication = RawPublication::new(id, config)?;
        let index = self.publications.len();
        self.publications.push(publication);
        self.publication_indices.insert(id, index);
        Ok(id)
    }

    /// Replaces one publication after fully initializing its new socket.
    ///
    /// The publication ID and context index are preserved. If validation,
    /// duplicate detection, or socket initialization fails, the original
    /// publication remains untouched and usable. Raw publications currently
    /// have no built-in metrics counters; caller-side metrics keyed by the
    /// stable ID can therefore continue across replacement.
    pub fn replace_publication(
        &mut self,
        id: RawPublicationId,
        config: RawPublicationConfig,
    ) -> Result<(), MctxError> {
        let index = *self
            .publication_indices
            .get(&id)
            .ok_or(MctxError::PublicationNotFound)?;
        self.ensure_publication_config_is_unique(&config, Some(id))?;

        let replacement = RawPublication::new(id, config)?;
        self.publications[index] = replacement;
        Ok(())
    }

    /// Removes one raw publication and drops its socket.
    pub fn remove_publication(&mut self, id: RawPublicationId) -> bool {
        self.take_publication(id).is_some()
    }

    /// Extracts one raw publication from the context.
    pub fn take_publication(&mut self, id: RawPublicationId) -> Option<RawPublication> {
        let index = self.publication_indices.remove(&id)?;
        let publication = self.publications.swap_remove(index);
        if index < self.publications.len() {
            let moved_id = self.publications[index].id();
            self.publication_indices.insert(moved_id, index);
        }

        Some(publication)
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
    use crate::test_support::recv_payload;
    #[cfg(target_os = "macos")]
    use crate::test_support::test_multicast_receiver;
    #[cfg(target_os = "macos")]
    use std::net::Ipv6Addr;
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

    #[cfg(target_os = "linux")]
    #[test]
    fn raw_context_updates_id_lookup_after_swap_remove() {
        let mut ctx = RawContext::new();
        let first = ctx
            .add_publication(RawPublicationConfig::ipv6().with_ipv6_interface_index(7))
            .unwrap();
        let second = ctx
            .add_publication(RawPublicationConfig::ipv6().with_ipv6_interface_index(8))
            .unwrap();

        assert!(ctx.remove_publication(first));
        assert!(!ctx.contains_publication(first));
        assert!(ctx.contains_publication(second));
        assert_eq!(
            ctx.get_publication(second).map(RawPublication::id),
            Some(second)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replacement_preserves_publication_id_and_count() {
        let mut ctx = RawContext::new();
        let id = ctx
            .add_publication(RawPublicationConfig::ipv6().with_ipv6_interface_index(7))
            .unwrap();

        ctx.replace_publication(
            id,
            RawPublicationConfig::ipv6().with_ipv6_interface_index(8),
        )
        .unwrap();

        let publication = ctx.get_publication(id).unwrap();
        assert_eq!(publication.id(), id);
        assert_eq!(
            publication.config().outgoing_interface,
            Some(crate::OutgoingInterface::Ipv6Index(8))
        );
        assert_eq!(ctx.publication_count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_replacement_leaves_original_publication_usable() {
        let mut ctx = RawContext::new();
        let original_config = RawPublicationConfig::ipv6().with_ipv6_interface_index(7);
        let id = ctx.add_publication(original_config.clone()).unwrap();

        let error = ctx
            .replace_publication(id, RawPublicationConfig::ipv4())
            .unwrap_err();

        assert!(matches!(error, MctxError::RawInterfaceRequired));
        let publication = ctx.get_publication(id).unwrap();
        assert_eq!(publication.id(), id);
        assert_eq!(publication.config(), &original_config);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn replacement_rejects_another_publications_config_transactionally() {
        let mut ctx = RawContext::new();
        let original_config = RawPublicationConfig::ipv6().with_ipv6_interface_index(7);
        let duplicate_config = RawPublicationConfig::ipv6().with_ipv6_interface_index(8);
        let id = ctx.add_publication(original_config.clone()).unwrap();
        ctx.add_publication(duplicate_config.clone()).unwrap();

        let error = ctx.replace_publication(id, duplicate_config).unwrap_err();

        assert!(matches!(error, MctxError::DuplicatePublication));
        assert_eq!(ctx.get_publication(id).unwrap().config(), &original_config);
    }

    #[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
    #[test]
    fn failed_route_selected_replacement_preserves_the_original_publication() {
        let mut ctx = RawContext::new();
        let original_config = RawPublicationConfig::ipv6().with_ipv6_interface_index(7);
        let id = ctx.add_publication(original_config.clone()).unwrap();

        let error = ctx
            .replace_publication(
                id,
                RawPublicationConfig::ipv6()
                    .with_route_selected_egress()
                    .with_loopback(true),
            )
            .unwrap_err();

        assert!(matches!(error, MctxError::RawPacketTransmitUnsupported(_)));
        assert_eq!(ctx.get_publication(id).unwrap().config(), &original_config);
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

    #[cfg(all(windows, feature = "raw-route-egress"))]
    #[test]
    fn windows_route_selected_egress_is_explicitly_unsupported() {
        let mut ctx = RawContext::new();

        let err = ctx
            .add_publication(RawPublicationConfig::ipv4().with_route_selected_egress())
            .unwrap_err();

        assert!(matches!(err, MctxError::RawPacketTransmitUnsupported(_)));
        assert_eq!(ctx.publication_count(), 0);
    }

    #[cfg(all(target_os = "macos", feature = "raw-route-egress"))]
    #[test]
    fn macos_route_selected_ipv6_is_explicitly_unsupported() {
        let mut ctx = RawContext::new();

        let error = ctx
            .add_publication(RawPublicationConfig::ipv6().with_route_selected_egress())
            .unwrap_err();

        assert!(matches!(error, MctxError::RawPacketTransmitUnsupported(_)));
        assert_eq!(ctx.publication_count(), 0);
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
    #[ignore = "requires CAP_NET_RAW and MCTX_RAW_TEST_SOURCE_V4 set to a local IPv4 address; validates send success/report only"]
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

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires root plus MCTX_RAW_TEST_INTERFACE_V6 and MCTX_RAW_TEST_SOURCE_V6; validates BPF full-header send/report only"]
    fn macos_bpf_ipv6_full_header_send_smoke_test() {
        let Some(interface) = std::env::var("MCTX_RAW_TEST_INTERFACE_V6")
            .ok()
            .and_then(|raw| raw.parse::<Ipv6Addr>().ok())
        else {
            return;
        };
        let Some(source) = std::env::var("MCTX_RAW_TEST_SOURCE_V6")
            .ok()
            .and_then(|raw| raw.parse::<Ipv6Addr>().ok())
        else {
            return;
        };
        let group = "ff3e::8000:1234".parse::<Ipv6Addr>().unwrap();
        let mut ctx = RawContext::new();
        let id = ctx
            .add_publication(
                RawPublicationConfig::ipv6()
                    .with_outgoing_interface(interface)
                    .with_loopback(false),
            )
            .unwrap();
        let datagram = build_ipv6_udp_datagram(source, group, 4000, 5000, b"macos-bpf-v6");

        let report = ctx.send_raw(id, &datagram).unwrap();

        assert_eq!(report.source_ip, Some(IpAddr::V6(source)));
        assert_eq!(report.destination_ip, Some(IpAddr::V6(group)));
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

    #[cfg(target_os = "macos")]
    fn build_ipv6_udp_datagram(
        source: Ipv6Addr,
        destination: Ipv6Addr,
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let payload_len = 8 + payload.len();
        let mut datagram = vec![0u8; 40 + payload_len];

        datagram[0] = 0x60;
        datagram[4..6].copy_from_slice(&(payload_len as u16).to_be_bytes());
        datagram[6] = 17;
        datagram[7] = 1;
        datagram[8..24].copy_from_slice(&source.octets());
        datagram[24..40].copy_from_slice(&destination.octets());
        datagram[40..42].copy_from_slice(&source_port.to_be_bytes());
        datagram[42..44].copy_from_slice(&destination_port.to_be_bytes());
        datagram[44..46].copy_from_slice(&(payload_len as u16).to_be_bytes());
        datagram[46..48].copy_from_slice(&0u16.to_be_bytes());
        datagram[48..].copy_from_slice(payload);

        let checksum = udp_checksum_v6(source, destination, &datagram[40..]);
        datagram[46..48].copy_from_slice(&checksum.to_be_bytes());
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

    #[cfg(target_os = "macos")]
    fn udp_checksum_v6(source: Ipv6Addr, destination: Ipv6Addr, udp_packet: &[u8]) -> u16 {
        let mut pseudo = Vec::with_capacity(40 + udp_packet.len() + (udp_packet.len() % 2));
        pseudo.extend_from_slice(&source.octets());
        pseudo.extend_from_slice(&destination.octets());
        pseudo.extend_from_slice(&(udp_packet.len() as u32).to_be_bytes());
        pseudo.extend_from_slice(&[0, 0, 0, 17]);
        pseudo.extend_from_slice(udp_packet);

        let checksum = ones_complement_checksum(&pseudo);
        if checksum == 0 { 0xffff } else { checksum }
    }

    #[cfg(target_os = "macos")]
    fn ones_complement_checksum(bytes: &[u8]) -> u16 {
        let mut sum = 0u32;

        for chunk in bytes.chunks_exact(2) {
            sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
        }

        if !bytes.len().is_multiple_of(2) {
            sum += (bytes[bytes.len() - 1] as u32) << 8;
        }

        while (sum >> 16) != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }

        !(sum as u16)
    }
}
