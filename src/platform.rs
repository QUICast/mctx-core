use crate::MctxError;
use std::net::Ipv6Addr;

#[cfg(unix)]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
    fn ambiguous_interface_error(interface: Ipv6Addr, first: u32, second: u32) -> MctxError {
        MctxError::InterfaceDiscoveryFailed(format!(
            "IPv6 interface address {interface} is ambiguous across interface indices {first} and {second}; provide an explicit interface index or scoped bind address"
        ))
    }

    unsafe {
        let mut ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifaddrs) != 0 {
            return Err(MctxError::InterfaceDiscoveryFailed(
                std::io::Error::last_os_error().to_string(),
            ));
        }

        let mut cursor = ifaddrs;
        let mut matched_index = None;

        while !cursor.is_null() {
            let addr = (*cursor).ifa_addr;

            if !addr.is_null() && (*addr).sa_family as libc::c_int == libc::AF_INET6 {
                let sockaddr = &*(addr as *const libc::sockaddr_in6);
                if Ipv6Addr::from(sockaddr.sin6_addr.s6_addr) == interface {
                    let index = libc::if_nametoindex((*cursor).ifa_name);
                    if index != 0 {
                        match matched_index {
                            Some(existing) if existing != index => {
                                libc::freeifaddrs(ifaddrs);
                                return Err(ambiguous_interface_error(interface, existing, index));
                            }
                            Some(_) => {}
                            None => matched_index = Some(index),
                        }
                    }
                }
            }

            cursor = (*cursor).ifa_next;
        }

        libc::freeifaddrs(ifaddrs);

        matched_index.ok_or_else(|| {
            MctxError::InterfaceDiscoveryFailed(format!(
                "failed to resolve IPv6 interface address {interface} to an interface index"
            ))
        })
    }
}

#[cfg(windows)]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET6, AF_UNSPEC, SOCKADDR_IN6};

    const INITIAL_BUFFER_SIZE: usize = 15_000;

    fn ambiguous_interface_error(interface: Ipv6Addr, first: u32, second: u32) -> MctxError {
        MctxError::InterfaceDiscoveryFailed(format!(
            "IPv6 interface address {interface} is ambiguous across interface indices {first} and {second}; provide an explicit interface index or scoped bind address"
        ))
    }

    let mut buf_len = INITIAL_BUFFER_SIZE as u32;

    loop {
        let mut buffer = vec![0u8; buf_len as usize];
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                0,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut buf_len,
            )
        };

        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }

        if result != NO_ERROR {
            return Err(MctxError::InterfaceDiscoveryFailed(format!(
                "GetAdaptersAddresses failed with status {result}"
            )));
        }

        let mut adapter = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let mut matched_index = None;

        unsafe {
            while !adapter.is_null() {
                let mut unicast = (*adapter).FirstUnicastAddress;

                while !unicast.is_null() {
                    let socket_address = (*unicast).Address;

                    if !socket_address.lpSockaddr.is_null()
                        && (*socket_address.lpSockaddr).sa_family == AF_INET6
                    {
                        let sockaddr = &*(socket_address.lpSockaddr as *const SOCKADDR_IN6);
                        let candidate = Ipv6Addr::from(sockaddr.sin6_addr.u.Byte);
                        if candidate == interface {
                            match matched_index {
                                Some(existing) if existing != (*adapter).Ipv6IfIndex => {
                                    return Err(ambiguous_interface_error(
                                        interface,
                                        existing,
                                        (*adapter).Ipv6IfIndex,
                                    ));
                                }
                                Some(_) => {}
                                None => matched_index = Some((*adapter).Ipv6IfIndex),
                            }
                        }
                    }

                    unicast = (*unicast).Next;
                }

                adapter = (*adapter).Next;
            }
        }

        return matched_index.ok_or_else(|| {
            MctxError::InterfaceDiscoveryFailed(format!(
                "failed to resolve IPv6 interface address {interface} to an interface index"
            ))
        });
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
    Err(MctxError::InterfaceDiscoveryFailed(format!(
        "IPv6 interface resolution is not implemented on this platform for {interface}"
    )))
}
