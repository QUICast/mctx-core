#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::config::OutgoingInterface;
use crate::config::PublicationAddressFamily;
use crate::error::MctxError;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::platform::resolve_ipv4_interface_index;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::platform::resolve_ipv6_interface_index;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::raw::RawValidationMode;
#[cfg(any(target_os = "linux", windows))]
use crate::raw::datagram::apply_ttl_or_hop_limit_override;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::raw::datagram::{ParsedRawIpDatagram, parse_raw_ip_datagram};
#[cfg(target_os = "linux")]
use crate::raw::linux_packet;
#[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
use crate::raw::linux_route::LinuxIpv6RouteEgress;
#[cfg(target_os = "macos")]
use crate::raw::macos_bpf::MacosBpfIpv6Socket;
use crate::raw::{RawPublicationConfig, RawPublicationId, RawSendReport};
#[cfg(any(target_os = "macos", windows))]
use crate::socket_cache::BoundedSocketCache;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
#[cfg(target_os = "macos")]
use std::io::IoSlice;
use std::net::IpAddr;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use std::net::Ipv4Addr;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::net::Ipv6Addr;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::net::SocketAddrV4;
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
    #[cfg(target_os = "linux")]
    linux_packet_ipv6_socket: Mutex<Option<Arc<Socket>>>,
    #[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
    linux_route_ipv6: Option<Mutex<LinuxIpv6RouteEgress>>,
    #[cfg(target_os = "macos")]
    macos_bpf_ipv6: Option<MacosBpfIpv6Socket>,
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
        #[cfg(target_os = "linux")]
        debug.field(
            "has_linux_packet_ipv6_socket",
            &lock_recover(&self.linux_packet_ipv6_socket).is_some(),
        );
        #[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
        debug.field("has_linux_route_ipv6", &self.linux_route_ipv6.is_some());
        #[cfg(target_os = "macos")]
        debug.field("has_macos_bpf_ipv6", &self.macos_bpf_ipv6.is_some());
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
    RawIpv6Explicit,
    #[cfg(feature = "raw-route-egress")]
    RawIpv6RouteSelected,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosRawTransmitBackend {
    RawIpv4,
    BpfIpv6,
}

#[cfg(target_os = "linux")]
pub(crate) fn open_raw_transmit_socket(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    config.validate()?;

    #[cfg(feature = "raw-route-egress")]
    if config.uses_route_selected_egress() {
        return match configured_socket_family(config) {
            Some(PublicationAddressFamily::Ipv4) => open_linux_route_selected_raw_socket_v4(config),
            Some(PublicationAddressFamily::Ipv6) => open_linux_route_selected_raw_socket_v6(config),
            None => unreachable!("route-selected config validation requires a family"),
        };
    }

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

    #[cfg(feature = "raw-route-egress")]
    if config.uses_route_selected_egress() {
        return match configured_socket_family(config) {
            Some(PublicationAddressFamily::Ipv4) => open_macos_route_selected_raw_socket_v4(config),
            Some(PublicationAddressFamily::Ipv6) => Err(MctxError::RawPacketTransmitUnsupported(
                "route-selected full-header IPv6 egress is not supported on macOS".to_string(),
            )),
            None => unreachable!("route-selected config validation requires a family"),
        };
    }

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

    #[cfg(feature = "raw-route-egress")]
    if config.uses_route_selected_egress() {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "route-selected raw egress is not supported on Windows".to_string(),
        ));
    }

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
        LinuxRawTransmitBackend::RawIpv6Explicit => {
            send_linux_raw_ipv6_datagram(socket, publication_id, config, parsed, ip_datagram)
        }
        #[cfg(feature = "raw-route-egress")]
        LinuxRawTransmitBackend::RawIpv6RouteSelected => send_linux_route_selected_ipv6_datagram(
            socket,
            publication_id,
            config,
            parsed,
            ip_datagram,
        ),
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
            let bytes_sent = preserve_raw_send_error(
                send_socket
                    .send_to_vectored(&buffers, &SockAddr::from(SocketAddrV4::new(group, 0))),
            )?;
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
        MacosRawTransmitBackend::BpfIpv6 => {
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

    let bytes_sent = preserve_raw_send_error(
        socket
            .socket
            .as_ref()
            .expect("linux raw IPv4 socket is opened during publication setup")
            .send_to(datagram, &SockAddr::from(destination)),
    )?;

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
    let group = match parsed.destination_ip {
        IpAddr::V6(group) if group.is_multicast() => group,
        IpAddr::V6(_)
            if config.validation_mode == RawValidationMode::StrictMulticastDestination =>
        {
            return Err(MctxError::InvalidRawMulticastDestination);
        }
        IpAddr::V6(_) => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "Linux full-header raw IPv6 transmit supports multicast destinations only"
                    .to_string(),
            ));
        }
        IpAddr::V4(_) => return Err(MctxError::InvalidRawIpDatagram),
    };
    ensure_ipv6_header_is_not_overridden(config, parsed)?;
    send_linux_packet_ipv6_datagram(socket, publication_id, config, parsed, datagram, group)
}

