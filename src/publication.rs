#[cfg(feature = "metrics")]
use crate::metrics::PublicationMetricsSnapshot;
use crate::platform::resolve_ipv6_interface_index;
use crate::{
    MctxError, SendReport,
    config::{Ipv6MulticastScope, OutgoingInterface, PublicationConfig, ipv6_destination_scope_id},
};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
#[cfg(feature = "metrics")]
use std::time::SystemTime;

/// Stable ID for one configured publication socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicationId(pub u64);

/// Extracted publication parts.
#[derive(Debug)]
pub struct PublicationParts {
    pub id: PublicationId,
    pub config: PublicationConfig,
    pub socket: Socket,
}

/// One ready-to-send multicast publication.
#[derive(Debug)]
pub struct Publication {
    id: PublicationId,
    config: PublicationConfig,
    socket: Socket,
    destination: SocketAddr,
    #[cfg(feature = "metrics")]
    metrics: PublicationMetricsState,
}

impl Publication {
    /// Creates and configures a new multicast publication socket.
    pub fn new(id: PublicationId, config: PublicationConfig) -> Result<Self, MctxError> {
        config.validate()?;

        let domain = match config.family() {
            crate::PublicationAddressFamily::Ipv4 => Domain::IPV4,
            crate::PublicationAddressFamily::Ipv6 => Domain::IPV6,
        };
        let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
            .map_err(MctxError::SocketCreateFailed)?;

        let destination = Self::configure_socket(&socket, &config, false)?;

        Ok(Self {
            id,
            config,
            socket,
            destination,
            #[cfg(feature = "metrics")]
            metrics: PublicationMetricsState::default(),
        })
    }

    /// Wraps and configures an existing multicast socket.
    pub fn new_with_socket(
        id: PublicationId,
        config: PublicationConfig,
        socket: Socket,
    ) -> Result<Self, MctxError> {
        config.validate()?;
        let destination = Self::configure_socket(&socket, &config, true)?;

        Ok(Self {
            id,
            config,
            socket,
            destination,
            #[cfg(feature = "metrics")]
            metrics: PublicationMetricsState::default(),
        })
    }

    /// Wraps and configures an existing standard-library UDP socket.
    pub fn new_with_udp_socket(
        id: PublicationId,
        config: PublicationConfig,
        socket: UdpSocket,
    ) -> Result<Self, MctxError> {
        Self::new_with_socket(id, config, Socket::from(socket))
    }

    /// Returns the publication ID.
    pub fn id(&self) -> PublicationId {
        self.id
    }

    /// Returns the configured publication parameters.
    pub fn config(&self) -> &PublicationConfig {
        &self.config
    }

    /// Returns the resolved destination address.
    pub fn destination(&self) -> SocketAddr {
        self.destination
    }

    /// Returns the resolved destination IPv4 address.
    pub fn destination_v4(&self) -> Result<SocketAddrV4, MctxError> {
        match self.destination {
            SocketAddr::V4(destination) => Ok(destination),
            SocketAddr::V6(_) => Err(MctxError::ExistingSocketAddressFamilyMismatch),
        }
    }

    /// Returns the resolved destination IPv6 address.
    pub fn destination_v6(&self) -> Result<SocketAddrV6, MctxError> {
        match self.destination {
            SocketAddr::V4(_) => Err(MctxError::ExistingSocketAddressFamilyMismatch),
            SocketAddr::V6(destination) => Ok(destination),
        }
    }

    /// Returns a shared reference to the live socket.
    pub fn socket(&self) -> &Socket {
        &self.socket
    }

    /// Returns a mutable reference to the live socket.
    pub fn socket_mut(&mut self) -> &mut Socket {
        &mut self.socket
    }

