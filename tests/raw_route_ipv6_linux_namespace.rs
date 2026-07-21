#![cfg(all(target_os = "linux", feature = "raw-route-egress"))]

use mctx_core::{MctxError, RawContext, RawPublicationConfig};
use socket2::Socket;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Command;
use std::time::Duration;

const PATH_A_TX: &str = "mctx6a0";
const PATH_A_RX: &str = "mctx6a1";
const PATH_B_TX: &str = "mctx6b0";
const PATH_B_RX: &str = "mctx6b1";
const SOURCE: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0xffff, 0, 0, 0, 0, 0x10);
const GROUP: Ipv6Addr = Ipv6Addr::new(0xff3e, 0, 0, 0, 0, 0, 0x8000, 0x1234);
const GROUP_ROUTE: &str = "ff3e::8000:1234/128";
const TRAFFIC_CLASS: u8 = 0xab;
const FLOW_LABEL: u32 = 0x54321;
const HOP_LIMIT: u8 = 37;
const UDP_CHECKSUM: u16 = 0x4a2b;
const DESTINATION_OPTIONS: [u8; 8] = [17, 0, 1, 4, 0xde, 0xad, 0xbe, 0xef];

#[test]
#[ignore = "requires root or CAP_NET_ADMIN/CAP_NET_RAW plus iproute2; creates an isolated two-path veth namespace"]
fn one_ipv6_publication_roams_and_recovers_without_rewriting_the_packet() {
    // SAFETY: this ignored test moves only its calling thread into a fresh
    // network namespace that is discarded when the test process exits.
    let unshare_result = unsafe { libc::unshare(libc::CLONE_NEWNET) };
    assert_eq!(
        unshare_result,
        0,
        "failed to create Linux network namespace: {}",
        std::io::Error::last_os_error()
    );

    configure_namespace();
    let path_a_capture = open_ipv6_packet_capture(PATH_A_RX);
    let path_b_capture = open_ipv6_packet_capture(PATH_B_RX);
    let path_a_index = interface_index(PATH_A_TX);
    let path_b_index = interface_index(PATH_B_TX);

    let mut context = RawContext::new();
    let publication_id = context
        .add_publication(
            RawPublicationConfig::ipv6()
                .with_route_selected_egress()
                .with_loopback(false),
        )
        .unwrap();

    let first = build_ipv6_datagram(b"path-a");
    let first_report = context.send_raw(publication_id, &first).unwrap();
    assert_eq!(first_report.publication_id, publication_id);
    assert_eq!(first_report.outgoing_interface_index, Some(path_a_index));
    assert_eq!(first_report.local_bind_addr, None);
    assert_eq!(first_report.outgoing_interface, None);
    let first_received = receive_ipv6_datagram(&path_a_capture, b"path-a");
    assert_preserved_ipv6_datagram(&first_received, &first, b"path-a");
    assert_no_ipv6_datagram(&path_b_capture, b"path-a");

    run_ip(&["-6", "route", "replace", GROUP_ROUTE, "dev", PATH_B_TX]);
    let second = build_ipv6_datagram(b"path-b");
    let second_report = context.send_raw(publication_id, &second).unwrap();
    assert_eq!(second_report.publication_id, publication_id);
    assert_eq!(second_report.outgoing_interface_index, Some(path_b_index));
    let second_received = receive_ipv6_datagram(&path_b_capture, b"path-b");
    assert_preserved_ipv6_datagram(&second_received, &second, b"path-b");
    assert_no_ipv6_datagram(&path_a_capture, b"path-b");

    run_ip(&["-6", "route", "del", GROUP_ROUTE]);
    let no_route = context.send_raw(publication_id, &second).unwrap_err();
    let MctxError::RawSendFailed(no_route) = no_route else {
        panic!("missing IPv6 route did not return RawSendFailed");
    };
    assert_eq!(no_route.raw_os_error(), Some(libc::ENETUNREACH));
    assert_eq!(no_route.kind(), std::io::ErrorKind::NetworkUnreachable);

    run_ip(&["-6", "route", "add", GROUP_ROUTE, "dev", PATH_B_TX]);
    let recovered = build_ipv6_datagram(b"recovered");
    let recovered_report = context.send_raw(publication_id, &recovered).unwrap();
    assert_eq!(recovered_report.publication_id, publication_id);
    assert_eq!(
        recovered_report.outgoing_interface_index,
        Some(path_b_index)
    );
    let recovered_packet = receive_ipv6_datagram(&path_b_capture, b"recovered");
    assert_preserved_ipv6_datagram(&recovered_packet, &recovered, b"recovered");
}

fn configure_namespace() {
    run_ip(&["link", "set", "lo", "up"]);
    add_veth_pair(PATH_A_TX, PATH_A_RX);
    add_veth_pair(PATH_B_TX, PATH_B_RX);
    run_ip(&["-6", "route", "flush", "dev", PATH_A_TX]);
    run_ip(&["-6", "route", "flush", "dev", PATH_B_TX]);
    run_ip(&["-6", "route", "add", GROUP_ROUTE, "dev", PATH_A_TX]);
}