#[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
fn send_linux_route_selected_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let group = match parsed.destination_ip {
        IpAddr::V6(group) if group.is_multicast() => group,
        IpAddr::V6(_)
            if config.validation_mode == RawValidationMode::StrictMulticastDestination =>
        {
            return Err(MctxError::InvalidRawMulticastDestination);
        }
        IpAddr::V6(_) => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "Linux route-selected raw IPv6 transmit supports multicast destinations only"
                    .to_string(),
            ));
        }
        IpAddr::V4(_) => return Err(MctxError::InvalidRawIpDatagram),
    };

    let route_state = socket
        .linux_route_ipv6
        .as_ref()
        .expect("route-selected IPv6 initializes route state");
    let (interface_index, send_socket) = lock_recover(route_state).egress(group)?;
    let send_result = linux_packet::send_ipv6(&send_socket, interface_index, group, datagram);
    let bytes_sent = match send_result {
        Ok(bytes_sent) => bytes_sent,
        Err(error) => {
            lock_recover(route_state).invalidate(interface_index);
            return Err(error);
        }
    };

    Ok(raw_send_report_with_metadata(
        publication_id,
        parsed,
        bytes_sent,
        config.outgoing_interface,
        None,
        Some(interface_index),
    ))
}

#[cfg(target_os = "macos")]
fn send_macos_raw_ipv6_datagram(
    socket: &RawTransmitSocket,
    publication_id: RawPublicationId,
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
    datagram: &[u8],
) -> Result<RawSendReport, MctxError> {
    let group = match parsed.destination_ip {
        IpAddr::V6(group) if group.is_multicast() => group,
        IpAddr::V6(_)
            if config.validation_mode == RawValidationMode::StrictMulticastDestination =>
        {
            return Err(MctxError::InvalidRawMulticastDestination);
        }
        IpAddr::V6(_) => {
            return Err(MctxError::RawPacketTransmitUnsupported(
                "macOS BPF IPv6 transmit supports multicast destinations only".to_string(),
            ));
        }
        IpAddr::V4(_) => return Err(MctxError::InvalidRawIpDatagram),
    };
    ensure_ipv6_header_is_not_overridden(config, parsed)?;
    let bpf = socket
        .macos_bpf_ipv6
        .as_ref()
        .expect("macOS IPv6 publication initializes BPF");
    let bytes_sent = bpf.send_ipv6(group, datagram)?;

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

    let bytes_sent =
        preserve_raw_send_error(send_socket.send_to(datagram, &SockAddr::from(destination)))?;

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
fn preserve_raw_send_error<T>(result: std::io::Result<T>) -> Result<T, MctxError> {
    result.map_err(MctxError::RawSendFailed)
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

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn ensure_ipv6_header_is_not_overridden(
    config: &RawPublicationConfig,
    parsed: ParsedRawIpDatagram,
) -> Result<(), MctxError> {
    if config
        .ttl
        .is_some_and(|hop_limit| hop_limit != parsed.ttl_or_hop_limit)
    {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "full-header IPv6 egress preserves the supplied hop limit and cannot override it"
                .to_string(),
        ));
    }
    Ok(())
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
        linux_packet_ipv6_socket: Mutex::new(None),
        #[cfg(feature = "raw-route-egress")]
        linux_route_ipv6: None,
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        linux_backend: LinuxRawTransmitBackend::RawIpv4,
    })
}

