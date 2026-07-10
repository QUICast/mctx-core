use crate::config::PublicationAddressFamily;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::config::{Ipv6MulticastScope, ipv6_multicast_scope};
use crate::error::MctxError;
use crate::raw_ip::RawIpPublicationId;
use crate::raw_ip::config::{RawIpSocketConfig, family_matches_ip};
use crate::raw_ip::datagram::{ParsedRawIpDatagram, parse_complete_ip_datagram};
use crate::raw_ip::report::RawIpSendReport;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::socket_cache::BoundedSocketCache;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
#[cfg(target_os = "macos")]
use std::io::IoSlice;
use std::net::IpAddr;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::net::Ipv6Addr;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::net::SocketAddrV4;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::net::SocketAddrV6;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::num::NonZeroU32;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::sync::{Arc, Mutex};

/// Platform-owned sockets for one generic raw-IP publication.
pub(crate) struct RawIpTransmitSocket {
    family: PublicationAddressFamily,
    selection: RawIpSelection,
    #[cfg(target_os = "linux")]
    ipv4_socket: Option<Socket>,
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    protocol_sockets: RawIpProtocolSocketCache,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
enum RawIpProtocolSocketCache {
    #[cfg(target_os = "linux")]
    None,
    #[cfg(any(target_os = "macos", windows))]
    Ipv4(Mutex<BoundedSocketCache<u8>>),
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Ipv6(Mutex<BoundedSocketCache<RawIpv6ProtocolSocketKey>>),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RawIpv6ProtocolSocketKey {
    protocol: u8,
    hop_limit: u8,
    traffic_class: u8,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
impl RawIpProtocolSocketCache {
    fn len(&self) -> usize {
        match self {
            #[cfg(target_os = "linux")]
            Self::None => 0,
            #[cfg(any(target_os = "macos", windows))]
            Self::Ipv4(cache) => lock_recover(cache).len(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Self::Ipv6(cache) => lock_recover(cache).len(),
        }
    }
}

impl std::fmt::Debug for RawIpTransmitSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("RawIpTransmitSocket");
        debug
            .field("family", &self.family)
            .field("selection", &self.selection);
        #[cfg(target_os = "linux")]
        debug.field("has_ipv4_socket", &self.ipv4_socket.is_some());
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        debug.field("cached_protocol_socket_count", &self.protocol_sockets.len());
        debug.finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
struct RawIpSelection {
    family: PublicationAddressFamily,
    bind_addr: Option<IpAddr>,
    interface_addr: Option<IpAddr>,
    interface_index: u32,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Opens and pins a raw-IP socket to its required egress interface.
#[cfg(target_os = "linux")]
pub(crate) fn open_raw_ip_socket(
    config: &RawIpSocketConfig,
) -> Result<RawIpTransmitSocket, MctxError> {
    let selection = resolve_selection(config)?;
    require_unix_ipv6_bind_addr(&selection)?;

    match selection.family {
        PublicationAddressFamily::Ipv4 => Ok(RawIpTransmitSocket {
            family: selection.family,
            selection,
            ipv4_socket: Some(open_unix_raw_socket(&selection, libc::IPPROTO_RAW)?),
            protocol_sockets: RawIpProtocolSocketCache::None,
        }),
        PublicationAddressFamily::Ipv6 => {
            let probe = open_unix_raw_socket(&selection, libc::IPPROTO_RAW)?;
            drop(probe);
            Ok(RawIpTransmitSocket {
                family: selection.family,
                selection,
                ipv4_socket: None,
                protocol_sockets: RawIpProtocolSocketCache::Ipv6(Mutex::new(
                    BoundedSocketCache::default(),
                )),
            })
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn open_raw_ip_socket(
    config: &RawIpSocketConfig,
) -> Result<RawIpTransmitSocket, MctxError> {
    let selection = resolve_selection(config)?;
    require_unix_ipv6_bind_addr(&selection)?;

    match selection.family {
        PublicationAddressFamily::Ipv4 => {
            let probe = open_unix_raw_socket(&selection, libc::IPPROTO_RAW)?;
            drop(probe);
            Ok(RawIpTransmitSocket {
                family: selection.family,
                selection,
                protocol_sockets: RawIpProtocolSocketCache::Ipv4(Mutex::new(
                    BoundedSocketCache::default(),
                )),
            })
        }
        PublicationAddressFamily::Ipv6 => {
            let probe = open_unix_raw_socket(&selection, libc::IPPROTO_RAW)?;
            drop(probe);
            Ok(RawIpTransmitSocket {
                family: selection.family,
                selection,
                protocol_sockets: RawIpProtocolSocketCache::Ipv6(Mutex::new(
                    BoundedSocketCache::default(),
                )),
            })
        }
    }
}

#[cfg(windows)]
pub(crate) fn open_raw_ip_socket(
    config: &RawIpSocketConfig,
) -> Result<RawIpTransmitSocket, MctxError> {
    let selection = resolve_selection(config)?;
    if selection.family != PublicationAddressFamily::Ipv4 {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Windows raw-IP transmit supports IPv4 only; raw IPv6 full-header transmit is unavailable"
                .to_string(),
        ));
    }

    let probe = open_windows_raw_ipv4_socket(
        &selection,
        windows_sys::Win32::Networking::WinSock::IPPROTO_RAW,
    )?;
    drop(probe);
    Ok(RawIpTransmitSocket {
        family: selection.family,
        selection,
        protocol_sockets: RawIpProtocolSocketCache::Ipv4(Mutex::new(BoundedSocketCache::default())),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn open_raw_ip_socket(
    _config: &RawIpSocketConfig,
) -> Result<RawIpTransmitSocket, MctxError> {
    Err(MctxError::RawPacketTransmitUnsupported(
        "raw-IP transmit is implemented on Linux and macOS for IPv4/IPv6, and on Windows for IPv4 only"
            .to_string(),
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn send_ip_datagram(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    let parsed = parse_for_socket(socket, ip_datagram)?;
    match parsed.family {
        PublicationAddressFamily::Ipv4 => {
            let destination = ipv4_destination(parsed)?;
            let send_socket = socket
                .ipv4_socket
                .as_ref()
                .expect("Linux IPv4 raw-IP publications open their socket eagerly");
            let bytes_sent = send_socket
                .send_to(ip_datagram, &SockAddr::from(destination))
                .map_err(MctxError::RawSendFailed)?;
            ensure_full_send(bytes_sent, ip_datagram.len(), "raw IPv4")?;
            Ok(raw_ip_send_report(
                socket,
                publication_id,
                parsed,
                bytes_sent,
            ))
        }
        PublicationAddressFamily::Ipv6 => {
            send_unix_raw_ipv6_datagram(socket, publication_id, parsed, ip_datagram)
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn send_ip_datagram(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    let parsed = parse_for_socket(socket, ip_datagram)?;
    match parsed.family {
        PublicationAddressFamily::Ipv4 => {
            send_macos_raw_ipv4_datagram(socket, publication_id, parsed, ip_datagram)
        }
        PublicationAddressFamily::Ipv6 => {
            send_unix_raw_ipv6_datagram(socket, publication_id, parsed, ip_datagram)
        }
    }
}

#[cfg(windows)]
pub(crate) fn send_ip_datagram(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    let parsed = parse_for_socket(socket, ip_datagram)?;
    if parsed.family != PublicationAddressFamily::Ipv4 {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Windows raw-IP transmit supports IPv4 only; raw IPv6 full-header transmit is unavailable"
                .to_string(),
        ));
    }

    let destination = ipv4_destination(parsed)?;
    let send_socket = cached_ipv4_protocol_socket(socket, parsed.protocol, || {
        open_windows_raw_ipv4_socket(&socket.selection, i32::from(parsed.protocol))
    })?;
    let bytes_sent = send_socket
        .send_to(ip_datagram, &SockAddr::from(destination))
        .map_err(MctxError::RawSendFailed)?;
    ensure_full_send(bytes_sent, ip_datagram.len(), "raw IPv4")?;
    Ok(raw_ip_send_report(
        socket,
        publication_id,
        parsed,
        bytes_sent,
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn send_ip_datagram(
    _socket: &RawIpTransmitSocket,
    _publication_id: RawIpPublicationId,
    _ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    Err(MctxError::RawPacketTransmitUnsupported(
        "raw-IP transmit is unsupported on this platform".to_string(),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn resolve_selection(config: &RawIpSocketConfig) -> Result<RawIpSelection, MctxError> {
    let family = config.resolved_family()?;
    let mut resolved_index = config.interface_index;

    for selector in [config.bind_addr, config.interface_addr]
        .into_iter()
        .flatten()
    {
        if !family_matches_ip(family, selector) {
            return Err(MctxError::OutgoingInterfaceFamilyMismatch);
        }

        let candidate = resolve_interface_index(selector)?;
        if let Some(existing) = resolved_index
            && existing != candidate
        {
            return Err(MctxError::InterfaceDiscoveryFailed(format!(
                "raw-IP selectors resolve to different interface indices {existing} and {candidate}"
            )));
        }
        resolved_index = Some(candidate);
    }

    let interface_index = resolved_index.ok_or(MctxError::RawInterfaceRequired)?;
    Ok(RawIpSelection {
        family,
        bind_addr: config.bind_addr,
        interface_addr: config.interface_addr,
        interface_index,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn resolve_interface_index(ip: IpAddr) -> Result<u32, MctxError> {
    match ip {
        IpAddr::V4(addr) => crate::platform::resolve_ipv4_interface_index(addr),
        IpAddr::V6(addr) => crate::platform::resolve_ipv6_interface_index(addr),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_unix_ipv6_bind_addr(selection: &RawIpSelection) -> Result<(), MctxError> {
    if selection.family == PublicationAddressFamily::Ipv6
        && !matches!(selection.bind_addr, Some(IpAddr::V6(_)))
    {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "raw IPv6 transmit requires a concrete local bind address so the kernel cannot select a different source"
                .to_string(),
        ));
    }

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_unix_raw_socket(selection: &RawIpSelection, protocol: i32) -> Result<Socket, MctxError> {
    let domain = match selection.family {
        PublicationAddressFamily::Ipv4 => Domain::IPV4,
        PublicationAddressFamily::Ipv6 => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::RAW, Some(Protocol::from(protocol)))
        .map_err(MctxError::RawSocketCreateFailed)?;
    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;

    match selection.family {
        PublicationAddressFamily::Ipv4 => {
            socket
                .set_header_included_v4(true)
                .map_err(MctxError::SocketOptionFailed)?;
            if let Some(IpAddr::V4(bind_addr)) = selection.bind_addr {
                socket
                    .bind(&SockAddr::from(SocketAddrV4::new(bind_addr, 0)))
                    .map_err(MctxError::RawSocketBindFailed)?;
            }
            bind_unix_socket_to_interface_v4(&socket, selection.interface_index)?;
        }
        PublicationAddressFamily::Ipv6 => {
            if let Some(IpAddr::V6(bind_addr)) = selection.bind_addr {
                let scope_id =
                    u32::from(bind_addr.is_unicast_link_local()) * selection.interface_index;
                socket
                    .bind(&SockAddr::from(SocketAddrV6::new(
                        bind_addr, 0, 0, scope_id,
                    )))
                    .map_err(MctxError::RawSocketBindFailed)?;
            }
            bind_unix_socket_to_interface_v6(&socket, selection.interface_index)?;
        }
    }

    Ok(socket)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bind_unix_socket_to_interface_v4(
    socket: &Socket,
    interface_index: u32,
) -> Result<(), MctxError> {
    let interface_index =
        NonZeroU32::new(interface_index).ok_or(MctxError::RawInterfaceRequired)?;
    socket
        .bind_device_by_index_v4(Some(interface_index))
        .map_err(MctxError::SocketOptionFailed)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn bind_unix_socket_to_interface_v6(
    socket: &Socket,
    interface_index: u32,
) -> Result<(), MctxError> {
    let interface_index =
        NonZeroU32::new(interface_index).ok_or(MctxError::RawInterfaceRequired)?;
    socket
        .bind_device_by_index_v6(Some(interface_index))
        .map_err(MctxError::SocketOptionFailed)
}

#[cfg(windows)]
fn open_windows_raw_ipv4_socket(
    selection: &RawIpSelection,
    protocol: i32,
) -> Result<Socket, MctxError> {
    let socket = Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::from(protocol)))
        .map_err(MctxError::RawSocketCreateFailed)?;
    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_header_included_v4(true)
        .map_err(MctxError::SocketOptionFailed)?;
    if let Some(IpAddr::V4(bind_addr)) = selection.bind_addr {
        socket
            .bind(&SockAddr::from(SocketAddrV4::new(bind_addr, 0)))
            .map_err(MctxError::RawSocketBindFailed)?;
    }
    set_windows_unicast_interface(&socket, selection.interface_index)?;
    Ok(socket)
}

#[cfg(windows)]
fn set_windows_unicast_interface(socket: &Socket, interface_index: u32) -> Result<(), MctxError> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        IP_UNICAST_IF, IPPROTO_IP, SOCKET_ERROR, setsockopt,
    };

    // Windows requires the interface index in network byte order for
    // IP_UNICAST_IF. This pins routing to the selected interface.
    let interface_index = interface_index.to_be();
    let raw_socket = usize::try_from(socket.as_raw_socket()).map_err(|_| {
        MctxError::SocketOptionFailed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "raw socket handle does not fit WinSock SOCKET",
        ))
    })?;
    // SAFETY: the socket is valid, the option receives a correctly sized u32,
    // and the pointer remains valid for the duration of the synchronous call.
    let result = unsafe {
        setsockopt(
            raw_socket,
            IPPROTO_IP,
            IP_UNICAST_IF,
            (&interface_index as *const u32).cast(),
            std::mem::size_of::<u32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        return Err(MctxError::SocketOptionFailed(
            std::io::Error::last_os_error(),
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn send_macos_raw_ipv4_datagram(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    parsed: ParsedRawIpDatagram,
    ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    let destination = ipv4_destination(parsed)?;
    let (header, header_len) = prepare_macos_ipv4_header(ip_datagram, parsed);
    let send_socket = cached_ipv4_protocol_socket(socket, parsed.protocol, || {
        open_unix_raw_socket(&socket.selection, i32::from(parsed.protocol))
    })?;
    let buffers = [
        IoSlice::new(&header[..header_len]),
        IoSlice::new(&ip_datagram[header_len..]),
    ];
    let bytes_sent = send_socket
        .send_to_vectored(&buffers, &SockAddr::from(destination))
        .map_err(MctxError::RawSendFailed)?;
    ensure_full_send(bytes_sent, ip_datagram.len(), "raw IPv4")?;
    Ok(raw_ip_send_report(
        socket,
        publication_id,
        parsed,
        bytes_sent,
    ))
}

#[cfg(target_os = "macos")]
fn prepare_macos_ipv4_header(ip_datagram: &[u8], parsed: ParsedRawIpDatagram) -> ([u8; 60], usize) {
    let mut header = [0u8; 60];
    header[..parsed.header_len].copy_from_slice(&ip_datagram[..parsed.header_len]);

    // Darwin's IP_HDRINCL ABI consumes ip_len and ip_off in host order and
    // computes the IPv4 header checksum itself. The caller slice is unchanged.
    let total_len = u16::from_be_bytes([header[2], header[3]]);
    let fragment_offset = u16::from_be_bytes([header[6], header[7]]);
    header[2..4].copy_from_slice(&total_len.to_ne_bytes());
    header[6..8].copy_from_slice(&fragment_offset.to_ne_bytes());
    header[10] = 0;
    header[11] = 0;

    (header, parsed.header_len)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn send_unix_raw_ipv6_datagram(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    parsed: ParsedRawIpDatagram,
    ip_datagram: &[u8],
) -> Result<RawIpSendReport, MctxError> {
    let destination_ip = validate_unix_ipv6_source(&socket.selection, parsed)?;

    let payload = ip_datagram
        .get(parsed.header_len..)
        .ok_or(MctxError::InvalidRawIpDatagram)?;
    let destination = SocketAddrV6::new(
        destination_ip,
        0,
        0,
        ipv6_destination_scope_id(destination_ip, socket.selection.interface_index),
    );
    let cache_key = RawIpv6ProtocolSocketKey {
        protocol: parsed.protocol,
        hop_limit: parsed.ttl_or_hop_limit,
        traffic_class: parsed.traffic_class,
    };
    let send_socket = cached_ipv6_protocol_socket(socket, cache_key, || {
        let socket = open_unix_raw_socket(&socket.selection, i32::from(parsed.protocol))?;
        socket
            .set_unicast_hops_v6(u32::from(parsed.ttl_or_hop_limit))
            .map_err(MctxError::SocketOptionFailed)?;
        socket
            .set_tclass_v6(u32::from(parsed.traffic_class))
            .map_err(MctxError::SocketOptionFailed)?;
        Ok(socket)
    })?;
    let bytes_sent = send_socket
        .send_to(payload, &SockAddr::from(destination))
        .map_err(MctxError::RawSendFailed)?;
    ensure_full_send(bytes_sent, payload.len(), "raw IPv6 payload")?;

    Ok(raw_ip_send_report(
        socket,
        publication_id,
        parsed,
        ip_datagram.len(),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_unix_ipv6_source(
    selection: &RawIpSelection,
    parsed: ParsedRawIpDatagram,
) -> Result<Ipv6Addr, MctxError> {
    let configured_bind_addr = match selection.bind_addr {
        Some(IpAddr::V6(bind_addr)) => bind_addr,
        _ => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "raw IPv6 transmit requires a concrete local bind address so the kernel cannot select a different source"
                    .to_string(),
            ));
        }
    };
    let (source_ip, destination_ip) = match (parsed.source_ip, parsed.destination_ip) {
        (IpAddr::V6(source_ip), IpAddr::V6(destination_ip)) => (source_ip, destination_ip),
        _ => return Err(MctxError::InvalidRawIpDatagram),
    };
    if source_ip != configured_bind_addr {
        return Err(MctxError::RawDatagramSourceMismatch {
            datagram_source: IpAddr::V6(source_ip),
            configured_bind_addr: IpAddr::V6(configured_bind_addr),
        });
    }

    Ok(destination_ip)
}

#[cfg(any(target_os = "macos", windows))]
fn cached_ipv4_protocol_socket(
    socket: &RawIpTransmitSocket,
    protocol: u8,
    open: impl FnOnce() -> Result<Socket, MctxError>,
) -> Result<Arc<Socket>, MctxError> {
    #[cfg(windows)]
    let RawIpProtocolSocketCache::Ipv4(cache) = &socket.protocol_sockets;
    #[cfg(target_os = "macos")]
    let cache = match &socket.protocol_sockets {
        RawIpProtocolSocketCache::Ipv4(cache) => cache,
        RawIpProtocolSocketCache::Ipv6(_) => {
            unreachable!("raw IPv4 protocol cache used by an IPv6 publication")
        }
    };
    lock_recover(cache).get_or_try_insert_with(protocol, open)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cached_ipv6_protocol_socket(
    socket: &RawIpTransmitSocket,
    key: RawIpv6ProtocolSocketKey,
    open: impl FnOnce() -> Result<Socket, MctxError>,
) -> Result<Arc<Socket>, MctxError> {
    let cache = match &socket.protocol_sockets {
        RawIpProtocolSocketCache::Ipv6(cache) => cache,
        #[cfg(target_os = "linux")]
        RawIpProtocolSocketCache::None => {
            unreachable!("raw IPv6 protocol cache used by an IPv4 publication")
        }
        #[cfg(target_os = "macos")]
        RawIpProtocolSocketCache::Ipv4(_) => {
            unreachable!("raw IPv6 protocol cache used by an IPv4 publication")
        }
    };
    lock_recover(cache).get_or_try_insert_with(key, open)
}

fn parse_for_socket(
    socket: &RawIpTransmitSocket,
    ip_datagram: &[u8],
) -> Result<ParsedRawIpDatagram, MctxError> {
    let parsed = parse_complete_ip_datagram(ip_datagram)?;
    if parsed.family != socket.family {
        return Err(MctxError::InvalidRawIpDatagram);
    }
    Ok(parsed)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn ipv4_destination(parsed: ParsedRawIpDatagram) -> Result<SocketAddrV4, MctxError> {
    match parsed.destination_ip {
        IpAddr::V4(destination) => Ok(SocketAddrV4::new(destination, 0)),
        IpAddr::V6(_) => Err(MctxError::InvalidRawIpDatagram),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ipv6_destination_scope_id(destination: Ipv6Addr, interface_index: u32) -> u32 {
    if destination.is_unicast_link_local() {
        return interface_index;
    }

    match ipv6_multicast_scope(destination) {
        Some(Ipv6MulticastScope::InterfaceLocal | Ipv6MulticastScope::LinkLocal) => interface_index,
        _ => 0,
    }
}

fn ensure_full_send(bytes_sent: usize, expected: usize, transport: &str) -> Result<(), MctxError> {
    if bytes_sent != expected {
        return Err(MctxError::RawSendFailed(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            format!("partial {transport} send: wrote {bytes_sent} of {expected} bytes"),
        )));
    }

    Ok(())
}

fn raw_ip_send_report(
    socket: &RawIpTransmitSocket,
    publication_id: RawIpPublicationId,
    parsed: ParsedRawIpDatagram,
    bytes_sent: usize,
) -> RawIpSendReport {
    RawIpSendReport {
        publication_id,
        family: parsed.family,
        source_ip: parsed.source_ip,
        destination_ip: parsed.destination_ip,
        ip_protocol: parsed.protocol,
        bytes_sent,
        local_bind_addr: socket.selection.bind_addr,
        interface_addr: socket.selection.interface_addr,
        interface_index: socket.selection.interface_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_send_rejects_partial_datagrams() {
        assert!(matches!(
            ensure_full_send(7, 8, "test"),
            Err(MctxError::RawSendFailed(error)) if error.kind() == std::io::ErrorKind::WriteZero
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn ipv6_destination_scope_is_limited_to_link_scopes() {
        assert_eq!(ipv6_destination_scope_id("fe80::1".parse().unwrap(), 7), 7);
        assert_eq!(
            ipv6_destination_scope_id("2001:db8::1".parse().unwrap(), 7),
            0
        );
        assert_eq!(
            ipv6_destination_scope_id("ff32::1234".parse().unwrap(), 7),
            7
        );
        assert_eq!(
            ipv6_destination_scope_id("ff3e::1234".parse().unwrap(), 7),
            0
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unix_ipv6_rejects_a_source_that_would_be_rewritten() {
        let configured_source: Ipv6Addr = "2001:db8::10".parse().unwrap();
        let datagram_source: Ipv6Addr = "2001:db8::20".parse().unwrap();
        let parsed = ParsedRawIpDatagram {
            family: PublicationAddressFamily::Ipv6,
            source_ip: IpAddr::V6(datagram_source),
            destination_ip: IpAddr::V6("2001:db8::30".parse().unwrap()),
            protocol: 58,
            header_len: 40,
            ttl_or_hop_limit: 64,
            traffic_class: 0,
        };
        let selection = RawIpSelection {
            family: PublicationAddressFamily::Ipv6,
            bind_addr: Some(IpAddr::V6(configured_source)),
            interface_addr: None,
            interface_index: 7,
        };

        assert!(matches!(
            validate_unix_ipv6_source(&selection, parsed),
            Err(MctxError::RawDatagramSourceMismatch {
                datagram_source: IpAddr::V6(actual),
                configured_bind_addr: IpAddr::V6(configured),
            }) if actual == datagram_source && configured == configured_source
        ));
    }
}