    /// Sends one payload.
    pub fn send(&self, payload: &[u8]) -> Result<SendReport, MctxError> {
        match self.socket.send(payload) {
            Ok(bytes_sent) => {
                #[cfg(feature = "metrics")]
                self.metrics.record_success(bytes_sent);

                let local_addr = self.local_addr().ok();

                Ok(SendReport {
                    publication_id: self.id,
                    destination: self.destination,
                    local_addr,
                    source_addr: local_addr.map(|addr| addr.ip()),
                    bytes_sent,
                })
            }
            Err(error) => {
                #[cfg(feature = "metrics")]
                self.metrics.record_error();

                Err(MctxError::SendFailed(error))
            }
        }
    }

    /// Returns the publication socket local address.
    pub fn local_addr(&self) -> Result<SocketAddr, MctxError> {
        self.socket
            .local_addr()
            .map_err(MctxError::SocketLocalAddrFailed)?
            .as_socket()
            .ok_or_else(|| {
                MctxError::SocketLocalAddrFailed(std::io::Error::other(
                    "socket local address was not an IP address",
                ))
            })
    }

    /// Returns the publication socket local IPv4 address.
    pub fn local_addr_v4(&self) -> Result<SocketAddrV4, MctxError> {
        match self.local_addr()? {
            SocketAddr::V4(local_addr) => Ok(local_addr),
            SocketAddr::V6(_) => Err(MctxError::ExistingSocketAddressFamilyMismatch),
        }
    }

    /// Returns the publication socket local IPv6 address.
    pub fn local_addr_v6(&self) -> Result<SocketAddrV6, MctxError> {
        match self.local_addr()? {
            SocketAddr::V4(_) => Err(MctxError::ExistingSocketAddressFamilyMismatch),
            SocketAddr::V6(local_addr) => Ok(local_addr),
        }
    }

    /// Returns the effective local source address.
    pub fn source_addr(&self) -> Result<IpAddr, MctxError> {
        Ok(self.local_addr()?.ip())
    }

    /// Returns the `(source, group, udp_port)` tuple needed for announce frames.
    pub fn announce_tuple(&self) -> Result<(IpAddr, IpAddr, u16), MctxError> {
        Ok((self.source_addr()?, self.config.group, self.config.dst_port))
    }

    /// Consumes the publication and returns the live socket.
    pub fn into_socket(self) -> Socket {
        self.socket
    }

    /// Consumes the publication and returns all parts.
    pub fn into_parts(self) -> PublicationParts {
        PublicationParts {
            id: self.id,
            config: self.config,
            socket: self.socket,
        }
    }

    /// Returns a metrics snapshot for one publication.
    #[cfg(feature = "metrics")]
    pub fn metrics_snapshot(&self) -> PublicationMetricsSnapshot {
        self.metrics.snapshot()
    }

    fn configure_socket(
        socket: &Socket,
        config: &PublicationConfig,
        existing_socket: bool,
    ) -> Result<SocketAddr, MctxError> {
        if existing_socket {
            Self::validate_existing_socket(socket, config)?;
        }

        socket
            .set_nonblocking(true)
            .map_err(MctxError::SocketOptionFailed)?;

        match config.family() {
            crate::PublicationAddressFamily::Ipv4 => Self::configure_socket_v4(socket, config),
            crate::PublicationAddressFamily::Ipv6 => {
                if !existing_socket {
                    socket
                        .set_only_v6(true)
                        .map_err(MctxError::SocketOptionFailed)?;
                }
                Self::configure_socket_v6(socket, config)
            }
        }
    }

    fn configure_socket_v4(
        socket: &Socket,
        config: &PublicationConfig,
    ) -> Result<SocketAddr, MctxError> {
        if let Some(bind_addr) = Self::bind_addr_v4(config) {
            Self::bind_if_needed(socket, SocketAddr::V4(bind_addr))?;
        }

        if let Some(OutgoingInterface::Ipv4Addr(interface)) = config.outgoing_interface {
            socket
                .set_multicast_if_v4(&interface)
                .map_err(MctxError::SocketOptionFailed)?;
        }

        socket
            .set_multicast_loop_v4(config.loopback)
            .map_err(MctxError::SocketOptionFailed)?;
        socket
            .set_multicast_ttl_v4(config.ttl)
            .map_err(MctxError::SocketOptionFailed)?;

        let destination = SocketAddrV4::new(Self::group_v4(config), config.dst_port);
        socket
            .connect(&SockAddr::from(destination))
            .map_err(MctxError::SocketConnectFailed)?;

        Ok(SocketAddr::V4(destination))
    }

