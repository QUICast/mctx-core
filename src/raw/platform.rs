#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::config::ipv6_destination_scope_id;
use crate::config::{OutgoingInterface, PublicationAddressFamily};
use crate::error::MctxError;
use crate::platform::resolve_ipv4_interface_index;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::platform::resolve_ipv6_interface_index;
use crate::raw::RawValidationMode;
#[cfg(any(target_os = "linux", windows))]
use crate::raw::datagram::apply_ttl_or_hop_limit_override;
use crate::raw::datagram::{ParsedRawIpDatagram, parse_raw_ip_datagram};
#[cfg(target_os = "linux")]
use crate::raw::linux_packet;
use crate::raw::socket_cache::BoundedSocketCache;
use crate::raw::{RawPublicationConfig, RawPublicationId, RawSendReport};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
#[cfg(target_os = "macos")]
use std::io::IoSlice;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::net::Ipv6Addr;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::net::SocketAddrV4;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::net::SocketAddrV6;
use std::net::{IpAddr, Ipv4Addr};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::IPPROTO_RAW;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct RawTransmitSocket {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    socket: Option<Socket>,
    #[cfg(any(target_os = "macos", windows))]
    raw_ipv4_selection: Option<RawIpv4Selection>,
    #[cfg(any(target_os = "macos", windows))]
    raw_ipv4_protocol_sockets: Mutex<BoundedSocketCache<i32>>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unix_raw_ipv6_sockets: Mutex<BoundedSocketCache<UnixRawIpv6SocketCacheKey>>,
    #[cfg(target_os = "linux")]
    linux_packet_ipv6_socket: Mutex<Option<Arc<Socket>>>,
    family: PublicationAddressFamily,
    interface_index: Option<u32>,
    local_bind_addr: Option<IpAddr>,
    #[cfg(target_os = "linux")]
    linux_backend: LinuxRawTransmitBackend,
    #[cfg(target_os = "macos")]
    macos_backend: MacosRawTransmitBackend,
}

impl std::fmt::Debug for RawTransmitSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("RawTransmitSocket");
        debug
            .field("family", &self.family)
            .field("interface_index", &self.interface_index)
            .field("local_bind_addr", &self.local_bind_addr);
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        debug.field("has_socket", &self.socket.is_some());
        #[cfg(any(target_os = "macos", windows))]
        debug.field(
            "cached_raw_ipv4_protocol_socket_count",
            &self
                .raw_ipv4_protocol_sockets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        debug.field(
            "cached_unix_raw_ipv6_socket_count",
            &self
                .unix_raw_ipv6_sockets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
        );
        #[cfg(target_os = "linux")]
        debug.field(
            "has_linux_packet_ipv6_socket",
            &lock_recover(&self.linux_packet_ipv6_socket).is_some(),
        );
        #[cfg(target_os = "linux")]
        debug.field("linux_backend", &self.linux_backend);
        #[cfg(target_os = "macos")]
        debug.field("macos_backend", &self.macos_backend);
        debug.finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxRawTransmitBackend {
    RawIpv4,
    RawIpv6,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosRawTransmitBackend {
    RawIpv4,
    RawIpv6,
}

#[cfg(target_os = "linux")]
pub(crate) fn open_raw_transmit_socket(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    config.validate()?;

    match configured_socket_family(config) {
        Some(PublicationAddressFamily::Ipv4) => open_linux_raw_socket_v4(config),
        Some(PublicationAddressFamily::Ipv6) | None => open_linux_raw_socket_v6(config),
    }
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

    match socket.linux_backend {
        LinuxRawTransmitBackend::RawIpv4 => {
            let datagram_storage;
            let datagram =
                if let Some(ttl) = config.ttl.filter(|ttl| *ttl != parsed.ttl_or_hop_limit) {
                    datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl);
                    datagram_storage.as_slice()
                } else {
                    ip_datagram
                };
            send_linux_raw_ipv4_datagram(socket, publication_id, config, parsed, datagram)
        }
        LinuxRawTransmitBackend::RawIpv6 => {
            send_linux_raw_ipv6_datagram(socket, publication_id, config, parsed, ip_datagram)
        }
    }
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
    match socket.macos_backend {
        MacosRawTransmitBackend::RawIpv4 => {
            let (header, header_len) =
                prepare_macos_ipv4_header_for_send(ip_datagram, parsed, config.ttl);
            let send_socket =
                cached_raw_ipv4_send_socket(socket, config.loopback, i32::from(parsed.protocol))?;
            let group = match parsed.destination_ip {
                IpAddr::V4(group) => group,
                IpAddr::V6(_) => unreachable!("validated above"),
            };
            let buffers = [
                IoSlice::new(&header[..header_len]),
                IoSlice::new(&ip_datagram[header_len..]),
            ];
            let bytes_sent = send_socket
                .send_to_vectored(&buffers, &SockAddr::from(SocketAddrV4::new(group, 0)))
                .map_err(MctxError::RawSendFailed)?;
            if bytes_sent != ip_datagram.len() {
                return Err(MctxError::RawSendFailed(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    format!(
                        "partial raw IPv4 send: wrote {bytes_sent} of {} bytes",
                        ip_datagram.len()
                    ),
                )));
            }

            Ok(raw_send_report(
                socket,
                publication_id,
                parsed,
                bytes_sent,
                config.outgoing_interface,
            ))
        }
        MacosRawTransmitBackend::RawIpv6 => {
            send_macos_raw_ipv6_datagram(socket, publication_id, config, parsed, ip_datagram)
        }
    }
}

