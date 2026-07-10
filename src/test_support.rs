use crate::{MctxError, platform::resolve_ipv6_interface_index};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::time::Duration;

pub(crate) const TEST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 12, 34);
pub(crate) const TEST_GROUP_V6_SAME_HOST: Ipv6Addr =
    Ipv6Addr::new(0xff31, 0, 0, 0, 0, 0, 0x8000, 0x1234);
pub(crate) const TEST_GROUP_V6_GLOBAL: Ipv6Addr =
    Ipv6Addr::new(0xff3e, 0, 0, 0, 0, 0, 0x8000, 0x1234);

pub(crate) fn is_multicast_test_network_unavailable(error: &MctxError) -> bool {
    match error {
        MctxError::SocketConnectFailed(error) => {
            error.kind() == std::io::ErrorKind::NetworkUnreachable
        }
        MctxError::SendFailed(error) => {
            error.kind() == std::io::ErrorKind::NetworkUnreachable
                || (cfg!(any(windows, target_os = "macos"))
                    && error.kind() == std::io::ErrorKind::BrokenPipe)
        }
        _ => false,
    }
}

pub(crate) fn multicast_test_result_or_skip<T>(result: Result<T, MctxError>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) if is_multicast_test_network_unavailable(&error) => {
            eprintln!("skipping multicast integration test: {error}");
            None
        }
        Err(error) => panic!("multicast integration test failed: {error}"),
    }
}

pub(crate) fn test_multicast_receiver() -> (UdpSocket, u16) {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = socket.local_addr().unwrap().port();

    socket
        .join_multicast_v4(&TEST_GROUP, &Ipv4Addr::UNSPECIFIED)
        .unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();

    (socket, port)
}

pub(crate) fn test_multicast_receiver_v6(group: Ipv6Addr, interface: Ipv6Addr) -> (UdpSocket, u16) {
    let socket = UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    let interface_index = resolve_ipv6_interface_index(interface).unwrap();

    socket.join_multicast_v6(&group, interface_index).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();

    (socket, port)
}

pub(crate) fn recv_payload(socket: &UdpSocket) -> Vec<u8> {
    recv_payload_with_source(socket).0
}

pub(crate) fn recv_payload_with_source(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buffer = [0_u8; 2048];
    let (len, addr) = socket.recv_from(&mut buffer).unwrap();
    (buffer[..len].to_vec(), addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_environmental_multicast_errors_are_skippable() {
        assert!(is_multicast_test_network_unavailable(
            &MctxError::SocketConnectFailed(std::io::Error::from(
                std::io::ErrorKind::NetworkUnreachable,
            ))
        ));
        assert_eq!(
            is_multicast_test_network_unavailable(&MctxError::SendFailed(std::io::Error::from(
                std::io::ErrorKind::BrokenPipe
            ))),
            cfg!(any(windows, target_os = "macos"))
        );
        assert!(!is_multicast_test_network_unavailable(
            &MctxError::SendFailed(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
        ));
    }
}