    fn configure_socket_v6(
        socket: &Socket,
        config: &PublicationConfig,
    ) -> Result<SocketAddr, MctxError> {
        let group = Self::group_v6(config);
        let explicit_source = Self::source_addr_v6(config);
        let interface_addr = Self::interface_addr_v6(config);
        let explicit_interface_index = Self::explicit_interface_index_v6(config, interface_addr)?;
        let source_interface_index = match explicit_source {
            Some(source) if source.is_unicast_link_local() => match explicit_interface_index {
                Some(interface_index) => Some(interface_index),
                None => Some(resolve_ipv6_interface_index(source)?),
            },
            Some(source) => Some(resolve_ipv6_interface_index(source)?),
            None => None,
        };

        if let (Some(source), Some(source_interface_index), Some(outgoing_interface_index)) = (
            explicit_source,
            source_interface_index,
            explicit_interface_index,
        ) && source_interface_index != outgoing_interface_index
        {
            return Err(MctxError::Ipv6SourceInterfaceMismatch {
                source_addr: IpAddr::V6(source),
                source_interface_index,
                outgoing_interface_index,
            });
        }

        let effective_interface_index = explicit_interface_index.or(source_interface_index);
        let bind_source = explicit_source.or(interface_addr);
        let bind_scope_id = match bind_source {
            Some(source) if source.is_unicast_link_local() => effective_interface_index
                .ok_or_else(|| {
                    MctxError::InterfaceDiscoveryFailed(format!(
                        "failed to determine interface index for link-local IPv6 address {source}"
                    ))
                })?,
            _ => 0,
        };

        if let Some(bind_addr) = Self::bind_addr_v6(config, bind_source, bind_scope_id) {
            Self::bind_if_needed(socket, SocketAddr::V6(bind_addr))?;
        }

        if let Some(interface_index) = effective_interface_index {
            socket
                .set_multicast_if_v6(interface_index)
                .map_err(MctxError::SocketOptionFailed)?;
        }

        socket
            .set_multicast_loop_v6(config.loopback)
            .map_err(MctxError::SocketOptionFailed)?;
        socket
            .set_multicast_hops_v6(config.ttl)
            .map_err(MctxError::SocketOptionFailed)?;

        let destination_scope_id = match config.ipv6_scope() {
            Some(Ipv6MulticastScope::InterfaceLocal | Ipv6MulticastScope::LinkLocal) => {
                effective_interface_index.ok_or(MctxError::Ipv6ScopedMulticastRequiresInterface)?
            }
            _ => ipv6_destination_scope_id(group, effective_interface_index.unwrap_or(0)),
        };
        let destination = SocketAddrV6::new(group, config.dst_port, 0, destination_scope_id);
        socket
            .connect(&SockAddr::from(destination))
            .map_err(MctxError::SocketConnectFailed)?;

        Ok(SocketAddr::V6(destination))
    }