#[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
fn open_linux_route_selected_raw_socket_v4(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
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

    if let Some(loopback) = config.loopback {
        socket
            .set_multicast_loop_v4(loopback)
            .map_err(MctxError::SocketOptionFailed)?;
    }

    Ok(RawTransmitSocket {
        socket: Some(socket),
        #[cfg(any(target_os = "macos", windows))]
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        linux_packet_ipv6_socket: Mutex::new(None),
        linux_route_ipv6: None,
        family: PublicationAddressFamily::Ipv4,
        interface_index: None,
        local_bind_addr: None,
        linux_backend: LinuxRawTransmitBackend::RawIpv4,
    })
}

#[cfg(all(target_os = "linux", feature = "raw-route-egress"))]
fn open_linux_route_selected_raw_socket_v6(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    if config.loopback == Some(true) {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Linux route-selected IPv6 uses AF_PACKET and cannot enable local IP multicast loopback"
                .to_string(),
        ));
    }

    Ok(RawTransmitSocket {
        socket: None,
        linux_packet_ipv6_socket: Mutex::new(None),
        linux_route_ipv6: Some(Mutex::new(LinuxIpv6RouteEgress::new()?)),
        family: PublicationAddressFamily::Ipv6,
        interface_index: None,
        local_bind_addr: None,
        linux_backend: LinuxRawTransmitBackend::RawIpv6RouteSelected,
    })
}

#[cfg(target_os = "linux")]
fn open_linux_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config)?;
    if config.loopback == Some(true) {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "Linux full-header IPv6 uses AF_PACKET and cannot enable local IP multicast loopback"
                .to_string(),
        ));
    }
    Ok(RawTransmitSocket {
        family: PublicationAddressFamily::Ipv6,
        socket: None,
        #[cfg(any(target_os = "macos", windows))]
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        linux_packet_ipv6_socket: Mutex::new(None),
        #[cfg(feature = "raw-route-egress")]
        linux_route_ipv6: None,
        interface_index: Some(selection.interface_index),
        local_bind_addr: selection.bind_addr.map(IpAddr::V6),
        linux_backend: LinuxRawTransmitBackend::RawIpv6Explicit,
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v4(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let probe_socket =
        open_raw_ipv4_socket_with_protocol(Some(selection), config.loopback, libc::IPPROTO_RAW)?;
    drop(probe_socket);

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: Some(selection),
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        macos_bpf_ipv6: None,
        family: PublicationAddressFamily::Ipv4,
        interface_index: Some(selection.interface_index),
        local_bind_addr: Some(IpAddr::V4(selection.bind_addr)),
        macos_backend: MacosRawTransmitBackend::RawIpv4,
    })
}

#[cfg(all(target_os = "macos", feature = "raw-route-egress"))]
fn open_macos_route_selected_raw_socket_v4(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    let probe_socket =
        open_raw_ipv4_socket_with_protocol(None, config.loopback, libc::IPPROTO_RAW)?;
    drop(probe_socket);

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: None,
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        macos_bpf_ipv6: None,
        family: PublicationAddressFamily::Ipv4,
        interface_index: None,
        local_bind_addr: None,
        macos_backend: MacosRawTransmitBackend::RawIpv4,
    })
}

