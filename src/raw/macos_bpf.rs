use crate::error::MctxError;
use crate::raw::link::ethernet_ipv6_header;
use socket2::Socket;
use std::ffi::{CStr, CString};
use std::io::{self, ErrorKind};
use std::mem::offset_of;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd};

#[derive(Debug)]
pub(crate) struct MacosBpfIpv6Socket {
    socket: Socket,
    source_mac: [u8; 6],
}

impl MacosBpfIpv6Socket {
    pub(crate) fn open(interface_index: u32) -> Result<Self, MctxError> {
        let interface_name = interface_name_from_index(interface_index)?;
        let source_mac = interface_mac(interface_index, &interface_name)?;
        let socket = open_available_bpf_device()?;
        bind_bpf_to_interface(&socket, &interface_name)?;

        let datalink = get_bpf_u32(&socket, libc::BIOCGDLT)?;
        ensure_ethernet_datalink(&interface_name, datalink)?;
        set_bpf_u32(&socket, libc::BIOCSHDRCMPLT, 1)?;

        Ok(Self { socket, source_mac })
    }

    pub(crate) fn send_ipv6(&self, group: Ipv6Addr, datagram: &[u8]) -> Result<usize, MctxError> {
        let header = ethernet_ipv6_header(group, self.source_mac);
        let vectors = [
            libc::iovec {
                iov_base: header.as_ptr().cast_mut().cast(),
                iov_len: header.len(),
            },
            libc::iovec {
                iov_base: datagram.as_ptr().cast_mut().cast(),
                iov_len: datagram.len(),
            },
        ];

        // SAFETY: both iovecs reference immutable buffers that remain valid for
        // the synchronous writev call. BPF treats one writev as one frame.
        let result = unsafe {
            libc::writev(
                self.socket.as_raw_fd(),
                vectors.as_ptr(),
                i32::try_from(vectors.len()).expect("two iovecs fit in c_int"),
            )
        };
        if result == -1 {
            return Err(MctxError::RawSendFailed(io::Error::last_os_error()));
        }

        let expected = header.len() + datagram.len();
        if result as usize != expected {
            return Err(MctxError::RawSendFailed(io::Error::new(
                ErrorKind::WriteZero,
                format!("partial macOS BPF send: wrote {result} of {expected} frame bytes"),
            )));
        }

        Ok(datagram.len())
    }
}

fn open_available_bpf_device() -> Result<Socket, MctxError> {
    for index in 0..256 {
        let path = CString::new(format!("/dev/bpf{index}")).expect("BPF path has no nul bytes");
        // SAFETY: path is a valid nul-terminated path and the returned
        // descriptor is immediately transferred to socket2 ownership.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };

        if fd != -1 {
            // SAFETY: fd is newly opened and uniquely owned here.
            return Ok(unsafe { Socket::from_raw_fd(fd) });
        }

        let error = io::Error::last_os_error();
        if error.kind() == ErrorKind::PermissionDenied {
            return Err(MctxError::RawSocketCreateFailed(error));
        }
        if error.raw_os_error() != Some(libc::EBUSY) && error.kind() != ErrorKind::NotFound {
            return Err(MctxError::RawSocketCreateFailed(error));
        }
    }

    Err(MctxError::RawSocketCreateFailed(io::Error::new(
        ErrorKind::NotFound,
        "no available /dev/bpf device found",
    )))
}

