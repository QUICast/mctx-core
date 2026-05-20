#[cfg(any(target_os = "linux", target_os = "macos"))]
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
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::collections::HashMap;
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

pub(crate) struct RawTransmitSocket {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    socket: Option<Socket>,
    #[cfg(any(target_os = "macos", windows))]
    raw_ipv4_protocol_sockets: Mutex<HashMap<i32, Arc<Socket>>>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unix_raw_ipv6_sockets: Mutex<HashMap<UnixRawIpv6SocketCacheKey, Arc<Socket>>>,
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
                .expect("raw IPv4 socket cache mutex poisoned")
                .len(),
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        debug.field(
            "cached_unix_raw_ipv6_socket_count",
            &self
                .unix_raw_ipv6_sockets
                .lock()
                .expect("Unix raw IPv6 socket cache mutex poisoned")
                .len(),
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

    let datagram_storage;
    let datagram = if let Some(ttl) = config.ttl {
        datagram_storage = apply_ttl_or_hop_limit_override(ip_datagram, parsed, ttl);
        datagram_storage.as_slice()
    } else {
        ip_datagram
    };

    match socket.linux_backend {
        LinuxRawTransmitBackend::RawIpv4 => {
            send_linux_raw_ipv4_datagram(socket, publication_id, config, parsed, datagram)
        }
        LinuxRawTransmitBackend::RawIpv6 => {
            send_linux_raw_ipv6_datagram(socket, publication_id, config, parsed, datagram)
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

    let datagram = prepare_macos_datagram_for_send(ip_datagram, parsed, config.ttl);
    match socket.macos_backend {
        MacosRawTransmitBackend::RawIpv4 => {
            let send_socket =
                cached_raw_ipv4_send_socket(socket, config, i32::from(parsed.protocol))?;
            let group = match parsed.destination_ip {
                IpAddr::V4(group) => group,
                IpAddr::V6(_) => unreachable!("validated above"),
            };
            let bytes_sent = send_socket
                .send_to(&datagram, &SockAddr::from(SocketAddrV4::new(group, 0)))
                .map_err(MctxError::RawSendFailed)?;

            Ok(raw_send_report(
                socket,
                publication_id,
                parsed,
                bytes_sent,
                config.outgoing_interface,
            ))
        }
        MacosRawTransmitBackend::RawIpv6 => {
            send_macos_raw_ipv6_datagram(socket, publication_id, config, parsed, &datagram)
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
    let send_socket = cached_raw_ipv4_send_socket(socket, config, i32::from(parsed.protocol))?;

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
        raw_ipv4_protocol_sockets: Mutex::new(HashMap::new()),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        unix_raw_ipv6_sockets: Mutex::new(HashMap::new()),
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        linux_backend: LinuxRawTransmitBackend::RawIpv4,
    })
}

#[cfg(target_os = "linux")]
fn open_linux_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config, None)?;
    Ok(RawTransmitSocket {
        family: PublicationAddressFamily::Ipv6,
        socket: None,
        #[cfg(any(target_os = "macos", windows))]
        raw_ipv4_protocol_sockets: Mutex::new(HashMap::new()),
        unix_raw_ipv6_sockets: Mutex::new(HashMap::new()),
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
        raw_ipv4_protocol_sockets: Mutex::new(HashMap::new()),
        unix_raw_ipv6_sockets: Mutex::new(HashMap::new()),
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        macos_backend: MacosRawTransmitBackend::RawIpv4,
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config, None)?;
    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_protocol_sockets: Mutex::new(HashMap::new()),
        unix_raw_ipv6_sockets: Mutex::new(HashMap::new()),
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
    let probe_socket =
        open_raw_ipv4_socket_with_protocol(selection, config.loopback, IPPROTO_RAW as i32)?;
    drop(probe_socket);

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_protocol_sockets: Mutex::new(HashMap::new()),
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
    bind_addr: Option<Ipv6Addr>,
    interface_index: u32,
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn resolve_raw_ipv6_selection(
    config: &RawPublicationConfig,
    datagram_source: Option<Ipv6Addr>,
) -> Result<RawIpv6Selection, MctxError> {
    let configured_bind_addr = config.bind_addr.and_then(|ip| match ip {
        IpAddr::V6(addr) => Some(addr),
        IpAddr::V4(_) => None,
    });
    if let (Some(configured_bind_addr), Some(datagram_source)) =
        (configured_bind_addr, datagram_source)
        && configured_bind_addr != datagram_source
    {
        return Err(MctxError::RawDatagramSourceMismatch {
            datagram_source: IpAddr::V6(datagram_source),
            configured_bind_addr: IpAddr::V6(configured_bind_addr),
        });
    }

    let bind_addr = configured_bind_addr
        .or(datagram_source)
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
    config: &RawPublicationConfig,
    protocol: i32,
) -> Result<Arc<Socket>, MctxError> {
    if let Some(cached) = socket
        .raw_ipv4_protocol_sockets
        .lock()
        .expect("raw IPv4 socket cache mutex poisoned")
        .get(&protocol)
        .cloned()
    {
        return Ok(cached);
    }

    let selection = resolve_raw_ipv4_selection(config)?;
    let new_socket = Arc::new(open_raw_ipv4_socket_with_protocol(
        selection,
        config.loopback,
        protocol,
    )?);

    let mut cache = socket
        .raw_ipv4_protocol_sockets
        .lock()
        .expect("raw IPv4 socket cache mutex poisoned");
    Ok(Arc::clone(
        cache
            .entry(protocol)
            .or_insert_with(|| Arc::clone(&new_socket)),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone)]
struct CachedUnixRawIpv6Socket {
    socket: Arc<Socket>,
    local_bind_addr: Option<IpAddr>,
    interface_index: u32,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cached_unix_raw_ipv6_send_socket(
    socket: &RawTransmitSocket,
    config: &RawPublicationConfig,
    source_addr: Ipv6Addr,
    protocol: i32,
    hop_limit: u8,
) -> Result<CachedUnixRawIpv6Socket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config, Some(source_addr))?;
    let cache_key = UnixRawIpv6SocketCacheKey {
        bind_addr: selection.bind_addr,
        interface_index: selection.interface_index,
        protocol,
        hop_limit,
    };

    if let Some(cached) = socket
        .unix_raw_ipv6_sockets
        .lock()
        .expect("Unix raw IPv6 socket cache mutex poisoned")
        .get(&cache_key)
        .cloned()
    {
        return Ok(CachedUnixRawIpv6Socket {
            socket: cached,
            local_bind_addr: cache_key.bind_addr.map(IpAddr::V6),
            interface_index: cache_key.interface_index,
        });
    }

    let new_socket = Arc::new(open_unix_raw_ipv6_socket_with_selection(
        selection,
        protocol,
        hop_limit,
        config.loopback,
    )?);

    let mut cache = socket
        .unix_raw_ipv6_sockets
        .lock()
        .expect("Unix raw IPv6 socket cache mutex poisoned");
    let socket = Arc::clone(
        cache
            .entry(cache_key)
            .or_insert_with(|| Arc::clone(&new_socket)),
    );

    Ok(CachedUnixRawIpv6Socket {
        socket,
        local_bind_addr: cache_key.bind_addr.map(IpAddr::V6),
        interface_index: cache_key.interface_index,
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

    let payload = datagram
        .get(parsed.header_len..)
        .ok_or(MctxError::InvalidRawIpDatagram)?;
    let effective_hop_limit = config.ttl.unwrap_or(parsed.ttl_or_hop_limit);
    let send_socket = cached_unix_raw_ipv6_send_socket(
        socket,
        config,
        source_addr,
        i32::from(parsed.protocol),
        effective_hop_limit,
    )?;
    let destination_scope_id = ipv6_destination_scope_id(group, send_socket.interface_index);
    let destination = SocketAddrV6::new(group, 0, 0, destination_scope_id);

    let bytes_sent = send_socket
        .socket
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
        send_socket.local_bind_addr,
        Some(send_socket.interface_index),
    ))
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

#[cfg(test)]
pub(crate) fn ipv4_multicast_mac(group: Ipv4Addr) -> [u8; 6] {
    let octets = group.octets();
    [0x01, 0x00, 0x5e, octets[1] & 0x7f, octets[2], octets[3]]
}

#[cfg(test)]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn raw_ipv6_selection_uses_datagram_source_when_bind_addr_is_not_set() {
        let loopback_index = resolve_ipv6_interface_index(Ipv6Addr::LOCALHOST).unwrap();
        let selection = resolve_raw_ipv6_selection(
            &RawPublicationConfig::ipv6().with_ipv6_interface_index(loopback_index),
            Some(Ipv6Addr::LOCALHOST),
        )
        .unwrap();

        assert_eq!(selection.bind_addr, Some(Ipv6Addr::LOCALHOST));
        assert_eq!(selection.interface_index, loopback_index);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn raw_ipv6_selection_rejects_mismatched_configured_bind_addr() {
        let err = resolve_raw_ipv6_selection(
            &RawPublicationConfig::ipv6().with_bind_addr(Ipv6Addr::LOCALHOST),
            Some("fd00::10".parse::<Ipv6Addr>().unwrap()),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            MctxError::RawDatagramSourceMismatch {
                datagram_source,
                configured_bind_addr,
            } if datagram_source == IpAddr::V6("fd00::10".parse::<Ipv6Addr>().unwrap())
                && configured_bind_addr == IpAddr::V6(Ipv6Addr::LOCALHOST)
        ));
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