#[cfg(target_os = "macos")]
fn open_macos_raw_socket_v6(config: &RawPublicationConfig) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv6_selection(config)?;
    if config.loopback == Some(true) {
        return Err(MctxError::RawPacketTransmitUnsupported(
            "macOS full-header IPv6 uses BPF and cannot enable local IP multicast loopback"
                .to_string(),
        ));
    }
    let bpf = MacosBpfIpv6Socket::open(selection.interface_index)?;

    Ok(RawTransmitSocket {
        socket: None,
        raw_ipv4_selection: None,
        raw_ipv4_protocol_sockets: Mutex::new(BoundedSocketCache::default()),
        macos_bpf_ipv6: Some(bpf),
        family: PublicationAddressFamily::Ipv6,
        interface_index: Some(selection.interface_index),
        local_bind_addr: selection.bind_addr.map(IpAddr::V6),
        macos_backend: MacosRawTransmitBackend::BpfIpv6,
    })
}

#[cfg(windows)]
fn open_windows_raw_socket_v4(
    config: &RawPublicationConfig,
) -> Result<RawTransmitSocket, MctxError> {
    let selection = resolve_raw_ipv4_selection(config)?;
    let probe_socket =
        open_raw_ipv4_socket_with_protocol(Some(selection), config.loopback, IPPROTO_RAW)?;
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
    selection: Option<RawIpv4Selection>,
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
    if let Some(selection) = selection {
        socket
            .bind(&SockAddr::from(SocketAddrV4::new(selection.bind_addr, 0)))
            .map_err(MctxError::RawSocketBindFailed)?;
        socket
            .set_multicast_if_v4(&selection.interface_addr)
            .map_err(MctxError::SocketOptionFailed)?;
    }

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

#[cfg(any(target_os = "macos", windows))]
fn cached_raw_ipv4_send_socket(
    socket: &RawTransmitSocket,
    loopback: Option<bool>,
    protocol: i32,
) -> Result<Arc<Socket>, MctxError> {
    let selection = socket.raw_ipv4_selection;
    let mut cache = lock_recover(&socket.raw_ipv4_protocol_sockets);
    cache.get_or_try_insert_with(protocol, || {
        open_raw_ipv4_socket_with_protocol(selection, loopback, protocol)
    })
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
    let interface_index = socket
        .interface_index
        .ok_or(MctxError::RawInterfaceRequired)?;
    let send_socket = cached_linux_packet_ipv6_socket(socket)?;
    let bytes_sent = linux_packet::send_ipv6(&send_socket, interface_index, group, ip_datagram)?;

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
    fn transient_send_errors_preserve_their_kind_without_latching_state() {
        let error = preserve_raw_send_error::<usize>(Err(std::io::Error::new(
            std::io::ErrorKind::NetworkUnreachable,
            "temporary route failure",
        )))
        .unwrap_err();

        let MctxError::RawSendFailed(error) = error else {
            panic!("unexpected raw send error variant");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::NetworkUnreachable);
        assert_eq!(error.to_string(), "temporary route failure");
        assert_eq!(preserve_raw_send_error(Ok(42usize)).unwrap(), 42);
    }

    #[test]
    fn full_header_ipv6_rejects_a_conflicting_hop_limit_override() {
        let datagram = build_ipv6_datagram(
            "2001:db8::10".parse().unwrap(),
            "ff3e::8000:1234".parse().unwrap(),
            37,
        );
        let parsed = parse_raw_ip_datagram(&datagram).unwrap();

        assert!(
            ensure_ipv6_header_is_not_overridden(
                &RawPublicationConfig::ipv6().with_ttl(37),
                parsed
            )
            .is_ok()
        );
        assert!(matches!(
            ensure_ipv6_header_is_not_overridden(
                &RawPublicationConfig::ipv6().with_ttl(38),
                parsed
            ),
            Err(MctxError::RawPacketTransmitUnsupported(_))
        ));
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

    fn build_ipv6_datagram(source: Ipv6Addr, destination: Ipv6Addr, hop_limit: u8) -> Vec<u8> {
        let mut datagram = vec![0x60, 0, 0, 0, 0, 0, 59, hop_limit];
        datagram.extend_from_slice(&source.octets());
        datagram.extend_from_slice(&destination.octets());
        datagram
    }
}