#[cfg(target_os = "linux")]
fn send_linux_raw_ipv4_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let destination = match parsed.destination_ip {
        IpAddr::V4(group) if group.is_multicast() => SocketAddrV4::new(group, 0),
        IpAddr::V4(_) => match config.validation_mode {
            RawValidationMode::StrictMulticastDestination => {
                return Err(MctxError::InvalidRawMulticastDestination);
            }
            RawValidationMode::AllowAnyDestination => {
                return Err(MctxError::RawPacketTransmitUnsupported(
                    "Linux raw IPv4 transmit currently supports multicast destinations only"
                        .to_string(),
                ));
            }
        },
        IpAddr::V6(_) => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "Linux raw IPv4 transmit cannot send IPv6 datagrams".to_string(),
            ));
        }
    };

    let bytes_sent = socket
        .socket
        .as_ref()
        .expect("linux raw IPv4 socket is opened during publication setup")
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

#[cfg(target_os = "linux")]
fn send_linux_raw_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    send_unix_raw_ipv6_datagram(socket, publication_id, config, parsed, datagram)
}

#[cfg(target_os = "macos")]
fn send_macos_raw_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    send_unix_raw_ipv6_datagram(socket, publication_id, config, parsed, datagram)
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
    let datagram = if let Some(ttl) = config.ttl.filter(|ttl| *ttl != parsed.ttl_or_hop_limit) {
        datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl);
        datagram_storage.as_slice()
    } else {
        ip_datagram
    };

    let destination = match parsed.destination_ip {
        IpAddr::V4(group) => SocketAddrV4::new(group, 0),
        IpAddr::V6(_) => unreachable!("validated above"),
    };
    let send_socket =
        cached_raw_ipv4_send_socket(socket, config.loopback, i32::from(parsed.protocol))?;

    let bytes_sent = send_socket
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
    raw_send_report_with_metadata(
        publication_id,
        parsed,
        bytes_sent,
        outgoing_interface,
        socket.local_bind_addr,
        socket.interface_index,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn raw_send_report_with_metadata(
    publication_id: RawPublicationId,
    parsed: ParsedRawIpDatagram,
    bytes_sent: usize,
    outgoing_interface: Option<OutgoingInterface>,
    local_bind_addr: Option<IpAddr>,
    outgoing_interface_index: Option<u32>,
) -> RawSendReport {
    RawSendReport {
        publication_id,
        family: parsed.family,
        source_ip: Some(parsed.source_ip),
        destination_ip: Some(parsed.destination_ip),
        ip_protocol: Some(parsed.protocol),
        bytes_sent,
        local_bind_addr,
        outgoing_interface,
        outgoing_interface_index,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn configured_socket_family(config: &RawPublicationConfig) -> Option<PublicationAddressFamily> {
    config
        .family
        .or_else(|| config.bind_addr.map(ip_family))
        .or_else(|| outgoing_interface_family(config.outgoing_interface))
}

#[cfg(any(target_os = "macos", windows))]
fn infer_socket_family(
    config: &RawPublicationConfig,
) -> Result<PublicationAddressFamily, MctxError> {
    configured_socket_family(config).ok_or(MctxError::RawInterfaceRequired)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
fn ip_family(ip: IpAddr) -> PublicationAddressFamily {
    match ip {
        IpAddr::V4(_) => PublicationAddressFamily::Ipv4,
        IpAddr::V6(_) => PublicationAddressFamily::Ipv6,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
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
fn open_linux_raw_socket_v4(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
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
        socket: Some(socket),
        #[cfg(any(target_os = "macos", windows))]
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        unix_raw_ipv6_sockets: Mutex::new(BoundedSocketCache::default()),
        linux_packet_ipv6_socket: Mutex::new(None),
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        linux_backend: LinuxRawTransmitBackend::RawIpv4,
    })
}

#[cfg(target_os = "linux")]
fn open_linux_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config)?;
    Ok(RawTransmitSocket {
        family: PublicationAddressFamily::Ipv6,
        socket: None,
        #[cfg(any(target_os = "macos", windows))]
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        unix_raw_ipv6_sockets: Mutex::new(BoundedSocketCache::default()),
        linux_packet_ipv6_socket: Mutex::new(None),
        interface_index: Some(selection.interface_index),
        local_bind_addr: selection.bind_addr.map(IpAddr::V6),
        linux_backend: LinuxRawTransmitBackend::RawIpv6,
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v4(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let probe_socket =
        open_raw_ipv4_socket_with_protocol(selection, config.loopback, libc::IPPROTO_RAW)?;
    drop(probe_socket);

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: Some(selection),
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        unix_raw_ipv6_sockets: Mutex::new(BoundedSocketCache::default()),
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        macos_backend: MacosRawTransmitBackend::RawIpv4,
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config)?;
    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: None,
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        unix_raw_ipv6_sockets: Mutex::new(BoundedSocketCache::default()),
        family: PublicationAddressFamily::Ipv6,
        interface_index: Some(selection.interface_index),
        local_bind_addr: selection.bind_addr.map(IpAddr::V6),
        macos_backend: MacosRawTransmitBackend::RawIpv6,
    })
}

#[cfg(windows)]
fn open_windows_raw_socket_v4(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let probe_socket = open_raw_ipv4_socket_with_protocol(selection, config.loopback, IPPROTO_RAW)?;
    drop(probe_socket);

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: Some(selection),
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
    })
}