    fn validate_existing_socket(
        socket: &Socket,
        config: &PublicationConfig,
    ) -> Result<(), MctxError> {
        let Ok(local_addr) = socket.local_addr() else {
            return Ok(());
        };

        match local_addr.as_socket() {
            Some(SocketAddr::V4(local_v4)) => {
                if config.is_ipv6() {
                    return Err(MctxError::ExistingSocketAddressFamilyMismatch);
                }

                if let Some(IpAddr::V4(expected)) = config.source_addr
                    && local_v4.ip() != &Ipv4Addr::UNSPECIFIED
                    && local_v4.ip() != &expected
                {
                    return Err(MctxError::ExistingSocketAddressMismatch {
                        expected: IpAddr::V4(expected),
                        actual: IpAddr::V4(*local_v4.ip()),
                    });
                }

                if let Some(expected) = config.source_port
                    && local_v4.port() != 0
                    && local_v4.port() != expected
                {
                    return Err(MctxError::ExistingSocketPortMismatch {
                        expected,
                        actual: local_v4.port(),
                    });
                }

                Ok(())
            }
            Some(SocketAddr::V6(local_v6)) => {
                if config.is_ipv4() {
                    return Err(MctxError::ExistingSocketAddressFamilyMismatch);
                }

                if let Some(IpAddr::V6(expected)) = config.source_addr
                    && local_v6.ip() != &Ipv6Addr::UNSPECIFIED
                    && local_v6.ip() != &expected
                {
                    return Err(MctxError::ExistingSocketAddressMismatch {
                        expected: IpAddr::V6(expected),
                        actual: IpAddr::V6(*local_v6.ip()),
                    });
                }

                if let Some(expected) = config.source_port
                    && local_v6.port() != 0
                    && local_v6.port() != expected
                {
                    return Err(MctxError::ExistingSocketPortMismatch {
                        expected,
                        actual: local_v6.port(),
                    });
                }

                Ok(())
            }
            None => Err(MctxError::ExistingSocketAddressFamilyMismatch),
        }
    }

    fn bind_if_needed(socket: &Socket, bind_addr: SocketAddr) -> Result<(), MctxError> {
        let needs_bind = match socket.local_addr() {
            Ok(local_addr) => match local_addr.as_socket() {
                Some(local_addr) => local_addr != bind_addr,
                None => return Err(MctxError::ExistingSocketAddressFamilyMismatch),
            },
            Err(_) => true,
        };

        if needs_bind {
            socket
                .bind(&SockAddr::from(bind_addr))
                .map_err(MctxError::SocketBindFailed)?;
        }

        Ok(())
    }

    fn bind_addr_v4(config: &PublicationConfig) -> Option<SocketAddrV4> {
        if config.source_addr.is_none() && config.source_port.is_none() {
            return None;
        }

        Some(SocketAddrV4::new(
            match config.source_addr {
                Some(IpAddr::V4(source_addr)) => source_addr,
                Some(IpAddr::V6(_)) => unreachable!("validated as IPv4"),
                None => Ipv4Addr::UNSPECIFIED,
            },
            config.source_port.unwrap_or(0),
        ))
    }

    fn bind_addr_v6(
        config: &PublicationConfig,
        bind_source: Option<Ipv6Addr>,
        bind_scope_id: u32,
    ) -> Option<SocketAddrV6> {
        if bind_source.is_none() && config.source_port.is_none() {
            return None;
        }

        Some(SocketAddrV6::new(
            bind_source.unwrap_or(Ipv6Addr::UNSPECIFIED),
            config.source_port.unwrap_or(0),
            0,
            bind_scope_id,
        ))
    }

    fn group_v4(config: &PublicationConfig) -> Ipv4Addr {
        match config.group {
            IpAddr::V4(group) => group,
            IpAddr::V6(_) => unreachable!("validated as IPv4"),
        }
    }

    fn group_v6(config: &PublicationConfig) -> Ipv6Addr {
        match config.group {
            IpAddr::V4(_) => unreachable!("validated as IPv6"),
            IpAddr::V6(group) => group,
        }
    }

    fn source_addr_v6(config: &PublicationConfig) -> Option<Ipv6Addr> {
        match config.source_addr {
            Some(IpAddr::V4(_)) => unreachable!("validated as IPv6"),
            Some(IpAddr::V6(source)) => Some(source),
            None => None,
        }
    }

