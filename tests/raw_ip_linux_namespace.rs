#![cfg(all(target_os = "linux", feature = "raw-ip"))]

use mctx_core::{RawIpContext, RawIpSocketConfig};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::process::Command;
use std::time::Duration;

const VETH_SOURCE: &str = "mctxraw0";
const VETH_PEER: &str = "mctxraw1";
const IPV4_SOURCE: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
const IPV4_PEER: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 2);
const IPV4_MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(232, 1, 2, 3);
const IPV6_SOURCE: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0xfeed, 0, 0, 0, 0, 1);
const IPV6_PEER: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0xfeed, 0, 0, 0, 0, 2);
const IPV6_MULTICAST_GROUP: Ipv6Addr = Ipv6Addr::new(0xff3e, 0, 0, 0, 0, 0, 0, 0x1234);

#[test]
#[ignore = "requires root or CAP_NET_ADMIN/CAP_NET_RAW plus iproute2; creates an isolated veth network namespace"]
fn complete_icmp_control_datagrams_reach_the_veth_peer() {
    // SAFETY: this ignored integration test intentionally places only its own
    // process in a fresh network namespace before creating test interfaces.
    let unshare_result = unsafe { libc::unshare(libc::CLONE_NEWNET) };
    assert_eq!(
        unshare_result,
        0,
        "failed to create Linux network namespace: {}",
        std::io::Error::last_os_error()
    );

    configure_veth_namespace();
    let source_interface_index = interface_index(VETH_SOURCE);

    send_and_observe_icmpv4(source_interface_index);
    send_and_observe_icmpv6(source_interface_index);
}

fn configure_veth_namespace() {
    run_ip(&["link", "set", "lo", "up"]);
    run_ip(&[
        "link",
        "add",
        VETH_SOURCE,
        "type",
        "veth",
        "peer",
        "name",
        VETH_PEER,
    ]);
    run_ip(&["link", "set", VETH_SOURCE, "up"]);
    run_ip(&["link", "set", VETH_PEER, "up"]);
    run_ip(&["addr", "add", "198.18.0.1/24", "dev", VETH_SOURCE]);
    run_ip(&["addr", "add", "198.18.0.2/24", "dev", VETH_PEER]);
    run_ip(&[
        "-6",
        "addr",
        "add",
        "2001:db8:feed::1/64",
        "dev",
        VETH_SOURCE,
        "nodad",
    ]);
    run_ip(&[
        "-6",
        "addr",
        "add",
        "2001:db8:feed::2/64",
        "dev",
        VETH_PEER,
        "nodad",
    ]);
}

fn send_and_observe_icmpv4(interface_index: u32) {
    let receiver = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4)).unwrap();
    receiver
        .bind(&SockAddr::from(SocketAddrV4::new(IPV4_PEER, 0)))
        .unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let datagram = build_ipv4_fragmentation_needed(IPV4_SOURCE, IPV4_PEER, 1280);
    let mut context = RawIpContext::new();
    let id = context
        .add_publication(
            RawIpSocketConfig::ipv4()
                .with_bind_addr(IPV4_SOURCE)
                .with_interface_index(interface_index),
        )
        .unwrap();
    let report = context.send_ip_datagram(id, &datagram).unwrap();
    assert_eq!(report.bytes_sent, datagram.len());

    let mut received = [MaybeUninit::<u8>::uninit(); 256];
    let bytes_received = receiver.recv(&mut received).unwrap();
    // SAFETY: socket2 initialized exactly the prefix reported by recv.
    let received =
        unsafe { std::slice::from_raw_parts(received.as_ptr().cast::<u8>(), bytes_received) };
    assert!(bytes_received >= 56);
    assert_eq!(received[0] >> 4, 4);
    assert_eq!(&received[12..16], &IPV4_SOURCE.octets());
    assert_eq!(&received[16..20], &IPV4_PEER.octets());
    let outer_header_len = usize::from(received[0] & 0x0f) * 4;
    let icmp = &received[outer_header_len..];
    assert_eq!(&icmp[..2], &[3, 4]);
    let quoted = &icmp[8..];
    assert_eq!(quoted[0] >> 4, 4);
    assert_eq!(quoted[9], 17);
    assert_eq!(&quoted[12..16], &IPV4_PEER.octets());
    assert_eq!(&quoted[16..20], &IPV4_MULTICAST_GROUP.octets());
}