fn add_veth_pair(tx: &str, rx: &str) {
    run_ip(&["link", "add", tx, "type", "veth", "peer", "name", rx]);
    run_ip(&["link", "set", tx, "up"]);
    run_ip(&["link", "set", rx, "up"]);
}

fn open_ipv6_packet_capture(interface: &str) -> Socket {
    let protocol = (libc::ETH_P_IPV6 as u16).to_be();
    // SAFETY: the descriptor is created with Linux AF_PACKET constants and is
    // transferred immediately into socket2 ownership.
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            i32::from(protocol),
        )
    };
    assert_ne!(
        raw_fd,
        -1,
        "failed to open packet capture: {}",
        std::io::Error::last_os_error()
    );

    // SAFETY: raw_fd is newly created and uniquely owned.
    let socket = unsafe { Socket::from_raw_fd(raw_fd) };
    let bind_address = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: i32::try_from(interface_index(interface)).unwrap(),
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };
    // SAFETY: bind_address is a fully initialized sockaddr_ll.
    let bind_result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&bind_address as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    assert_eq!(
        bind_result,
        0,
        "failed to bind packet capture to {interface}: {}",
        std::io::Error::last_os_error()
    );
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    socket
}

fn receive_ipv6_datagram(socket: &Socket, marker: &[u8]) -> Vec<u8> {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
    for _ in 0..16 {
        let bytes_received = socket.recv(&mut buffer).unwrap();
        // SAFETY: socket2 initialized exactly the prefix reported by recv.
        let packet =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), bytes_received) };
        if packet.len() >= 56
            && packet[0] >> 4 == 6
            && packet[24..40] == GROUP.octets()
            && packet.ends_with(marker)
        {
            return packet.to_vec();
        }
    }
    panic!("did not capture expected IPv6 multicast datagram");
}

fn assert_no_ipv6_datagram(socket: &Socket, marker: &[u8]) {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
    loop {
        match socket.recv(&mut buffer) {
            Ok(bytes_received) => {
                // SAFETY: socket2 initialized the reported prefix.
                let packet = unsafe {
                    std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), bytes_received)
                };
                assert!(
                    !packet.ends_with(marker),
                    "packet leaked onto the old route"
                );
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return;
            }
            Err(error) => panic!("unexpected capture error: {error}"),
        }
    }
}

fn assert_preserved_ipv6_datagram(received: &[u8], expected: &[u8], payload: &[u8]) {
    assert_eq!(received, expected);
    let traffic_class = ((received[0] & 0x0f) << 4) | (received[1] >> 4);
    let flow_label = (u32::from(received[1] & 0x0f) << 16)
        | (u32::from(received[2]) << 8)
        | u32::from(received[3]);
    assert_eq!(traffic_class, TRAFFIC_CLASS);
    assert_eq!(flow_label, FLOW_LABEL);
    assert_eq!(received[6], 60);
    assert_eq!(received[7], HOP_LIMIT);
    assert_eq!(&received[8..24], &SOURCE.octets());
    assert_eq!(&received[24..40], &GROUP.octets());
    assert_eq!(&received[40..48], &DESTINATION_OPTIONS);
    assert_eq!(
        u16::from_be_bytes([received[54], received[55]]),
        UDP_CHECKSUM
    );
    assert_eq!(&received[56..], payload);
}

fn build_ipv6_datagram(payload: &[u8]) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let payload_len = DESTINATION_OPTIONS.len() + udp_len;
    let mut datagram = vec![0u8; 40 + payload_len];
    let version_class_flow = (6u32 << 28) | (u32::from(TRAFFIC_CLASS) << 20) | FLOW_LABEL;
    datagram[..4].copy_from_slice(&version_class_flow.to_be_bytes());
    datagram[4..6].copy_from_slice(&u16::try_from(payload_len).unwrap().to_be_bytes());
    datagram[6] = 60;
    datagram[7] = HOP_LIMIT;
    datagram[8..24].copy_from_slice(&SOURCE.octets());
    datagram[24..40].copy_from_slice(&GROUP.octets());
    datagram[40..48].copy_from_slice(&DESTINATION_OPTIONS);
    datagram[48..50].copy_from_slice(&4000u16.to_be_bytes());
    datagram[50..52].copy_from_slice(&5000u16.to_be_bytes());
    datagram[52..54].copy_from_slice(&u16::try_from(udp_len).unwrap().to_be_bytes());
    datagram[54..56].copy_from_slice(&UDP_CHECKSUM.to_be_bytes());
    datagram[56..].copy_from_slice(payload);
    datagram
}

fn interface_index(name: &str) -> u32 {
    let name = CString::new(name).unwrap();
    // SAFETY: name remains valid for the synchronous libc call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(index, 0, "failed to resolve interface index for {name:?}");
    index
}

fn run_ip(arguments: &[&str]) {
    let output = Command::new("ip").args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "ip {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
