use std::io;
use thiserror::Error;

/// Errors returned by the multicast sender core.
#[derive(Debug, Error)]
pub enum MctxError {
    /// The configured destination port is invalid.
    #[error("MCTX: invalid destination port")]
    InvalidDestinationPort,

    /// The configured group address is not a valid multicast IPv4 address.
    #[error("MCTX: group must be a multicast IPv4 address")]
    InvalidMulticastGroup,

    /// The configured source port is invalid.
    #[error("MCTX: invalid source port")]
    InvalidSourcePort,

    /// The configured multicast interface address is invalid.
    #[error("MCTX: invalid interface address")]
    InvalidInterfaceAddress,

    /// A publication with the same configuration already exists.
    #[error("MCTX: publication already exists")]
    DuplicatePublication,

    /// No publication with the requested ID exists.
    #[error("MCTX: publication not found")]
    PublicationNotFound,

    /// Creating the UDP socket failed.
    #[error("MCTX: failed to create UDP socket: {0}")]
    SocketCreateFailed(io::Error),

    /// Setting a socket option failed.
    #[error("MCTX: failed to set socket option: {0}")]
    SocketOptionFailed(io::Error),

    /// Binding the UDP socket failed.
    #[error("MCTX: failed to bind UDP socket: {0}")]
    SocketBindFailed(io::Error),

    /// Connecting the UDP socket failed.
    #[error("MCTX: failed to connect UDP socket: {0}")]
    SocketConnectFailed(io::Error),

    /// Reading the local address from a socket failed.
    #[error("MCTX: failed to read local address from socket: {0}")]
    SocketLocalAddrFailed(io::Error),

    /// The provided existing socket does not match the current IPv4-only send model.
    #[error("MCTX: existing socket must be an IPv4 UDP socket")]
    ExistingSocketMustBeIpv4,

    /// The provided existing socket is bound to a different UDP port than requested.
    #[error("MCTX: existing socket is bound to UDP port {actual}, expected {expected}")]
    ExistingSocketPortMismatch { expected: u16, actual: u16 },

    /// Sending a packet failed.
    #[error("MCTX: send failed: {0}")]
    SendFailed(io::Error),
}

impl MctxError {
    #[cfg(feature = "tokio")]
    pub(crate) fn is_would_block(&self) -> bool {
        matches!(self, Self::SendFailed(error) if error.kind() == io::ErrorKind::WouldBlock)
    }
}
