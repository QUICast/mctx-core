use crate::error::MctxError;
use socket2::Socket;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd};

pub(crate) fn open_ipv6(interface_index: u32) -> Result<Socket, MctxError> {
    let interface_index = packet_interface_index(interface_index)?;
    ensure_ethernet_interface(interface_index as u32)?;
    let protocol = (libc::ETH_P_IPV6 as u16).to_be();

    // SAFETY: socket is called with Linux AF_PACKET constants and ownership of
    // the returned descriptor is immediately transferred to socket2::Socket.
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            i32::from(protocol),
        )
    };
    if raw_fd == -1 {
        return Err(MctxError::RawSocketCreateFailed(
            std::io::Error::last_os_error(),
        ));
    }

    // SAFETY: raw_fd is newly created and uniquely owned here.
    let socket = unsafe { Socket::from_raw_fd(raw_fd) };
    let bind_addr = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: interface_index,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };

    // SAFETY: bind_addr is a valid sockaddr_ll for the selected interface.
    let result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&bind_addr as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if result == -1 {
        return Err(MctxError::RawSocketBindFailed(
            std::io::Error::last_os_error(),
        ));
    }

    Ok(socket)
}

pub(crate) fn send_ipv6(
    socket: &Socket,
    interface_index: u32,
    group: Ipv6Addr,
    datagram: &[u8],
) -> Result<usize, MctxError> {
    let destination_mac = ipv6_multicast_mac(group);
    let mut destination = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: (libc::ETH_P_IPV6 as u16).to_be(),
        sll_ifindex: packet_interface_index(interface_index)?,
        sll_hatype: libc::ARPHRD_ETHER,
        sll_pkttype: 0,
        sll_halen: destination_mac.len() as u8,
        sll_addr: [0; 8],
    };
    destination.sll_addr[..destination_mac.len()].copy_from_slice(&destination_mac);

    // SAFETY: destination points to a fully initialized sockaddr_ll and the
    // datagram slice remains valid for the duration of sendto.
    let result = unsafe {
        libc::sendto(
            socket.as_raw_fd(),
            datagram.as_ptr().cast(),
            datagram.len(),
            0,
            (&destination as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if result == -1 {
        return Err(MctxError::RawSendFailed(std::io::Error::last_os_error()));
    }
    if result as usize != datagram.len() {
        return Err(MctxError::RawSendFailed(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            format!(
                "partial Linux packet-socket send: wrote {result} of {} bytes",
                datagram.len()
            ),
        )));
    }

    Ok(result as usize)
}

fn packet_interface_index(interface_index: u32) -> Result<i32, MctxError> {
    i32::try_from(interface_index).map_err(|_| {
        MctxError::InterfaceDiscoveryFailed(format!(
            "Linux interface index {interface_index} exceeds the packet-socket range"
        ))
    })
}

fn ensure_ethernet_interface(interface_index: u32) -> Result<(), MctxError> {
    let (interface_name, link_type) = link_info(interface_index)?;
    if link_type != i32::from(libc::ARPHRD_ETHER) {
        return Err(MctxError::RawUnsupportedLinkType(format!(
            "{interface_name} (Linux ARPHRD {link_type})"
        )));
    }

    Ok(())
}

fn link_info(interface_index: u32) -> Result<(String, i32), MctxError> {
    // SAFETY: getifaddrs owns the linked list until the matching freeifaddrs
    // call, and each address is checked for null and family before casting.
    unsafe {
        let mut ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return Err(MctxError::InterfaceDiscoveryFailed(
                std::io::Error::last_os_error().to_string(),
            ));
        }

        let mut cursor = ifaddrs;
        while !cursor.is_null() {
            let addr = (*cursor).ifa_addr;
            if !addr.is_null() && (*addr).sa_family as libc::c_int == libc::AF_PACKET {
                let sockaddr = &*(addr as *const libc::sockaddr_ll);
                if sockaddr.sll_ifindex == interface_index as i32 {
                    let interface_name = std::ffi::CStr::from_ptr((*cursor).ifa_name)
                        .to_string_lossy()
                        .into_owned();
                    let link_type = i32::from(sockaddr.sll_hatype);
                    libc::freeifaddrs(ifaddrs);
                    return Ok((interface_name, link_type));
                }
            }

            cursor = (*cursor).ifa_next;
        }

        libc::freeifaddrs(ifaddrs);
    }

    Err(MctxError::InterfaceDiscoveryFailed(format!(
        "failed to resolve Linux interface metadata for index {interface_index}"
    )))
}

fn ipv6_multicast_mac(group: Ipv6Addr) -> [u8; 6] {
    let octets = group.octets();
    [0x33, 0x33, octets[12], octets[13], octets[14], octets[15]]
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
}
