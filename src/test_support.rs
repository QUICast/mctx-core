use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

pub(crate) const TEST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 12, 34);

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

pub(crate) fn recv_payload(socket: &UdpSocket) -> Vec<u8> {
    let mut buffer = [0_u8; 2048];
    let (len, _) = socket.recv_from(&mut buffer).unwrap();
    buffer[..len].to_vec()
}