    fn interface_addr_v6(config: &PublicationConfig) -> Option<Ipv6Addr> {
        match config.outgoing_interface {
            Some(OutgoingInterface::Ipv4Addr(_)) => unreachable!("validated as IPv6"),
            Some(OutgoingInterface::Ipv6Addr(interface)) => Some(interface),
            Some(OutgoingInterface::Ipv6Index(_)) | None => None,
        }
    }

    fn explicit_interface_index_v6(
        config: &PublicationConfig,
        interface_addr: Option<Ipv6Addr>,
    ) -> Result<Option<u32>, MctxError> {
        match config.outgoing_interface {
            Some(OutgoingInterface::Ipv4Addr(_)) => unreachable!("validated as IPv6"),
            Some(OutgoingInterface::Ipv6Index(index)) => Ok(Some(index)),
            Some(OutgoingInterface::Ipv6Addr(_)) => {
                interface_addr.map(resolve_ipv6_interface_index).transpose()
            }
            None => Ok(None),
        }
    }
}

#[cfg(unix)]
impl AsRawFd for Publication {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

#[cfg(windows)]
impl AsRawSocket for Publication {
    fn as_raw_socket(&self) -> RawSocket {
        self.socket.as_raw_socket()
    }
}

#[cfg(feature = "metrics")]
#[derive(Debug, Default)]
struct PublicationMetricsState {
    send_calls: std::sync::atomic::AtomicU64,
    packets_sent: std::sync::atomic::AtomicU64,
    bytes_sent: std::sync::atomic::AtomicU64,
    send_errors: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "metrics")]
impl PublicationMetricsState {
    fn record_success(&self, bytes_sent: usize) {
        use std::sync::atomic::Ordering::Relaxed;

        self.send_calls.fetch_add(1, Relaxed);
        self.packets_sent.fetch_add(1, Relaxed);
        self.bytes_sent.fetch_add(bytes_sent as u64, Relaxed);
    }

    fn record_error(&self) {
        use std::sync::atomic::Ordering::Relaxed;

        self.send_calls.fetch_add(1, Relaxed);
        self.send_errors.fetch_add(1, Relaxed);
    }

    fn snapshot(&self) -> PublicationMetricsSnapshot {
        use std::sync::atomic::Ordering::Relaxed;

        PublicationMetricsSnapshot {
            send_calls: self.send_calls.load(Relaxed),
            packets_sent: self.packets_sent.load(Relaxed),
            bytes_sent: self.bytes_sent.load(Relaxed),
            send_errors: self.send_errors.load(Relaxed),
            captured_at: SystemTime::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "metrics")]
    use crate::metrics::PublicationMetricsSampler;
    use crate::test_support::{
        TEST_GROUP, TEST_GROUP_V6_GLOBAL, TEST_GROUP_V6_SAME_HOST, recv_payload,
        recv_payload_with_source, test_multicast_receiver, test_multicast_receiver_v6,
    };
    use socket2::{Domain, Protocol, SockAddr, Type};

    #[test]
    fn publication_send_reaches_a_local_receiver() {
        let (receiver, port) = test_multicast_receiver();
        let publication =
            Publication::new(PublicationId(1), PublicationConfig::new(TEST_GROUP, port)).unwrap();

        let report = publication.send(b"hello multicast").unwrap();
        let payload = recv_payload(&receiver);
        let announce = publication.announce_tuple().unwrap();

        assert_eq!(
            report.destination,
            SocketAddr::V4(SocketAddrV4::new(TEST_GROUP, port))
        );
        assert!(report.local_addr.is_some());
        assert_eq!(report.source_addr, report.local_addr.map(|addr| addr.ip()));
        assert_eq!(announce.1, IpAddr::V4(TEST_GROUP));
        assert_eq!(announce.2, port);
        assert_eq!(payload, b"hello multicast");
    }

