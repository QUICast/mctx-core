#![cfg(all(target_os = "linux", feature = "raw-route-egress"))]

use mctx_core::{MctxError, RawContext, RawPublicationConfig};
use socket2::Socket;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Command;
use std::time::Duration;

const PATH_A_TX: &str = "mctxrta0";
const PATH_A_RX: &str = "mctxrta1";
const PATH_B_TX: &str = "mctxrtb0";
const PATH_B_RX: &str = "mctxrtb1";
const SOURCE: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 10);
const GROUP: Ipv4Addr = Ipv4Addr::new(239, 250, 0, 1);
const GROUP_ROUTE: &str = "239.250.0.1/32";
const TRAFFIC_CLASS: u8 = 0xbb;
const TTL: u8 = 17;

#[test]
#[ignore = "requires root or CAP_NET_ADMIN/CAP_NET_RAW plus iproute2; creates an isolated two-path veth namespace"]
fn one_route_selected_publication_follows_ipv4_route_changes() {
    // SAFETY: this ignored test intentionally moves only its calling thread
    // into a fresh network namespace that is discarded when the test exits.
    let unshare_result = unsafe { libc::unshare(libc::CLONE_NEWNET) };
    assert_eq!(
        unshare_result,
        0,
        "failed to create Linux network namespace: {}",
        std::io::Error::last_os_error()
    );

    configure_namespace();
    let path_a_capture = open_ipv4_packet_capture(PATH_A_RX);
    let path_b_capture = open_ipv4_packet_capture(PATH_B_RX);

    let mut context = RawContext::new();
    let publication_id = context
        .add_publication(RawPublicationConfig::ipv4().with_route_selected_egress())
        .unwrap();

    let first = build_ipv4_udp_datagram(0x1234, b"path-a");
    let transient = context.send_raw(publication_id, &first).unwrap_err();
    let MctxError::RawSendFailed(transient) = transient else {
        panic!("unreachable route did not return RawSendFailed");
    };
    assert!(
        transient.raw_os_error().is_some(),
        "kernel route failure lost its native OS error"
    );

    run_ip(&["route", "replace", GROUP_ROUTE, "dev", PATH_A_TX]);
    let first_report = context.send_raw(publication_id, &first).unwrap();
    assert_eq!(first_report.publication_id, publication_id);
    assert_eq!(first_report.local_bind_addr, None);
    assert_eq!(first_report.outgoing_interface, None);
    assert_eq!(first_report.outgoing_interface_index, None);
    let first_received = receive_ipv4_datagram(&path_a_capture, 0x1234);
    assert_preserved_ipv4_header(&first_received, 0x1234, b"path-a");

    run_ip(&["route", "replace", GROUP_ROUTE, "dev", PATH_B_TX]);
    let second = build_ipv4_udp_datagram(0x5678, b"path-b");
    let second_report = context.send_raw(publication_id, &second).unwrap();
    assert_eq!(second_report.publication_id, publication_id);
    let second_received = receive_ipv4_datagram(&path_b_capture, 0x5678);
    assert_preserved_ipv4_header(&second_received, 0x5678, b"path-b");

    assert_eq!(
        context.get_publication(publication_id).unwrap().id(),
        publication_id
    );
}

fn configure_namespace() {
    run_ip(&["link", "set", "lo", "up"]);
    add_veth_pair(PATH_A_TX, PATH_A_RX, "198.18.10.1/24", "198.18.10.2/24");
    add_veth_pair(PATH_B_TX, PATH_B_RX, "198.18.20.1/24", "198.18.20.2/24");
    run_ip(&["route", "add", "unreachable", GROUP_ROUTE]);
}

fn add_veth_pair(tx: &str, rx: &str, tx_addr: &str, rx_addr: &str) {
    run_ip(&["link", "add", tx, "type", "veth", "peer", "name", rx]);
    run_ip(&["addr", "add", tx_addr, "dev", tx]);
    run_ip(&["addr", "add", rx_addr, "dev", rx]);
    run_ip(&["link", "set", tx, "up"]);
    run_ip(&["link", "set", rx, "up"]);
}

fn open_ipv4_packet_capture(interface: &str) -> Socket {
    let protocol = (libc::ETH_P_IP as u16).to_be();
    // SAFETY: the descriptor is created with Linux AF_PACKET constants and is
    // transferred immediately into socket2 ownership below.
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
    let bind_addr = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: i32::try_from(interface_index(interface)).unwrap(),
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };
    // SAFETY: bind_addr is a fully initialized sockaddr_ll for this interface.
    let bind_result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&bind_addr as *const libc::sockaddr_ll).cast(),
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
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
}

fn receive_ipv4_datagram(socket: &Socket, identification: u16) -> Vec<u8> {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];

    for _ in 0..8 {
        let bytes_received = socket.recv(&mut buffer).unwrap();
        // SAFETY: socket2 initialized exactly the prefix reported by recv.
        let packet =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), bytes_received) };
        if packet.len() >= 20
            && packet[0] >> 4 == 4
            && packet[4..6] == identification.to_be_bytes()
            && packet[16..20] == GROUP.octets()
        {
            return packet.to_vec();
        }
    }

    panic!("did not capture IPv4 datagram with identification {identification:#06x}");
}

fn assert_preserved_ipv4_header(datagram: &[u8], identification: u16, payload: &[u8]) {
    assert!(datagram.len() >= 28 + payload.len());
    assert_eq!(datagram[0], 0x45);
    assert_eq!(datagram[1], TRAFFIC_CLASS);
    assert_eq!(
        u16::from_be_bytes([datagram[2], datagram[3]]) as usize,
        datagram.len()
    );
    assert_eq!(
        u16::from_be_bytes([datagram[4], datagram[5]]),
        identification
    );
    assert_eq!(&datagram[6..8], &0x4000u16.to_be_bytes());
    assert_eq!(datagram[8], TTL);
    assert_eq!(datagram[9], 17);
    assert_eq!(&datagram[12..16], &SOURCE.octets());
    assert_eq!(&datagram[16..20], &GROUP.octets());
    assert_eq!(internet_checksum(&datagram[..20]), 0);
    assert_eq!(&datagram[28..], payload);
}

fn build_ipv4_udp_datagram(identification: u16, payload: &[u8]) -> Vec<u8> {
    let total_len = 28 + payload.len();
    let mut datagram = vec![0u8; total_len];
    datagram[0] = 0x45;
    datagram[1] = TRAFFIC_CLASS;
    datagram[2..4].copy_from_slice(&u16::try_from(total_len).unwrap().to_be_bytes());
    datagram[4..6].copy_from_slice(&identification.to_be_bytes());
    datagram[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    datagram[8] = TTL;
    datagram[9] = 17;
    datagram[12..16].copy_from_slice(&SOURCE.octets());
    datagram[16..20].copy_from_slice(&GROUP.octets());
    datagram[20..22].copy_from_slice(&4000u16.to_be_bytes());
    datagram[22..24].copy_from_slice(&5000u16.to_be_bytes());
    datagram[24..26].copy_from_slice(&u16::try_from(8 + payload.len()).unwrap().to_be_bytes());
    datagram[28..].copy_from_slice(payload);
    let checksum = internet_checksum(&datagram[..20]);
    datagram[10..12].copy_from_slice(&checksum.to_be_bytes());
    datagram
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in bytes.chunks_exact(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    if let Some(byte) = bytes.chunks_exact(2).remainder().first() {
        sum += u32::from(*byte) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn interface_index(name: &str) -> u32 {
    let name = CString::new(name).unwrap();
    // SAFETY: the C string remains valid for the synchronous call.
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
