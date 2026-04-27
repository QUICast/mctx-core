#[cfg(feature = "metrics")]
use crate::metrics::PublicationMetricsSnapshot;
use crate::{MctxError, PublicationConfig, SendReport};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
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

                Ok(SendReport {
                    publication_id: self.id,
                    destination: self.destination(),
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

        if let Some(source_port) = config.source_port {
            Self::bind_source_port_if_needed(socket, source_port)?;
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

    fn bind_source_port_if_needed(socket: &Socket, source_port: u16) -> Result<(), MctxError> {
        let needs_bind = match socket.local_addr() {
            Ok(local_addr) => match local_addr.as_socket() {
                Some(SocketAddr::V4(local_v4)) => local_v4.port() == 0,
                Some(SocketAddr::V6(_)) | None => return Err(MctxError::ExistingSocketMustBeIpv4),
            },
            Err(_) => true,
        };

        if needs_bind {
            socket
                .bind(&SockAddr::from(SocketAddrV4::new(
                    Ipv4Addr::UNSPECIFIED,
                    source_port,
                )))
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

    #[test]
    fn publication_send_reaches_a_local_receiver() {
        let (receiver, port) = test_multicast_receiver();
        let publication =
            Publication::new(PublicationId(1), PublicationConfig::new(TEST_GROUP, port)).unwrap();

        let report = publication.send(b"hello multicast").unwrap();
        let payload = recv_payload(&receiver);

        assert_eq!(report.destination, SocketAddrV4::new(TEST_GROUP, port));
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
}
