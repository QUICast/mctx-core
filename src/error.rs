use std::{io, net::IpAddr};
use thiserror::Error;

/// Errors returned by the multicast sender core.
#[derive(Debug, Error)]
pub enum MctxError {
    /// The configured destination port is invalid.
    #[error("MCTX: invalid destination port")]
    InvalidDestinationPort,

    /// The configured group address is not a valid multicast IP address.
    #[error("MCTX: group must be a multicast IPv4 or IPv6 address")]
    InvalidMulticastGroup,

    /// The configured source port is invalid.
    #[error("MCTX: invalid source port")]
    InvalidSourcePort,

    /// The configured source IP address is invalid.
    #[error("MCTX: invalid source address")]
    InvalidSourceAddress,

    /// The configured multicast interface selector is invalid.
    #[error("MCTX: invalid interface address")]
    InvalidInterfaceAddress,

    /// The configured IPv6 interface index is invalid.
    #[error("MCTX: invalid IPv6 interface index")]
    InvalidIpv6InterfaceIndex,

    /// The configured raw bind address is invalid.
    #[error("MCTX: invalid raw bind address")]
    InvalidRawBindAddress,

    /// The configured source address does not match the group address family.
    #[error("MCTX: source address family must match multicast group family")]
    SourceAddressFamilyMismatch,

    /// The configured outgoing interface does not match the group address family.
    #[error("MCTX: outgoing interface family must match multicast group family")]
    OutgoingInterfaceFamilyMismatch,

    /// The configured raw bind address does not match the expected datagram family.
    #[error("MCTX: raw bind address family must match the raw publication family")]
    RawBindAddressFamilyMismatch,

    /// The configured IPv6 source address and outgoing interface disagree about
    /// which interface should be used.
    #[error(
        "MCTX: IPv6 source address {source_addr} resolves to interface index {source_interface_index}, expected {outgoing_interface_index}"
    )]
    Ipv6SourceInterfaceMismatch {
        source_addr: IpAddr,
        source_interface_index: u32,
        outgoing_interface_index: u32,
    },

    /// A scoped IPv6 multicast destination needs a concrete interface index.
    #[error(
        "MCTX: IPv6 interface-local and link-local multicast destinations require an outgoing interface or source address"
    )]
    Ipv6ScopedMulticastRequiresInterface,

    /// Resolving a local IPv6 address to its interface index failed.
    #[error("MCTX: failed to resolve IPv6 interface: {0}")]
    InterfaceDiscoveryFailed(String),

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

    /// The provided existing socket does not match the configured IP family.
    #[error("MCTX: existing socket address family does not match the publication")]
    ExistingSocketAddressFamilyMismatch,

    /// The provided existing socket is bound to a different UDP port than requested.
    #[error("MCTX: existing socket is bound to UDP port {actual}, expected {expected}")]
    ExistingSocketPortMismatch { expected: u16, actual: u16 },

    /// The provided existing socket is bound to a different local IP address than requested.
    #[error("MCTX: existing socket is bound to local IP address {actual}, expected {expected}")]
    ExistingSocketAddressMismatch { expected: IpAddr, actual: IpAddr },

    /// Sending a packet failed.
    #[error("MCTX: send failed: {0}")]
    SendFailed(io::Error),

    /// Raw packet transmit is not supported on the current platform or configuration.
    #[error("MCTX: raw packet transmit is unsupported: {0}")]
    RawPacketTransmitUnsupported(String),

    /// Creating the raw transmit socket failed.
    #[error("MCTX: failed to create raw transmit socket: {0}")]
    RawSocketCreateFailed(io::Error),

    /// Binding the raw transmit socket failed.
    #[error("MCTX: failed to bind raw transmit socket: {0}")]
    RawSocketBindFailed(io::Error),

    /// Sending a raw IP datagram failed.
    #[error("MCTX: raw send failed: {0}")]
    RawSendFailed(io::Error),

    /// The supplied raw datagram bytes are not a valid complete IPv4 or IPv6 datagram.
    #[error("MCTX: invalid raw IP datagram")]
    InvalidRawIpDatagram,

    /// The supplied raw datagram does not target a multicast destination.
    #[error("MCTX: raw datagram destination must be multicast")]
    InvalidRawMulticastDestination,

    /// The supplied IPv6 datagram source does not match the configured local
    /// bind address required by a host-stack raw IPv6 backend.
    ///
    /// The multicast raw-packet backend does not emit this error on Linux,
    /// where remote sources use link-layer injection. The generic `raw-ip`
    /// backend emits it when a kernel-built IPv6 base header would otherwise
    /// select or rewrite a different source address.
    #[error(
        "MCTX: raw datagram source address {datagram_source} does not match configured bind address {configured_bind_addr}"
    )]
    RawDatagramSourceMismatch {
        datagram_source: IpAddr,
        configured_bind_addr: IpAddr,
    },

    /// Raw packet transmit needs an explicit outgoing interface selection.
    #[error("MCTX: raw packet transmit requires an explicit outgoing interface or bind address")]
    RawInterfaceRequired,

    /// Raw packet transmit is not implemented for the selected link type.
    #[error("MCTX: raw packet transmit does not support link type {0}")]
    RawUnsupportedLinkType(String),
}

#[cfg(all(feature = "tokio", unix))]
impl MctxError {
    pub(crate) fn is_would_block(&self) -> bool {
        matches!(
            self,
            Self::SendFailed(error) | Self::RawSendFailed(error)
                if error.kind() == io::ErrorKind::WouldBlock
        )
    }
}