    #[test]
    fn publication_send_reaches_a_local_ipv6_receiver_with_configured_source() {
        let interface = Ipv6Addr::LOCALHOST;
        let source = Ipv6Addr::LOCALHOST;
        let (receiver, port) = test_multicast_receiver_v6(TEST_GROUP_V6_SAME_HOST, interface);
        let publication = Publication::new(
            PublicationId(1),
            PublicationConfig::new(TEST_GROUP_V6_SAME_HOST, port)
                .with_source_addr(source)
                .with_outgoing_interface(interface),
        )
        .unwrap();

        let report = publication.send(b"hello multicast v6").unwrap();
        let (payload, sender) = recv_payload_with_source(&receiver);

        assert_eq!(
            report.destination,
            SocketAddr::V6(SocketAddrV6::new(
                TEST_GROUP_V6_SAME_HOST,
                port,
                0,
                publication.destination_v6().unwrap().scope_id()
            ))
        );
        assert_eq!(report.source_addr, Some(IpAddr::V6(source)));
        assert_eq!(sender.ip(), IpAddr::V6(source));
        assert_eq!(payload, b"hello multicast v6");
    }

    #[test]
    fn ipv6_interface_address_auto_binds_the_sender_source() {
        let interface = Ipv6Addr::LOCALHOST;
        let (receiver, port) = test_multicast_receiver_v6(TEST_GROUP_V6_SAME_HOST, interface);
        let publication = Publication::new(
            PublicationId(1),
            PublicationConfig::new(TEST_GROUP_V6_SAME_HOST, port)
                .with_outgoing_interface(interface),
        )
        .unwrap();

        let report = publication.send(b"auto-bind v6").unwrap();
        let (_payload, sender) = recv_payload_with_source(&receiver);

        assert_eq!(report.source_addr, Some(IpAddr::V6(interface)));
        assert_eq!(sender.ip(), IpAddr::V6(interface));
    }

    #[test]
    fn wider_scope_ipv6_group_clears_destination_scope_id() {
        let publication = Publication::new(
            PublicationId(1),
            PublicationConfig::new(TEST_GROUP_V6_GLOBAL, 5000)
                .with_source_addr(Ipv6Addr::LOCALHOST),
        )
        .unwrap();

        assert_eq!(publication.destination_v6().unwrap().scope_id(), 0);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn publication_metrics_track_successful_sends() {
        let (_receiver, port) = test_multicast_receiver();
        let publication =
            Publication::new(PublicationId(1), PublicationConfig::new(TEST_GROUP, port)).unwrap();
        let mut sampler = PublicationMetricsSampler::new(&publication);

        assert!(sampler.sample().is_none());
        publication.send(b"metrics packet").unwrap();

        let snapshot = publication.metrics_snapshot();
        let delta = sampler.sample().unwrap();

        assert_eq!(snapshot.send_calls, 1);
        assert_eq!(snapshot.packets_sent, 1);
        assert_eq!(snapshot.bytes_sent, b"metrics packet".len() as u64);
        assert_eq!(snapshot.send_errors, 0);
        assert_eq!(delta.send_calls, 1);
        assert_eq!(delta.packets_sent, 1);
        assert_eq!(delta.bytes_sent, b"metrics packet".len() as u64);
        assert_eq!(delta.send_errors, 0);
    }

    #[test]
    fn existing_socket_source_addr_mismatch_is_rejected() {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        socket
            .bind(&SockAddr::from(SocketAddrV4::new(
                Ipv4Addr::new(127, 0, 0, 1),
                0,
            )))
            .unwrap();

        let result = Publication::new_with_socket(
            PublicationId(1),
            PublicationConfig::new(TEST_GROUP, 5000).with_source_addr(Ipv4Addr::new(127, 0, 0, 2)),
            socket,
        );

        assert!(matches!(
            result,
            Err(MctxError::ExistingSocketAddressMismatch { expected, actual })
                if expected == IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2))
                    && actual == IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        ));
    }
}
