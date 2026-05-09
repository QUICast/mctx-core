use crate::MctxError;
use std::net::Ipv6Addr;

#[cfg(unix)]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
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
                        matched_index = Some(index);
                        break;
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
                            return Ok((*adapter).Ipv6IfIndex);
                        }
                    }

                    unicast = (*unicast).Next;
                }

                adapter = (*adapter).Next;
            }
        }

        return Err(MctxError::InterfaceDiscoveryFailed(format!(
            "failed to resolve IPv6 interface address {interface} to an interface index"
        )));
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
    Err(MctxError::InterfaceDiscoveryFailed(format!(
        "IPv6 interface resolution is not implemented on this platform for {interface}"
    )))
}