fn send_and_observe_icmpv6(interface_index: u32) {
    let receiver = Socket::new(
        Domain::IPV6,
        Type::RAW,
        Some(Protocol::from(libc::IPPROTO_ICMPV6)),
    )
    .unwrap();
    receiver
        .bind(&SockAddr::from(SocketAddrV6::new(IPV6_PEER, 0, 0, 0)))
        .unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let datagram = build_ipv6_packet_too_big(IPV6_SOURCE, IPV6_PEER, 1280);
    let mut context = RawIpContext::new();
    let id = context
        .add_publication(
            RawIpSocketConfig::ipv6()
                .with_bind_addr(IPV6_SOURCE)
                .with_interface_index(interface_index),
        )
        .unwrap();
    let report = context.send_ip_datagram(id, &datagram).unwrap();
    assert_eq!(report.bytes_sent, datagram.len());

    let mut received = [MaybeUninit::<u8>::uninit(); 256];
    let bytes_received = receiver.recv(&mut received).unwrap();
    // SAFETY: socket2 initialized exactly the prefix reported by recv.
    let packet =
        unsafe { std::slice::from_raw_parts(received.as_ptr().cast::<u8>(), bytes_received) };
    let icmp = if packet.first().is_some_and(|first| first >> 4 == 6) {
        assert!(packet.len() >= 96);
        assert_eq!(&packet[8..24], &IPV6_SOURCE.octets());
        assert_eq!(&packet[24..40], &IPV6_PEER.octets());
        &packet[40..]
    } else {
        packet
    };
    assert!(icmp.len() >= 56);
    assert_eq!(&icmp[..2], &[2, 0]);
    let quoted = &icmp[8..];
    assert_eq!(quoted[0] >> 4, 6);
    assert_eq!(quoted[6], 17);
    assert_eq!(&quoted[8..24], &IPV6_PEER.octets());
    assert_eq!(&quoted[24..40], &IPV6_MULTICAST_GROUP.octets());
}

