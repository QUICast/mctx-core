use crate::MctxError;
#[cfg(feature = "raw-packets")]
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

#[cfg(windows)]
fn aligned_ffi_buffer<T>(byte_len: u32) -> Vec<std::mem::MaybeUninit<T>> {
    let element_count = (byte_len as usize).div_ceil(std::mem::size_of::<T>());
    Vec::with_capacity(element_count)
}

#[cfg(all(unix, feature = "raw-packets"))]
pub(crate) fn resolve_ipv4_interface_index(interface: Ipv4Addr) -> Result<u32, MctxError> {
    fn ambiguous_interface_error(interface: Ipv4Addr, first: u32, second: u32) -> MctxError {
        MctxError::InterfaceDiscoveryFailed(format!(
            "IPv4 interface address {interface} is ambiguous across interface indices {first} and {second}; provide an explicit interface index or choose a unique local address"
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

            if !addr.is_null() && (*addr).sa_family as libc::c_int == libc::AF_INET {
                let sockaddr = &*(addr as *const libc::sockaddr_in);
                let candidate = Ipv4Addr::from(u32::from_be(sockaddr.sin_addr.s_addr));
                if candidate == interface {
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
                "failed to resolve IPv4 interface address {interface} to an interface index"
            ))
        })
    }
}

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

#[cfg(all(windows, feature = "raw-packets"))]
pub(crate) fn resolve_ipv4_interface_index(interface: Ipv4Addr) -> Result<u32, MctxError> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};

    const INITIAL_BUFFER_SIZE: usize = 15_000;

    fn ambiguous_interface_error(interface: Ipv4Addr, first: u32, second: u32) -> MctxError {
        MctxError::InterfaceDiscoveryFailed(format!(
            "IPv4 interface address {interface} is ambiguous across interface indices {first} and {second}; provide an explicit interface index or choose a unique local address"
        ))
    }

    fn adapter_ipv4_if_index(
        adapter: *const windows_sys::Win32::NetworkManagement::IpHelper::IP_ADAPTER_ADDRESSES_LH,
    ) -> u32 {
        unsafe { (*adapter).Anonymous1.Anonymous.IfIndex }
    }

    let mut buf_len = INITIAL_BUFFER_SIZE as u32;

    loop {
        // GetAdaptersAddresses writes typed linked structures into this buffer,
        // so its base pointer must satisfy the structure's alignment.
        let mut buffer = aligned_ffi_buffer::<IP_ADAPTER_ADDRESSES_LH>(buf_len);
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                0,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
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
                        && (*socket_address.lpSockaddr).sa_family == AF_INET
                    {
                        let sockaddr = &*(socket_address.lpSockaddr as *const SOCKADDR_IN);
                        let candidate = Ipv4Addr::from(u32::from_be(sockaddr.sin_addr.S_un.S_addr));
                        if candidate == interface {
                            let if_index = adapter_ipv4_if_index(adapter);
                            match matched_index {
                                Some(existing) if existing != if_index => {
                                    return Err(ambiguous_interface_error(
                                        interface, existing, if_index,
                                    ));
                                }
                                Some(_) => {}
                                None => matched_index = Some(if_index),
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
                "failed to resolve IPv4 interface address {interface} to an interface index"
            ))
        });
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
        // GetAdaptersAddresses writes typed linked structures into this buffer,
        // so its base pointer must satisfy the structure's alignment.
        let mut buffer = aligned_ffi_buffer::<IP_ADAPTER_ADDRESSES_LH>(buf_len);
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                0,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
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

#[cfg(all(not(any(unix, windows)), feature = "raw-packets"))]
pub(crate) fn resolve_ipv4_interface_index(interface: Ipv4Addr) -> Result<u32, MctxError> {
    Err(MctxError::InterfaceDiscoveryFailed(format!(
        "IPv4 interface resolution is not implemented on this platform for {interface}"
    )))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn resolve_ipv6_interface_index(interface: Ipv6Addr) -> Result<u32, MctxError> {
    Err(MctxError::InterfaceDiscoveryFailed(format!(
        "IPv6 interface resolution is not implemented on this platform for {interface}"
    )))
}
