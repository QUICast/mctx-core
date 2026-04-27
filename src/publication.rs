#[cfg(feature = "metrics")]
use crate::metrics::PublicationMetricsSnapshot;
use crate::{MctxError, PublicationConfig, SendReport};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};

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
    #[cfg(feature = "metrics")]
    metrics: PublicationMetricsState,
}

impl Publication {
    /// Creates and configures a new multicast publication socket.
    pub fn new(id: PublicationId, config: PublicationConfig) -> Result<Self, MctxError> {
        config.validate()?;

        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(MctxError::SocketCreateFailed)?;

        Self::configure_socket(&socket, &config, false)?;

        Ok(Self {
            id,
            config,
            socket,
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
        Self::configure_socket(&socket, &config, true)?;

        Ok(Self {
            id,
            config,
            socket,
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

    /// Returns the destination address.
    pub fn destination(&self) -> SocketAddrV4 {
        self.config.destination()
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

                let local_addr = self.local_addr_v4().ok();

                Ok(SendReport {
                    publication_id: self.id,
                    destination: self.destination(),
                    local_addr,
                    source_addr: local_addr.map(|addr| *addr.ip()),
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
            .ok_or(MctxError::ExistingSocketMustBeIpv4)
    }

    /// Returns the publication socket local IPv4 address.
    pub fn local_addr_v4(&self) -> Result<SocketAddrV4, MctxError> {
        match self.local_addr()? {
            SocketAddr::V4(local_addr) => Ok(local_addr),
            SocketAddr::V6(_) => Err(MctxError::ExistingSocketMustBeIpv4),
        }
    }

    /// Returns the effective local IPv4 source address.
    pub fn source_addr(&self) -> Result<Ipv4Addr, MctxError> {
        Ok(*self.local_addr_v4()?.ip())
    }

    /// Returns the `(source, group, udp_port)` tuple needed for announce frames.
    pub fn announce_tuple(&self) -> Result<(Ipv4Addr, Ipv4Addr, u16), MctxError> {
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
    ) -> Result<(), MctxError> {
        if existing_socket {
            Self::validate_existing_socket(socket, config)?;
        }

        socket
            .set_nonblocking(true)
            .map_err(MctxError::SocketOptionFailed)?;

        if let Some(bind_addr) = config.bind_addr() {
            Self::bind_if_needed(socket, bind_addr)?;
        }

        if let Some(interface) = config.interface {
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
        socket
            .connect(&SockAddr::from(config.destination()))
            .map_err(MctxError::SocketConnectFailed)?;

        Ok(())
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
                if let Some(expected) = config.source_addr
                    && local_v4.ip() != &Ipv4Addr::UNSPECIFIED
                    && local_v4.ip() != &expected
                {
                    return Err(MctxError::ExistingSocketAddressMismatch {
                        expected,
                        actual: *local_v4.ip(),
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
            Some(SocketAddr::V6(_)) | None => Err(MctxError::ExistingSocketMustBeIpv4),
        }
    }

    fn bind_if_needed(socket: &Socket, bind_addr: SocketAddrV4) -> Result<(), MctxError> {
        let needs_bind = match socket.local_addr() {
            Ok(local_addr) => match local_addr.as_socket() {
                Some(SocketAddr::V4(local_v4)) => local_v4 != bind_addr,
                Some(SocketAddr::V6(_)) | None => return Err(MctxError::ExistingSocketMustBeIpv4),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "metrics")]
    use crate::metrics::PublicationMetricsSampler;
    use crate::test_support::{TEST_GROUP, recv_payload, test_multicast_receiver};
    use socket2::{Domain, Protocol, SockAddr, Type};

    #[test]
    fn publication_send_reaches_a_local_receiver() {
        let (receiver, port) = test_multicast_receiver();
        let publication =
            Publication::new(PublicationId(1), PublicationConfig::new(TEST_GROUP, port)).unwrap();

        let report = publication.send(b"hello multicast").unwrap();
        let payload = recv_payload(&receiver);
        let announce = publication.announce_tuple().unwrap();

        assert_eq!(report.destination, SocketAddrV4::new(TEST_GROUP, port));
        assert!(report.local_addr.is_some());
        assert_eq!(report.source_addr, report.local_addr.map(|addr| *addr.ip()));
        assert_eq!(announce.1, TEST_GROUP);
        assert_eq!(announce.2, port);
        assert_eq!(payload, b"hello multicast");
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn publication_metrics_track_successful_sends() {
        let (_receiver, port) = test_multicast_receiver();
        let publication =
            Publication::new(PublicationId(1), PublicationConfig::new(TEST_GROUP, port)).unwrap();
        let sampler = PublicationMetricsSampler::new(&publication);

        publication.send(b"metrics packet").unwrap();

        let delta = sampler.delta();
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
            Err(MctxError::ExistingSocketAddressMismatch {
                expected,
                actual
            }) if expected == Ipv4Addr::new(127, 0, 0, 2)
                && actual == Ipv4Addr::new(127, 0, 0, 1)
        ));
    }
}
