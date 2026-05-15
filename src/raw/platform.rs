#[cfg(target_os = "macos")]
use crate::config::ipv6_destination_scope_id;
use crate::config::{OutgoingInterface, PublicationAddressFamily};
use crate::error::MctxError;
use crate::platform::resolve_ipv4_interface_index;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::platform::resolve_ipv6_interface_index;
use crate::raw::RawValidationMode;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::raw::datagram::apply_ttl_or_hop_limit_override;
use crate::raw::datagram::{ParsedRawIpDatagram, parse_raw_ip_datagram};
use crate::raw::{RawPublicationConfig, RawPublicationId, RawSendReport};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::net::Ipv6Addr;
#[cfg(target_os = "macos")]
use std::net::SocketAddrV6;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::IPPROTO_RAW;

pub(crate) struct RawTransmitSocket {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    socket: Socket,
    family: PublicationAddressFamily,
    interface_index: Option<u32>,
    local_bind_addr: Option<IpAddr>,
}

impl std::fmt::Debug for RawTransmitSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawTransmitSocket")
            .field("family", &self.family)
            .field("interface_index", &self.interface_index)
            .field("local_bind_addr", &self.local_bind_addr)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn open_raw_transmit_socket(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    config.validate()?;

    if config.loopback.is_some() {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Linux raw packet transmit does not currently support explicit multicast loopback control".to_string(),
        ));
    }

    let interface_index = resolve_linux_transmit_interface_index(config)?;
    ensure_linux_ethernet_interface(interface_index)?;

    let protocol = packet_protocol(config.family);
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            protocol as i32,
        )
    };

    if raw_fd == -1 {
        return Err(MctxError::RawSocketCreateFailed(
            std::io::Error::last_os_error(),
        ));
    }

    let socket = unsafe { Socket::from_raw_fd(raw_fd) };
    let bind_addr = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: interface_index as i32,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    };

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

    Ok(RawTransmitSocket {
        socket,
        family: config.family.unwrap_or(PublicationAddressFamily::Ipv4),
        interface_index: Some(interface_index),
        local_bind_addr: config.bind_addr,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn open_raw_transmit_socket(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    config.validate()?;

    match infer_socket_family(config)? {
        PublicationAddressFamily::Ipv4 => open_macos_raw_socket_v4(config),
        PublicationAddressFamily::Ipv6 => open_macos_raw_socket_v6(config),
    }
}

#[cfg(windows)]
pub(crate) fn open_raw_transmit_socket(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    config.validate()?;

    match infer_socket_family(config)? {
        PublicationAddressFamily::Ipv4 => open_windows_raw_socket_v4(config),
        PublicationAddressFamily::Ipv6 => Err(MctxError::RawPacketTransmitUnsupported(
            "Windows raw multicast transmit currently supports IPv4 only".to_string(),
        )),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn open_raw_transmit_socket(
    _config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    Err(MctxError::RawPacketTransmitUnsupported(
        "raw multicast transmit is currently implemented on Linux, macOS, and Windows".to_string(),
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn send_raw_ip_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    ip_datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let parsed = parse_raw_ip_datagram(ip_datagram)?;
    validate_datagram_against_config(parsed, config)?;

    let destination_mac = match parsed.destination_ip {
        IpAddr::V4(group) if group.is_multicast() => ipv4_multicast_mac(group),
        IpAddr::V6(group) if group.is_multicast() => ipv6_multicast_mac(group),
        _ => match config.validation_mode {
            RawValidationMode::StrictMulticastDestination => {
                return Err(MctxError::InvalidRawMulticastDestination);
            }
            RawValidationMode::AllowAnyDestination => {
                return Err(MctxError::RawPacketTransmitUnsupported(
                    "Linux raw packet transmit currently supports multicast destinations only"
                        .to_string(),
                ));
            }
        },
    };

    let datagram_storage;
    let datagram = if let Some(ttl) = config.ttl {
        datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl);
        datagram_storage.as_slice()
    } else {
        ip_datagram
    };

    let protocol = packet_protocol(Some(parsed.family));
    let mut send_addr = libc::sockaddr_ll {
        sll_family: libc::AF_PACKET as u16,
        sll_protocol: protocol,
        sll_ifindex: socket
            .interface_index
            .expect("linux raw interface index is known") as i32,
        sll_hatype: libc::ARPHRD_ETHER,
        sll_pkttype: 0,
        sll_halen: destination_mac.len() as u8,
        sll_addr: [0; 8],
    };
    send_addr.sll_addr[..destination_mac.len()].copy_from_slice(&destination_mac);

    let result = unsafe {
        libc::sendto(
            socket.socket.as_raw_fd(),
            datagram.as_ptr().cast(),
            datagram.len(),
            0,
            (&send_addr as *const libc::sockaddr_ll).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };

    if result == -1 {
        return Err(MctxError::RawSendFailed(std::io::Error::last_os_error()));
    }

    Ok(raw_send_report(
        socket,
        publication_id,
        parsed,
        result as usize,
        config.outgoing_interface,
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn send_raw_ip_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    ip_datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let parsed = parse_raw_ip_datagram(ip_datagram)?;
    validate_datagram_against_config(parsed, config)?;

    if parsed.family != socket.family {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    let datagram = prepare_macos_datagram_for_send(ip_datagram, parsed, config.ttl);

    let bytes_sent = match parsed.destination_ip {
        IpAddr::V4(group) => socket
            .socket
            .send_to(&datagram, &SockAddr::from(SocketAddrV4::new(group, 0)))
            .map_err(MctxError::RawSendFailed)?,
        IpAddr::V6(group) => socket
            .socket
            .send_to(
                &datagram,
                &SockAddr::from(destination_sockaddr_v6(group, socket.interface_index)),
            )
            .map_err(MctxError::RawSendFailed)?,
    };

    Ok(raw_send_report(
        socket,
        publication_id,
        parsed,
        bytes_sent,
        config.outgoing_interface,
    ))
}

#[cfg(windows)]
pub(crate) fn send_raw_ip_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    ip_datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let parsed = parse_raw_ip_datagram(ip_datagram)?;
    validate_datagram_against_config(parsed, config)?;

    if parsed.family != PublicationAddressFamily::Ipv4 {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Windows raw multicast transmit currently supports IPv4 only".to_string(),
        ));
    }

    let datagram_storage;
    let datagram = if let Some(ttl) = config.ttl {
        datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl);
        datagram_storage.as_slice()
    } else {
        ip_datagram
    };

    let destination = match parsed.destination_ip {
        IpAddr::V4(group) => SocketAddrV4::new(group, 0),
        IpAddr::V6(_) => unreachable!("validated above"),
    };

    let bytes_sent = socket
        .socket
        .send_to(datagram, &SockAddr::from(destination))
        .map_err(MctxError::RawSendFailed)?;

    Ok(raw_send_report(
        socket,
        publication_id,
        parsed,
        bytes_sent,
        config.outgoing_interface,
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn send_raw_ip_datagram(
    _socket: &RawTransmitSocket,
    _publication_id: RawPublicationId,
    _config: &RawPublicationConfig,
    _ip_datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    Err(MctxError::RawPacketTransmitUnsupported(
        "raw multicast transmit is currently implemented on Linux, macOS, and Windows".to_string(),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn validate_datagram_against_config(
    parsed: ParsedRawIpDatagram,
    config: &RawPublicationConfig,
) -> Result<(), MctxError> {
    if let Some(expected_family) = config.family
        && expected_family != parsed.family
    {
        return Err(MctxError::InvalidRawIpDatagram);
    }

    if let Some(bind_addr) = config.bind_addr
        && !family_matches_ip(parsed.family, bind_addr)
    {
        return Err(MctxError::RawBindAddressFamilyMismatch);
    }

    if let Some(outgoing_interface) = config.outgoing_interface
        && !interface_matches_family(parsed.family, outgoing_interface)
    {
        return Err(MctxError::OutgoingInterfaceFamilyMismatch);
    }

    if matches!(
        config.validation_mode,
        RawValidationMode::StrictMulticastDestination
    ) && !parsed.destination_ip.is_multicast()
    {
        return Err(MctxError::InvalidRawMulticastDestination);
    }

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn raw_send_report(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    parsed: ParsedRawIpDatagram,
    bytes_sent: usize,
    outgoing_interface: Option<OutgoingInterface>,
) -> RawSendReport {
    RawSendReport {
        publication_id,
        family: parsed.family,
        source_ip: Some(parsed.source_ip),
        destination_ip: Some(parsed.destination_ip),
        ip_protocol: Some(parsed.protocol),
        bytes_sent,
        local_bind_addr: socket.local_bind_addr,
        outgoing_interface,
        outgoing_interface_index: socket.interface_index,
    }
}

#[cfg(any(target_os = "macos", windows))]
fn infer_socket_family(
    config: &RawPublicationConfig,
) -> Result<PublicationAddressFamily, MctxError> {
    config
        .family
        .or_else(|| config.bind_addr.map(ip_family))
        .or_else(|| outgoing_interface_family(config.outgoing_interface))
        .ok_or(MctxError::RawInterfaceRequired)
}

#[cfg(any(target_os = "macos", windows, test))]
fn ip_family(ip: IpAddr) -> PublicationAddressFamily {
    match ip {
        IpAddr::V4(_) => PublicationAddressFamily::Ipv4,
        IpAddr::V6(_) => PublicationAddressFamily::Ipv6,
    }
}

#[cfg(any(target_os = "macos", windows, test))]
fn outgoing_interface_family(
    outgoing_interface: Option<OutgoingInterface>,
) -> Option<PublicationAddressFamily> {
    match outgoing_interface {
        Some(OutgoingInterface::Ipv4Addr(_)) => Some(PublicationAddressFamily::Ipv4),
        Some(OutgoingInterface::Ipv6Addr(_) | OutgoingInterface::Ipv6Index(_)) => {
            Some(PublicationAddressFamily::Ipv6)
        }
        None => None,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn family_matches_ip(family: PublicationAddressFamily, ip: IpAddr) -> bool {
    matches!(
        (family, ip),
        (PublicationAddressFamily::Ipv4, IpAddr::V4(_))
            | (PublicationAddressFamily::Ipv6, IpAddr::V6(_))
    )
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn interface_matches_family(
    family: PublicationAddressFamily,
    outgoing_interface: OutgoingInterface,
) -> bool {
    matches!(
        (family, outgoing_interface),
        (
            PublicationAddressFamily::Ipv4,
            OutgoingInterface::Ipv4Addr(_)
        ) | (
            PublicationAddressFamily::Ipv6,
            OutgoingInterface::Ipv6Addr(_) | OutgoingInterface::Ipv6Index(_)
        )
    )
}

#[cfg(target_os = "linux")]
fn resolve_linux_transmit_interface_index(config: &RawPublicationConfig) -> Result<u32, MctxError> {
    let bind_index = config
        .bind_addr
        .map(resolve_interface_index_for_ip)
        .transpose()?;
    let outgoing_index = match config.outgoing_interface {
        Some(OutgoingInterface::Ipv4Addr(interface)) => {
            Some(resolve_ipv4_interface_index(interface)?)
        }
        Some(OutgoingInterface::Ipv6Addr(interface)) => {
            Some(resolve_ipv6_interface_index(interface)?)
        }
        Some(OutgoingInterface::Ipv6Index(index)) => Some(index),
        None => None,
    };

    if let (Some(bind_addr), Some(bind_index), Some(outgoing_index)) =
        (config.bind_addr, bind_index, outgoing_index)
        && bind_index != outgoing_index
    {
        return Err(MctxError::InterfaceDiscoveryFailed(format!(
            "raw bind address {bind_addr} resolves to interface index {bind_index}, expected {outgoing_index}"
        )));
    }

    bind_index
        .or(outgoing_index)
        .ok_or(MctxError::RawInterfaceRequired)
}

#[cfg(target_os = "linux")]
fn packet_protocol(family: Option<PublicationAddressFamily>) -> u16 {
    match family {
        Some(PublicationAddressFamily::Ipv4) => (libc::ETH_P_IP as u16).to_be(),
        Some(PublicationAddressFamily::Ipv6) => (libc::ETH_P_IPV6 as u16).to_be(),
        None => (libc::ETH_P_ALL as u16).to_be(),
    }
}

#[cfg(target_os = "linux")]
const LINUX_ARPHRD_ETHER: i32 = 1;

#[cfg(target_os = "linux")]
fn ensure_linux_ethernet_interface(interface_index: u32) -> Result<(), MctxError> {
    let (interface_name, link_type) = linux_link_info(interface_index)?;
    if link_type != LINUX_ARPHRD_ETHER {
        return Err(MctxError::RawUnsupportedLinkType(format!(
            "{interface_name} (linux ARPHRD {link_type})"
        )));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_link_info(interface_index: u32) -> Result<(String, i32), MctxError> {
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
                    let link_type = sockaddr.sll_hatype as i32;
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

#[cfg(target_os = "linux")]
fn resolve_interface_index_for_ip(ip: IpAddr) -> Result<u32, MctxError> {
    match ip {
        IpAddr::V4(interface) => resolve_ipv4_interface_index(interface),
        IpAddr::V6(interface) => resolve_ipv6_interface_index(interface),
    }
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v4(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let socket = Socket::new(
        Domain::IPV4,
        Type::RAW,
        Some(Protocol::from(libc::IPPROTO_RAW)),
    )
    .map_err(MctxError::RawSocketCreateFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_header_included_v4(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .bind(&SockAddr::from(SocketAddrV4::new(selection.bind_addr, 0)))
        .map_err(MctxError::RawSocketBindFailed)?;
    socket
        .set_multicast_if_v4(&selection.interface_addr)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(loopback) = config.loopback {
        socket
            .set_multicast_loop_v4(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(RawTransmitSocket {
        socket,
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config)?;
    let socket = Socket::new(
        Domain::IPV6,
        Type::RAW,
        Some(Protocol::from(libc::IPPROTO_RAW)),
    )
    .map_err(MctxError::RawSocketCreateFailed)?;

    socket
        .set_only_v6(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_header_included_v6(true)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(bind_addr) = selection.bind_addr {
        socket
            .bind(&SockAddr::from(SocketAddrV6::new(
                bind_addr,
                0,
                0,
                selection.bind_scope_id,
            )))
            .map_err(MctxError::RawSocketBindFailed)?;
    }

    socket
        .set_multicast_if_v6(selection.interface_index)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(loopback) = config.loopback {
        socket
            .set_multicast_loop_v6(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(RawTransmitSocket {
        socket,
        family: PublicationAddressFamily::Ipv6,
        interface_index: Some(selection.interface_index),
        local_bind_addr: selection.bind_addr.map(IpAddr::V6),
    })
}

#[cfg(windows)]
fn open_windows_raw_socket_v4(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let socket = Socket::new(
        Domain::IPV4,
        Type::RAW,
        Some(Protocol::from(IPPROTO_RAW as i32)),
    )
    .map_err(MctxError::RawSocketCreateFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_header_included_v4(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .bind(&SockAddr::from(SocketAddrV4::new(selection.bind_addr, 0)))
        .map_err(MctxError::RawSocketBindFailed)?;
    socket
        .set_multicast_if_v4(&selection.interface_addr)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(loopback) = config.loopback {
        socket
            .set_multicast_loop_v4(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(RawTransmitSocket {
        socket,
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
    })
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Clone, Copy)]
struct RawIpv4Selection {
    bind_addr: Ipv4Addr,
    interface_addr: Ipv4Addr,
    interface_index: u32,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct RawIpv6Selection {
    bind_addr: Option<Ipv6Addr>,
    bind_scope_id: u32,
    interface_index: u32,
}

#[cfg(any(target_os = "macos", windows))]
fn resolve_raw_ipv4_selection(
    config: &RawPublicationConfig,
) -> Result<RawIpv4Selection, MctxError> {
    let bind_addr = config.bind_addr.and_then(|ip| match ip {
        IpAddr::V4(addr) => Some(addr),
        IpAddr::V6(_) => None,
    });
    let interface_addr = match config.outgoing_interface {
        Some(OutgoingInterface::Ipv4Addr(addr)) => Some(addr),
        Some(OutgoingInterface::Ipv6Addr(_) | OutgoingInterface::Ipv6Index(_)) => None,
        None => None,
    }
    .or(bind_addr)
    .ok_or(MctxError::RawInterfaceRequired)?;

    let bind_addr = bind_addr.unwrap_or(interface_addr);
    let bind_index = resolve_ipv4_interface_index(bind_addr)?;
    let interface_index = resolve_ipv4_interface_index(interface_addr)?;

    if bind_index != interface_index {
        return Err(MctxError::InterfaceDiscoveryFailed(format!(
            "raw bind address {bind_addr} resolves to interface index {bind_index}, expected {interface_index}"
        )));
    }

    Ok(RawIpv4Selection {
        bind_addr,
        interface_addr,
        interface_index,
    })
}

#[cfg(target_os = "macos")]
fn resolve_raw_ipv6_selection(
    config: &RawPublicationConfig,
) -> Result<RawIpv6Selection, MctxError> {
    let bind_addr = config
        .bind_addr
        .and_then(|ip| match ip {
            IpAddr::V6(addr) => Some(addr),
            IpAddr::V4(_) => None,
        })
        .or(match config.outgoing_interface {
            Some(OutgoingInterface::Ipv6Addr(addr)) => Some(addr),
            _ => None,
        });

    let explicit_interface_index = match config.outgoing_interface {
        Some(OutgoingInterface::Ipv6Index(index)) => Some(index),
        Some(OutgoingInterface::Ipv6Addr(addr)) => Some(resolve_ipv6_interface_index(addr)?),
        _ => None,
    };

    let bind_interface_index = bind_addr.map(resolve_ipv6_interface_index).transpose()?;

    if let (Some(bind_addr), Some(bind_interface_index), Some(explicit_interface_index)) =
        (bind_addr, bind_interface_index, explicit_interface_index)
        && bind_interface_index != explicit_interface_index
    {
        return Err(MctxError::Ipv6SourceInterfaceMismatch {
            source_addr: IpAddr::V6(bind_addr),
            source_interface_index: bind_interface_index,
            outgoing_interface_index: explicit_interface_index,
        });
    }

    let interface_index = explicit_interface_index
        .or(bind_interface_index)
        .ok_or(MctxError::RawInterfaceRequired)?;
    let bind_scope_id = match bind_addr {
        Some(addr) if addr.is_unicast_link_local() => interface_index,
        _ => 0,
    };

    Ok(RawIpv6Selection {
        bind_addr,
        bind_scope_id,
        interface_index,
    })
}

#[cfg(target_os = "macos")]
fn destination_sockaddr_v6(group: Ipv6Addr, interface_index: Option<u32>) -> SocketAddrV6 {
    let scope_id = if group.is_multicast() {
        ipv6_destination_scope_id(group, interface_index.unwrap_or(0))
    } else if group.is_unicast_link_local() {
        interface_index.unwrap_or(0)
    } else {
        0
    };

    SocketAddrV6::new(group, 0, 0, scope_id)
}

#[cfg(target_os = "macos")]
fn prepare_macos_datagram_for_send(
    ip_datagram: &[u8],
    parsed: ParsedRawIpDatagram,
    ttl_override: Option<u8>,
) -> Vec<u8> {
    let mut datagram = if let Some(ttl) = ttl_override {
        apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl)
    } else {
        ip_datagram.to_vec()
    };

    if parsed.family == PublicationAddressFamily::Ipv4 {
        normalize_macos_ipv4_header_for_hdrincl(&mut datagram);
    }

    datagram
}

#[cfg(target_os = "macos")]
fn normalize_macos_ipv4_header_for_hdrincl(datagram: &mut [u8]) {
    if datagram.len() < 20 {
        return;
    }

    let ip_len = u16::from_be_bytes([datagram[2], datagram[3]]);
    let ip_off = u16::from_be_bytes([datagram[6], datagram[7]]);

    datagram[2..4].copy_from_slice(&ip_len.to_ne_bytes());
    datagram[6..8].copy_from_slice(&ip_off.to_ne_bytes());

    // macOS computes the IPv4 header checksum for IP_HDRINCL sends after it
    // interprets the BSD-style host-order header fields above.
    datagram[10] = 0;
    datagram[11] = 0;
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn ipv4_multicast_mac(group: Ipv4Addr) -> [u8; 6] {
    let octets = group.octets();
    [0x01, 0x00, 0x5e, octets[1] & 0x7f, octets[2], octets[3]]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn ipv6_multicast_mac(group: Ipv6Addr) -> [u8; 6] {
    let octets = group.octets();
    [0x33, 0x33, octets[12], octets[13], octets[14], octets[15]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{RawPublicationConfig, RawValidationMode};

    #[test]
    fn derives_ipv4_multicast_destination_mac() {
        assert_eq!(
            ipv4_multicast_mac(Ipv4Addr::new(239, 1, 2, 3)),
            [0x01, 0x00, 0x5e, 0x01, 0x02, 0x03]
        );
    }

    #[test]
    fn derives_ipv6_multicast_destination_mac() {
        assert_eq!(
            ipv6_multicast_mac("ff3e::8000:1234".parse::<Ipv6Addr>().unwrap()),
            [0x33, 0x33, 0x80, 0x00, 0x12, 0x34]
        );
    }

    #[test]
    fn strict_mode_rejects_non_multicast_destination() {
        let datagram = build_ipv4_datagram(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        let config = RawPublicationConfig::ipv4();

        assert!(matches!(
            validate_datagram_against_config(parsed, &config),
            Err(MctxError::InvalidRawMulticastDestination)
        ));
    }

    #[test]
    fn allow_any_destination_skips_early_multicast_validation() {
        let datagram = build_ipv4_datagram(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        let config = RawPublicationConfig::ipv4()
            .with_validation_mode(RawValidationMode::AllowAnyDestination);

        assert!(validate_datagram_against_config(parsed, &config).is_ok());
    }

    #[test]
    fn outgoing_interface_family_hint_is_inferred_without_explicit_family() {
        assert_eq!(
            outgoing_interface_family(Some(OutgoingInterface::Ipv6Index(7))),
            Some(PublicationAddressFamily::Ipv6)
        );
        assert_eq!(
            outgoing_interface_family(Some(OutgoingInterface::Ipv4Addr(Ipv4Addr::new(
                192, 168, 1, 20
            )))),
            Some(PublicationAddressFamily::Ipv4)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ipv4_hdrincl_header_is_normalized_to_host_order() {
        let datagram = build_ipv4_datagram(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(239, 1, 2, 3));
        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        let normalized = prepare_macos_datagram_for_send(&datagram, parsed, None);

        assert_eq!(
            &normalized[2..4],
            &u16::from_be_bytes([datagram[2], datagram[3]]).to_ne_bytes()
        );
        assert_eq!(
            &normalized[6..8],
            &u16::from_be_bytes([datagram[6], datagram[7]]).to_ne_bytes()
        );
        assert_eq!(&normalized[10..12], &[0, 0]);
    }

    fn build_ipv4_datagram(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
        let mut datagram = vec![
            0x45, 0x00, 0x00, 0x14, 0x12, 0x34, 0x00, 0x00, 0x01, 0x11, 0x00, 0x00,
        ];
        datagram.extend_from_slice(&source.octets());
        datagram.extend_from_slice(&destination.octets());
        datagram
    }
}