fn bind_bpf_to_interface(socket: &Socket, interface_name: &CStr) -> Result<(), MctxError> {
    // SAFETY: zero initializes all unused ifreq union storage.
    let mut request = unsafe { std::mem::zeroed::<libc::ifreq>() };
    let bytes = interface_name.to_bytes_with_nul();
    if bytes.len() > request.ifr_name.len() {
        return Err(MctxError::InterfaceDiscoveryFailed(format!(
            "interface name {} is too long for BPF",
            interface_name.to_string_lossy()
        )));
    }
    for (destination, source) in request.ifr_name.iter_mut().zip(bytes.iter().copied()) {
        *destination = source as libc::c_char;
    }

    // SAFETY: request contains a valid interface name for BIOCSETIF.
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            libc::BIOCSETIF,
            (&mut request as *mut libc::ifreq).cast::<libc::c_void>(),
        )
    };
    if result == -1 {
        Err(MctxError::RawSocketBindFailed(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn set_bpf_u32(socket: &Socket, request: libc::c_ulong, value: u32) -> Result<(), MctxError> {
    let mut value = value as libc::c_uint;
    // SAFETY: value is the c_uint payload required by this BPF ioctl.
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            request,
            (&mut value as *mut libc::c_uint).cast::<libc::c_void>(),
        )
    };
    if result == -1 {
        Err(MctxError::SocketOptionFailed(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

fn get_bpf_u32(socket: &Socket, request: libc::c_ulong) -> Result<u32, MctxError> {
    let mut value = 0 as libc::c_uint;
    // SAFETY: value is writable storage for the c_uint returned by the ioctl.
    let result = unsafe {
        libc::ioctl(
            socket.as_raw_fd(),
            request,
            (&mut value as *mut libc::c_uint).cast::<libc::c_void>(),
        )
    };
    if result == -1 {
        Err(MctxError::SocketOptionFailed(io::Error::last_os_error()))
    } else {
        Ok(value)
    }
}

fn ensure_ethernet_datalink(interface_name: &CStr, datalink: u32) -> Result<(), MctxError> {
    if datalink != libc::DLT_EN10MB {
        return Err(MctxError::RawUnsupportedLinkType(format!(
            "{} (macOS BPF DLT {datalink})",
            interface_name.to_string_lossy()
        )));
    }
    Ok(())
}

fn interface_name_from_index(interface_index: u32) -> Result<CString, MctxError> {
    let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
    // SAFETY: name is writable IFNAMSIZ storage.
    let pointer = unsafe { libc::if_indextoname(interface_index, name.as_mut_ptr()) };
    if pointer.is_null() {
        return Err(MctxError::InterfaceDiscoveryFailed(format!(
            "failed to resolve interface index {interface_index} to an interface name"
        )));
    }

    // SAFETY: if_indextoname returned a pointer into name containing a C string.
    Ok(unsafe { CStr::from_ptr(pointer) }.to_owned())
}

fn interface_mac(interface_index: u32, interface_name: &CStr) -> Result<[u8; 6], MctxError> {
    // SAFETY: getifaddrs initializes a linked list that remains valid until the
    // matching freeifaddrs call below.
    unsafe {
        let mut addresses = std::ptr::null_mut();
        if libc::getifaddrs(&mut addresses) != 0 {
            return Err(MctxError::InterfaceDiscoveryFailed(
                io::Error::last_os_error().to_string(),
            ));
        }

        let mut cursor = addresses;
        while !cursor.is_null() {
            let address = (*cursor).ifa_addr;
            if !address.is_null() && (*address).sa_family as i32 == libc::AF_LINK {
                let link = &*address.cast::<libc::sockaddr_dl>();
                if u32::from(link.sdl_index) == interface_index {
                    let address_offset =
                        offset_of!(libc::sockaddr_dl, sdl_data) + usize::from(link.sdl_nlen);
                    let address_end = address_offset + 6;
                    if link.sdl_alen == 6 && address_end <= usize::from(link.sdl_len) {
                        let mut mac = [0u8; 6];
                        std::ptr::copy_nonoverlapping(
                            address.cast::<u8>().add(address_offset),
                            mac.as_mut_ptr(),
                            mac.len(),
                        );
                        libc::freeifaddrs(addresses);
                        return Ok(mac);
                    }
                }
            }
            cursor = (*cursor).ifa_next;
        }

        libc::freeifaddrs(addresses);
    }

    Err(MctxError::RawUnsupportedLinkType(format!(
        "{} (no six-byte macOS link-layer address)",
        interface_name.to_string_lossy()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_ethernet_bpf_datalinks() {
        let name = CString::new("lo0").unwrap();
        let error = ensure_ethernet_datalink(&name, libc::DLT_NULL).unwrap_err();

        assert!(matches!(error, MctxError::RawUnsupportedLinkType(_)));
    }

    #[test]
    fn accepts_ethernet_bpf_datalinks() {
        let name = CString::new("en0").unwrap();
        assert!(ensure_ethernet_datalink(&name, libc::DLT_EN10MB).is_ok());
    }

    #[test]
    fn sockaddr_dl_layout_contains_link_address_storage() {
        assert!(
            offset_of!(libc::sockaddr_dl, sdl_data) + 6 <= std::mem::size_of::<libc::sockaddr_dl>()
        );
    }
}