fn build_ipv4_fragmentation_needed(source: Ipv4Addr, destination: Ipv4Addr, mtu: u16) -> [u8; 56] {
    let mut datagram = [0u8; 56];
    datagram[0] = 0x45;
    datagram[2..4].copy_from_slice(&56u16.to_be_bytes());
    datagram[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
    datagram[8] = 64;
    datagram[9] = 1;
    datagram[12..16].copy_from_slice(&source.octets());
    datagram[16..20].copy_from_slice(&destination.octets());
    datagram[20] = 3;
    datagram[21] = 4;
    datagram[26..28].copy_from_slice(&mtu.to_be_bytes());
    datagram[28..].copy_from_slice(&build_quoted_ipv4_udp_datagram(
        destination,
        IPV4_MULTICAST_GROUP,
    ));
    let icmp_checksum = internet_checksum(&datagram[20..]);
    datagram[22..24].copy_from_slice(&icmp_checksum.to_be_bytes());
    let ip_checksum = internet_checksum(&datagram[..20]);
    datagram[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    datagram
}

fn build_quoted_ipv4_udp_datagram(source: Ipv4Addr, destination: Ipv4Addr) -> [u8; 28] {
    let mut datagram = [0u8; 28];
    datagram[0] = 0x45;
    datagram[2..4].copy_from_slice(&28u16.to_be_bytes());
    datagram[4..6].copy_from_slice(&0x5678u16.to_be_bytes());
    datagram[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    datagram[8] = 16;
    datagram[9] = 17;
    datagram[12..16].copy_from_slice(&source.octets());
    datagram[16..20].copy_from_slice(&destination.octets());
    datagram[20..22].copy_from_slice(&4000u16.to_be_bytes());
    datagram[22..24].copy_from_slice(&5000u16.to_be_bytes());
    datagram[24..26].copy_from_slice(&8u16.to_be_bytes());
    let checksum = internet_checksum(&datagram[..20]);
    datagram[10..12].copy_from_slice(&checksum.to_be_bytes());
    datagram
}

fn build_ipv6_packet_too_big(source: Ipv6Addr, destination: Ipv6Addr, mtu: u32) -> [u8; 96] {
    let mut datagram = [0u8; 96];
    datagram[0] = 0x60;
    datagram[4..6].copy_from_slice(&56u16.to_be_bytes());
    datagram[6] = 58;
    datagram[7] = 64;
    datagram[8..24].copy_from_slice(&source.octets());
    datagram[24..40].copy_from_slice(&destination.octets());
    datagram[40] = 2;
    datagram[44..48].copy_from_slice(&mtu.to_be_bytes());
    datagram[48..].copy_from_slice(&build_quoted_ipv6_udp_datagram(
        destination,
        IPV6_MULTICAST_GROUP,
    ));
    let checksum = ipv6_icmp_checksum(source, destination, &datagram[40..]);
    datagram[42..44].copy_from_slice(&checksum.to_be_bytes());
    datagram
}

fn build_quoted_ipv6_udp_datagram(source: Ipv6Addr, destination: Ipv6Addr) -> [u8; 48] {
    let mut datagram = [0u8; 48];
    datagram[0] = 0x60;
    datagram[4..6].copy_from_slice(&8u16.to_be_bytes());
    datagram[6] = 17;
    datagram[7] = 16;
    datagram[8..24].copy_from_slice(&source.octets());
    datagram[24..40].copy_from_slice(&destination.octets());
    datagram[40..42].copy_from_slice(&4000u16.to_be_bytes());
    datagram[42..44].copy_from_slice(&5000u16.to_be_bytes());
    datagram[44..46].copy_from_slice(&8u16.to_be_bytes());
    let checksum = ipv6_transport_checksum(source, destination, 17, &datagram[40..]);
    datagram[46..48].copy_from_slice(&checksum.to_be_bytes());
    datagram
}

fn ipv6_icmp_checksum(source: Ipv6Addr, destination: Ipv6Addr, payload: &[u8]) -> u16 {
    ipv6_transport_checksum(source, destination, 58, payload)
}

fn ipv6_transport_checksum(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    next_header: u8,
    payload: &[u8],
) -> u16 {
    let payload_len = (payload.len() as u32).to_be_bytes();
    let next_header = [0u8, 0, 0, next_header];
    internet_checksum_slices(&[
        &source.octets(),
        &destination.octets(),
        &payload_len,
        &next_header,
        payload,
    ])
}

fn internet_checksum(data: &[u8]) -> u16 {
    internet_checksum_slices(&[data])
}

fn internet_checksum_slices(slices: &[&[u8]]) -> u16 {
    let mut sum = 0u32;
    let mut trailing = None;

    for slice in slices {
        let mut bytes = *slice;
        if let Some(high) = trailing.take() {
            if let Some((&low, rest)) = bytes.split_first() {
                sum += u32::from(u16::from_be_bytes([high, low]));
                bytes = rest;
            } else {
                trailing = Some(high);
                continue;
            }
        }

        for chunk in bytes.chunks_exact(2) {
            sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
        }
        if let Some(byte) = bytes.chunks_exact(2).remainder().first() {
            trailing = Some(*byte);
        }
    }

    if let Some(high) = trailing {
        sum += u32::from(high) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn interface_index(name: &str) -> u32 {
    let name = CString::new(name).unwrap();
    // SAFETY: the C string is NUL-terminated and remains valid for the call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(index, 0, "failed to resolve interface index for {name:?}");
    index
}

fn run_ip(args: &[&str]) {
    let output = Command::new("ip").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "ip {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