#[cfg(any(target_os = "macos", windows))]
fn open_raw_ipv4_socket_with_protocol(
    selection: RawIpv4Selection,
    loopback: Option<bool>,
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
    socket
        .bind(&SockAddr::from(SocketAddrV4::new(selection.bind_addr, 0)))
        .map_err(MctxError::RawSocketBindFailed)?;
    socket
        .set_multicast_if_v4(&selection.interface_addr)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(loopback) = loopback {
        socket
            .set_multicast_loop_v4(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(socket)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[derive(Debug, Clone, Copy)]
struct RawIpv4Selection {
    bind_addr: Ipv4Addr,
    interface_addr: Ipv4Addr,
    interface_index: u32,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UnixRawIpv6SocketCacheKey {
    protocol: i32,
    hop_limit: u8,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Copy)]
struct RawIpv6Selection {
    bind_addr: Option<Ipv6Addr>,
    interface_index: u32,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
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
    let interface_index = resolve_ipv4_interface_index(interface_addr)?;
    let bind_index = if bind_addr == interface_addr {
        interface_index
    } else {
        resolve_ipv4_interface_index(bind_addr)?
    };

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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn resolve_raw_ipv6_selection(
    config: &RawPublicationConfig,
) -> Result<RawIpv6Selection, MctxError> {
    let configured_bind_addr = config.bind_addr.and_then(|ip| match ip {
        IpAddr::V6(addr) => Some(addr),
        IpAddr::V4(_) => None,
    });
    let bind_addr = configured_bind_addr.or(match config.outgoing_interface {
        Some(OutgoingInterface::Ipv6Addr(addr)) => Some(addr),
        _ => None,
    });

    let explicit_interface_index = match config.outgoing_interface {
        Some(OutgoingInterface::Ipv6Index(index)) => Some(index),
        Some(OutgoingInterface::Ipv6Addr(addr)) => Some(resolve_ipv6_interface_index(addr)?),
        _ => None,
    };

    let bind_interface_index = match (bind_addr, config.outgoing_interface) {
        (Some(bind_addr), Some(OutgoingInterface::Ipv6Addr(interface_addr)))
            if bind_addr == interface_addr =>
        {
            explicit_interface_index
        }
        (Some(bind_addr), _) => Some(resolve_ipv6_interface_index(bind_addr)?),
        (None, _) => None,
    };

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
    Ok(RawIpv6Selection {
        bind_addr,
        interface_index,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_unix_raw_ipv6_socket_with_selection(
    selection: RawIpv6Selection,
    protocol: i32,
    hop_limit: u8,
    loopback: Option<bool>,
) -> Result<Socket, MctxError> {
    let socket = Socket::new(Domain::IPV6, Type::RAW, Some(Protocol::from(protocol)))
        .map_err(MctxError::RawSocketCreateFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(bind_addr) = selection.bind_addr {
        let scope_id = if bind_addr.is_unicast_link_local() {
            selection.interface_index
        } else {
            0
        };

        socket
            .bind(&SockAddr::from(SocketAddrV6::new(
                bind_addr, 0, 0, scope_id,
            )))
            .map_err(MctxError::RawSocketBindFailed)?;
    }

    socket
        .set_multicast_if_v6(selection.interface_index)
        .map_err(MctxError::SocketOptionFailed)?;
    socket
        .set_multicast_hops_v6(u32::from(hop_limit))
        .map_err(MctxError::SocketOptionFailed)?;

    if let Some(loopback) = loopback {
        socket
            .set_multicast_loop_v6(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(socket)
}

#[cfg(any(target_os = "macos", windows))]
fn cached_raw_ipv4_send_socket(
    socket: &RawTransmitSocket,
    loopback: Option<bool>,
    protocol: i32,
) -> Result<Arc<Socket>, MctxError> {
    let selection = socket
        .raw_ipv4_selection
        .ok_or(MctxError::RawInterfaceRequired)?;
    let mut cache = lock_recover(&socket.raw_ipv4_protocol_sockets);
    cache.get_or_try_insert_with(protocol, || {
        open_raw_ipv4_socket_with_protocol(selection, loopback, protocol)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cached_unix_raw_ipv6_send_socket(
    socket: &RawTransmitSocket,
    loopback: Option<bool>,
    protocol: i32,
    hop_limit: u8,
) -> Result<Arc<Socket>, MctxError> {
    let cache_key = UnixRawIpv6SocketCacheKey {
        protocol,
        hop_limit,
    };
    let bind_addr = socket.local_bind_addr.and_then(|addr| match addr {
        IpAddr::V6(addr) => Some(addr),
        IpAddr::V4(_) => None,
    });
    let interface_index = socket
        .interface_index
        .ok_or(MctxError::RawInterfaceRequired)?;
    let selection = RawIpv6Selection {
        bind_addr,
        interface_index,
    };

    let mut cache = lock_recover(&socket.unix_raw_ipv6_sockets);
    cache.get_or_try_insert_with(cache_key, || {
        open_unix_raw_ipv6_socket_with_selection(selection, protocol, hop_limit, loopback)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn send_unix_raw_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let (source_addr, group) = match (parsed.source_ip, parsed.destination_ip) {
        (IpAddr::V6(source_addr), IpAddr::V6(group)) if group.is_multicast() => {
            (source_addr, group)
        }
        (IpAddr::V6(_), IpAddr::V6(_)) => match config.validation_mode {
            RawValidationMode::StrictMulticastDestination => {
                return Err(MctxError::InvalidRawMulticastDestination);
            }
            RawValidationMode::AllowAnyDestination => {
                return Err(MctxError::RawPacketTransmitUnsupported(
                    "Unix raw IPv6 transmit currently supports multicast destinations only"
                        .to_string(),
                ));
            }
        },
        (IpAddr::V6(_), IpAddr::V4(_)) | (IpAddr::V4(_), _) => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "Unix raw IPv6 transmit cannot send IPv4 datagrams".to_string(),
            ));
        }
    };

    let source_uses_local_stack = source_uses_local_ipv6_stack(socket.local_bind_addr, source_addr);

    #[cfg(target_os = "linux")]
    if !source_uses_local_stack {
        return send_linux_packet_ipv6_datagram(
            socket,
            publication_id,
            config,
            parsed,
            datagram,
            group,
        );
    }

    #[cfg(target_os = "macos")]
    if !source_uses_local_stack {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "macOS raw IPv6 transmit can preserve the source only when the datagram source matches the configured local bind address; remote-source IPv6 injection requires a link-layer backend"
                .to_string(),
        ));
    }

    let payload = datagram
        .get(parsed.header_len..)
        .ok_or(MctxError::InvalidRawIpDatagram)?;
    let effective_hop_limit = config.ttl.unwrap_or(parsed.ttl_or_hop_limit);
    let send_socket = cached_unix_raw_ipv6_send_socket(
        socket,
        config.loopback,
        i32::from(parsed.protocol),
        effective_hop_limit,
    )?;
    let interface_index = socket
        .interface_index
        .ok_or(MctxError::RawInterfaceRequired)?;
    let destination_scope_id = ipv6_destination_scope_id(group, interface_index);
    let destination = SocketAddrV6::new(group, 0, 0, destination_scope_id);

    let bytes_sent = send_socket
        .send_to(payload, &SockAddr::from(destination))
        .map_err(MctxError::RawSendFailed)?;

    if bytes_sent != payload.len() {
        return Err(MctxError::RawSendFailed(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            format!(
                "partial raw IPv6 send: wrote {bytes_sent} of {} payload bytes",
                payload.len()
            ),
        )));
    }

    Ok(raw_send_report_with_metadata(
        publication_id,
        parsed,
        datagram.len(),
        config.outgoing_interface,
        socket.local_bind_addr,
        Some(interface_index),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn source_uses_local_ipv6_stack(local_bind_addr: Option<IpAddr>, source_addr: Ipv6Addr) -> bool {
    local_bind_addr == Some(IpAddr::V6(source_addr))
}

#[cfg(target_os = "linux")]
fn send_linux_packet_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    ip_datagram: &[u8],
    group: Ipv6Addr,
) -> Result<RawSendReport, MctxError> {
    if config.loopback == Some(true) {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Linux link-layer IPv6 injection cannot enable local IP multicast loopback".to_string(),
        ));
    }

    let datagram_storage;
    let datagram = if let Some(hop_limit) = config
        .ttl
        .filter(|hop_limit| *hop_limit != parsed.ttl_or_hop_limit)
    {
        datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, hop_limit);
        datagram_storage.as_slice()
    } else {
        ip_datagram
    };

    let interface_index = socket
        .interface_index
        .ok_or(MctxError::RawInterfaceRequired)?;
    let send_socket = cached_linux_packet_ipv6_socket(socket)?;
    let bytes_sent = linux_packet::send_ipv6(&send_socket, interface_index, group, datagram)?;

    Ok(raw_send_report(
        socket,
        publication_id,
        parsed,
        bytes_sent,
        config.outgoing_interface,
    ))
}

#[cfg(target_os = "linux")]
fn cached_linux_packet_ipv6_socket(socket: &RawTransmitSocket) -> Result<Arc<Socket>, MctxError> {
    let mut cached = lock_recover(&socket.linux_packet_ipv6_socket);
    if let Some(socket) = cached.as_ref() {
        return Ok(Arc::clone(socket));
    }

    let interface_index = socket
        .interface_index
        .ok_or(MctxError::RawInterfaceRequired)?;
    let packet_socket = Arc::new(linux_packet::open_ipv6(interface_index)?);
    *cached = Some(Arc::clone(&packet_socket));
    Ok(packet_socket)
}

#[cfg(target_os = "macos")]
fn prepare_macos_ipv4_header_for_send(
    ip_datagram: &[u8],
    parsed: ParsedRawIpDatagram,
    ttl_override: Option<u8>,
) -> ([u8; 60], usize) {
    let mut header = [0u8; 60];
    header[..parsed.header_len].copy_from_slice(&ip_datagram[..parsed.header_len]);
    if let Some(ttl) = ttl_override {
        header[8] = ttl;
    }
    normalize_macos_ipv4_header_for_hdrincl(&mut header[..parsed.header_len]);

    (header, parsed.header_len)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{RawPublicationConfig, RawValidationMode};

    #[test]
    fn raw_ipv6_backend_selection_distinguishes_local_and_remote_sources() {
        let local = Ipv6Addr::LOCALHOST;

        assert!(source_uses_local_ipv6_stack(Some(IpAddr::V6(local)), local));
        assert!(!source_uses_local_ipv6_stack(
            Some(IpAddr::V6(local)),
            "2001:db8::10".parse().unwrap()
        ));
        assert!(!source_uses_local_ipv6_stack(None, local));
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn raw_ipv6_selection_keeps_interface_only_selection_unbound() {
        let loopback_index = resolve_ipv6_interface_index(Ipv6Addr::LOCALHOST).unwrap();
        let selection = resolve_raw_ipv6_selection(
            &RawPublicationConfig::ipv6().with_ipv6_interface_index(loopback_index),
        )
        .unwrap();

        assert_eq!(selection.bind_addr, None);
        assert_eq!(selection.interface_index, loopback_index);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn raw_ipv6_selection_resolves_configured_bind_once() {
        let loopback_index = resolve_ipv6_interface_index(Ipv6Addr::LOCALHOST).unwrap();
        let selection = resolve_raw_ipv6_selection(
            &RawPublicationConfig::ipv6().with_bind_addr(Ipv6Addr::LOCALHOST),
        )
        .unwrap();

        assert_eq!(selection.bind_addr, Some(Ipv6Addr::LOCALHOST));
        assert_eq!(selection.interface_index, loopback_index);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ipv4_hdrincl_header_is_normalized_to_host_order() {
        let datagram = build_ipv4_datagram(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(239, 1, 2, 3));
        let parsed = parse_raw_ip_datagram(&datagram).unwrap();
        let (normalized, header_len) = prepare_macos_ipv4_header_for_send(&datagram, parsed, None);

        assert_eq!(
            &normalized[2..4],
            &u16::from_be_bytes([datagram[2], datagram[3]]).to_ne_bytes()
        );
        assert_eq!(
            &normalized[6..8],
            &u16::from_be_bytes([datagram[6], datagram[7]]).to_ne_bytes()
        );
        assert_eq!(&normalized[10..12], &[0, 0]);
        assert_eq!(header_len, 20);
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
