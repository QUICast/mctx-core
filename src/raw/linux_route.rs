use crate::error::MctxError;
use crate::raw::linux_packet;
use socket2::Socket;
use std::fmt;
use std::io;
use std::mem::size_of;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;

const NETLINK_INITIAL_BUFFER_LEN: usize = 8 * 1024;
const NETLINK_MAX_DATAGRAM_LEN: usize = 1024 * 1024;
const NLM_F_DUMP_BITS: u16 = 0x300;
const NLM_F_DUMP_INTR_BIT: u16 = 0x10;
const NLA_TYPE_MASK_BITS: u16 = 0x3fff;
const RTNH_F_DEAD_BIT: u8 = 0x01;

#[repr(C)]
#[derive(Clone, Copy)]
struct RouteMessage {
    family: u8,
    destination_len: u8,
    source_len: u8,
    tos: u8,
    table: u8,
    protocol: u8,
    scope: u8,
    kind: u8,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RouteAttribute {
    len: u16,
    kind: u16,
}

#[repr(C)]
struct RouteDumpRequest {
    header: libc::nlmsghdr,
    route: RouteMessage,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RouteNextHop {
    len: u16,
    flags: u8,
    hops: u8,
    interface_index: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteDisposition {
    Interface(u32),
    Error(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteCandidate {
    prefix_len: u8,
    priority: u32,
    disposition: RouteDisposition,
}

impl RouteCandidate {
    fn is_better_than(self, current: Option<Self>) -> bool {
        current.is_none_or(|current| {
            self.prefix_len > current.prefix_len
                || (self.prefix_len == current.prefix_len && self.priority < current.priority)
        })
    }
}

#[derive(Debug)]
struct CachedIpv6Route {
    destination: Ipv6Addr,
    interface_index: u32,
    socket: Arc<Socket>,
}

pub(crate) struct LinuxIpv6RouteEgress {
    request_socket: OwnedFd,
    notification_socket: OwnedFd,
    next_sequence: u32,
    response_buffer: Vec<u8>,
    cached: Option<CachedIpv6Route>,
}

impl fmt::Debug for LinuxIpv6RouteEgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinuxIpv6RouteEgress")
            .field("next_sequence", &self.next_sequence)
            .field("response_buffer_capacity", &self.response_buffer.capacity())
            .field("cached", &self.cached)
            .finish_non_exhaustive()
    }
}

impl LinuxIpv6RouteEgress {
    pub(crate) fn new() -> Result<Self, MctxError> {
        Ok(Self {
            request_socket: open_netlink_socket(0, false)?,
            notification_socket: open_netlink_socket(
                (libc::RTMGRP_LINK | libc::RTMGRP_IPV6_ROUTE) as u32,
                true,
            )?,
            next_sequence: 1,
            response_buffer: Vec::with_capacity(NETLINK_INITIAL_BUFFER_LEN),
            cached: None,
        })
    }

    pub(crate) fn egress(
        &mut self,
        destination: Ipv6Addr,
    ) -> Result<(u32, Arc<Socket>), MctxError> {
        ensure_route_selectable_scope(destination)?;

        if drain_invalidation_notifications(&self.notification_socket)
            .map_err(MctxError::RawSendFailed)?
        {
            self.cached = None;
        }

        if let Some(cached) = self.cached.as_ref()
            && cached.destination == destination
        {
            return Ok((cached.interface_index, Arc::clone(&cached.socket)));
        }

        let interface_index = self.lookup(destination).map_err(MctxError::RawSendFailed)?;
        let socket = Arc::new(linux_packet::open_ipv6(interface_index)?);
        self.cached = Some(CachedIpv6Route {
            destination,
            interface_index,
            socket: Arc::clone(&socket),
        });
        Ok((interface_index, socket))
    }

    pub(crate) fn invalidate(&mut self, interface_index: u32) {
        if self
            .cached
            .as_ref()
            .is_some_and(|cached| cached.interface_index == interface_index)
        {
            self.cached = None;
        }
    }

    fn lookup(&mut self, destination: Ipv6Addr) -> io::Result<u32> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);

        let request = RouteDumpRequest {
            header: libc::nlmsghdr {
                nlmsg_len: u32::try_from(size_of::<RouteDumpRequest>())
                    .expect("route request fits in u32"),
                nlmsg_type: libc::RTM_GETROUTE,
                nlmsg_flags: libc::NLM_F_REQUEST as u16 | NLM_F_DUMP_BITS,
                nlmsg_seq: sequence,
                nlmsg_pid: 0,
            },
            route: RouteMessage {
                family: libc::AF_INET6 as u8,
                destination_len: 0,
                source_len: 0,
                tos: 0,
                table: libc::RT_TABLE_MAIN,
                protocol: 0,
                scope: 0,
                kind: 0,
                flags: 0,
            },
        };

        let kernel = netlink_address(0);
        // SAFETY: request and kernel are initialized C-compatible structures
        // whose storage remains valid for the synchronous sendto call.
        let sent = unsafe {
            libc::sendto(
                self.request_socket.as_raw_fd(),
                (&request as *const RouteDumpRequest).cast(),
                size_of::<RouteDumpRequest>(),
                0,
                (&kernel as *const libc::sockaddr_nl).cast(),
                size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if sent == -1 {
            return Err(io::Error::last_os_error());
        }
        if sent as usize != size_of::<RouteDumpRequest>() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "partial NETLINK_ROUTE request",
            ));
        }

        receive_route_dump(
            &self.request_socket,
            sequence,
            destination,
            &mut self.response_buffer,
        )
    }
}

fn open_netlink_socket(groups: u32, nonblocking: bool) -> Result<OwnedFd, MctxError> {
    let mut socket_flags = libc::SOCK_RAW | libc::SOCK_CLOEXEC;
    if nonblocking {
        socket_flags |= libc::SOCK_NONBLOCK;
    }

    // SAFETY: socket is called with Linux NETLINK_ROUTE constants and the
    // returned descriptor is immediately transferred to OwnedFd.
    let raw_fd = unsafe { libc::socket(libc::AF_NETLINK, socket_flags, libc::NETLINK_ROUTE) };
    if raw_fd == -1 {
        return Err(MctxError::RawSocketCreateFailed(io::Error::last_os_error()));
    }
    // SAFETY: raw_fd is newly created and uniquely owned here.
    let socket = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let address = netlink_address(groups);
    // SAFETY: address is a fully initialized sockaddr_nl.
    let result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_nl).cast(),
            size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if result == -1 {
        return Err(MctxError::RawSocketBindFailed(io::Error::last_os_error()));
    }

    if !nonblocking {
        let timeout = libc::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        // SAFETY: timeout points to a valid timeval for SO_RCVTIMEO.
        let result = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if result == -1 {
            return Err(MctxError::SocketOptionFailed(io::Error::last_os_error()));
        }
    }

    Ok(socket)
}

fn netlink_address(groups: u32) -> libc::sockaddr_nl {
    // SAFETY: zero is a valid initial representation for sockaddr_nl; public
    // fields are then populated explicitly.
    let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_nl>() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    address.nl_pid = 0;
    address.nl_groups = groups;
    address
}

fn receive_route_dump(
    socket: &OwnedFd,
    sequence: u32,
    destination: Ipv6Addr,
    response_buffer: &mut Vec<u8>,
) -> io::Result<u32> {
    let mut best = None;
    loop {
        let received = receive_netlink_datagram(socket, response_buffer)?;
        if parse_route_dump_datagram(
            &response_buffer[..received],
            sequence,
            destination,
            &mut best,
        )? {
            return match best.map(|candidate| candidate.disposition) {
                Some(RouteDisposition::Interface(interface_index)) => Ok(interface_index),
                Some(RouteDisposition::Error(error)) => Err(io::Error::from_raw_os_error(error)),
                None => Err(io::Error::from_raw_os_error(libc::ENETUNREACH)),
            };
        }
    }
}

fn receive_netlink_datagram(socket: &OwnedFd, buffer: &mut Vec<u8>) -> io::Result<usize> {
    let datagram_len = loop {
        let mut first_byte = 0u8;
        // MSG_TRUNC returns the complete datagram length while MSG_PEEK leaves
        // it queued for the following receive.
        // SAFETY: first_byte is writable and the socket remains open.
        let received = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                (&mut first_byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_TRUNC,
            )
        };
        if received == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        break received as usize;
    };

    if datagram_len == 0 || datagram_len > NETLINK_MAX_DATAGRAM_LEN {
        return Err(invalid_netlink_data("invalid netlink datagram length"));
    }
    if buffer.len() < datagram_len {
        buffer.resize(datagram_len, 0);
    }

    loop {
        // SAFETY: the vector has writable initialized storage for datagram_len
        // bytes and recv initializes the returned prefix.
        let received = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                datagram_len,
                0,
            )
        };
        if received == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if received as usize != datagram_len {
            return Err(invalid_netlink_data("netlink datagram length changed"));
        }
        return Ok(datagram_len);
    }
}

fn parse_route_dump_datagram(
    buffer: &[u8],
    sequence: u32,
    destination: Ipv6Addr,
    best: &mut Option<RouteCandidate>,
) -> io::Result<bool> {
    let mut offset = 0usize;
    while offset + size_of::<libc::nlmsghdr>() <= buffer.len() {
        let header = read_unaligned::<libc::nlmsghdr>(buffer, offset)?;
        let message_len = header.nlmsg_len as usize;
        if message_len < size_of::<libc::nlmsghdr>() || offset + message_len > buffer.len() {
            return Err(invalid_netlink_data("invalid netlink message length"));
        }

        if header.nlmsg_seq == sequence {
            if header.nlmsg_flags & NLM_F_DUMP_INTR_BIT != 0 {
                return Err(io::Error::from_raw_os_error(libc::EAGAIN));
            }
            let payload_offset = offset + size_of::<libc::nlmsghdr>();
            match header.nlmsg_type as i32 {
                libc::NLMSG_ERROR => {
                    let error = read_unaligned::<i32>(buffer, payload_offset)?;
                    if error != 0 {
                        return Err(io::Error::from_raw_os_error(error.saturating_neg()));
                    }
                }
                libc::NLMSG_DONE => {
                    if payload_offset + size_of::<i32>() <= offset + message_len {
                        let error = read_unaligned::<i32>(buffer, payload_offset)?;
                        if error != 0 {
                            return Err(io::Error::from_raw_os_error(error.saturating_neg()));
                        }
                    }
                    return Ok(true);
                }
                message_type if message_type == libc::RTM_NEWROUTE as i32 => {
                    if let Some(candidate) = parse_route_candidate(
                        &buffer[payload_offset..offset + message_len],
                        destination,
                    )? && candidate.is_better_than(*best)
                    {
                        *best = Some(candidate);
                    }
                }
                libc::NLMSG_OVERRUN => {
                    return Err(io::Error::from_raw_os_error(libc::ENOBUFS));
                }
                _ => {}
            }
        }

        offset += netlink_align(message_len);
    }

    Ok(false)
}

fn parse_route_candidate(
    payload: &[u8],
    destination: Ipv6Addr,
) -> io::Result<Option<RouteCandidate>> {
    let route = read_unaligned::<RouteMessage>(payload, 0)?;
    if route.family as i32 != libc::AF_INET6 || route.destination_len > 128 {
        return Ok(None);
    }

    let mut route_table = u32::from(route.table);
    let mut prefix = Ipv6Addr::UNSPECIFIED;
    let mut interface_index = None;
    let mut multipath_interface_index = None;
    let mut priority = 0u32;
    let mut offset = netlink_align(size_of::<RouteMessage>());
    while offset + size_of::<RouteAttribute>() <= payload.len() {
        let attribute = read_unaligned::<RouteAttribute>(payload, offset)?;
        let attribute_len = attribute.len as usize;
        if attribute_len < size_of::<RouteAttribute>() || offset + attribute_len > payload.len() {
            return Err(invalid_netlink_data("invalid route attribute length"));
        }

        let value_offset = offset + size_of::<RouteAttribute>();
        let value = &payload[value_offset..offset + attribute_len];
        match attribute.kind & NLA_TYPE_MASK_BITS {
            libc::RTA_DST => {
                prefix = Ipv6Addr::from(read_unaligned::<[u8; 16]>(value, 0)?);
            }
            libc::RTA_OIF => interface_index = Some(read_interface_index(value)?),
            libc::RTA_PRIORITY => priority = read_unaligned::<u32>(value, 0)?,
            libc::RTA_TABLE => route_table = read_unaligned::<u32>(value, 0)?,
            libc::RTA_MULTIPATH => {
                multipath_interface_index = parse_multipath_interface(value)?;
            }
            _ => {}
        }

        offset += netlink_align(attribute_len);
    }

    if route_table != u32::from(libc::RT_TABLE_MAIN)
        || !ipv6_prefix_matches(destination, prefix, route.destination_len)
    {
        return Ok(None);
    }

    let disposition = match route.kind {
        libc::RTN_UNICAST | libc::RTN_MULTICAST => RouteDisposition::Interface(
            interface_index
                .or(multipath_interface_index)
                .ok_or_else(|| invalid_netlink_data("usable route has no output interface"))?,
        ),
        libc::RTN_BLACKHOLE | libc::RTN_UNREACHABLE => RouteDisposition::Error(libc::ENETUNREACH),
        libc::RTN_PROHIBIT => RouteDisposition::Error(libc::EACCES),
        _ => return Ok(None),
    };

    Ok(Some(RouteCandidate {
        prefix_len: route.destination_len,
        priority,
        disposition,
    }))
}

fn read_interface_index(value: &[u8]) -> io::Result<u32> {
    let interface_index = read_unaligned::<u32>(value, 0)?;
    if interface_index == 0 {
        return Err(invalid_netlink_data("route returned interface index zero"));
    }
    Ok(interface_index)
}

fn parse_multipath_interface(value: &[u8]) -> io::Result<Option<u32>> {
    let mut offset = 0usize;
    while offset + size_of::<RouteNextHop>() <= value.len() {
        let next_hop = read_unaligned::<RouteNextHop>(value, offset)?;
        let next_hop_len = next_hop.len as usize;
        if next_hop_len < size_of::<RouteNextHop>() || offset + next_hop_len > value.len() {
            return Err(invalid_netlink_data("invalid multipath next-hop length"));
        }
        if next_hop.flags & RTNH_F_DEAD_BIT == 0 && next_hop.interface_index > 0 {
            return Ok(Some(next_hop.interface_index as u32));
        }
        offset += netlink_align(next_hop_len);
    }
    Ok(None)
}

fn ipv6_prefix_matches(destination: Ipv6Addr, prefix: Ipv6Addr, prefix_len: u8) -> bool {
    let destination = destination.octets();
    let prefix = prefix.octets();
    let whole_bytes = usize::from(prefix_len / 8);
    let remaining_bits = prefix_len % 8;

    if destination[..whole_bytes] != prefix[..whole_bytes] {
        return false;
    }
    if remaining_bits == 0 {
        return true;
    }

    let mask = u8::MAX << (8 - remaining_bits);
    destination[whole_bytes] & mask == prefix[whole_bytes] & mask
}

fn drain_invalidation_notifications(socket: &OwnedFd) -> io::Result<bool> {
    let mut buffer = [0u8; NETLINK_INITIAL_BUFFER_LEN];
    let mut invalidated = false;

    loop {
        // SAFETY: buffer is writable and the socket is nonblocking.
        let received = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if received == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(invalidated);
            }
            if error.raw_os_error() == Some(libc::ENOBUFS) {
                return Ok(true);
            }
            return Err(error);
        }
        if received == 0 {
            return Ok(true);
        }

        invalidated |= notification_buffer_invalidates_route(&buffer[..received as usize]);
    }
}

fn notification_buffer_invalidates_route(buffer: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + size_of::<libc::nlmsghdr>() <= buffer.len() {
        let Ok(header) = read_unaligned::<libc::nlmsghdr>(buffer, offset) else {
            return true;
        };
        let message_len = header.nlmsg_len as usize;
        if message_len < size_of::<libc::nlmsghdr>() || offset + message_len > buffer.len() {
            return true;
        }
        if notification_type_invalidates_route(header.nlmsg_type) {
            return true;
        }
        offset += netlink_align(message_len);
    }
    false
}

fn notification_type_invalidates_route(message_type: u16) -> bool {
    matches!(
        message_type,
        libc::RTM_NEWLINK | libc::RTM_DELLINK | libc::RTM_NEWROUTE | libc::RTM_DELROUTE
    )
}

fn ensure_route_selectable_scope(group: Ipv6Addr) -> Result<(), MctxError> {
    let scope = group.octets()[1] & 0x0f;
    if matches!(scope, 1 | 2) {
        return Err(MctxError::Ipv6ScopedMulticastRequiresInterface);
    }
    Ok(())
}

fn read_unaligned<T: Copy>(bytes: &[u8], offset: usize) -> io::Result<T> {
    let end = offset
        .checked_add(size_of::<T>())
        .ok_or_else(|| invalid_netlink_data("netlink offset overflow"))?;
    if end > bytes.len() {
        return Err(invalid_netlink_data("truncated netlink message"));
    }

    // SAFETY: the bounds check above guarantees a readable object-sized
    // region; read_unaligned avoids imposing alignment on the byte buffer.
    Ok(unsafe { std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<T>()) })
}

const fn netlink_align(length: usize) -> usize {
    (length + 3) & !3
}

fn invalid_netlink_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_or_link_notifications_invalidate_cached_egress() {
        assert!(notification_type_invalidates_route(libc::RTM_NEWROUTE));
        assert!(notification_type_invalidates_route(libc::RTM_DELROUTE));
        assert!(notification_type_invalidates_route(libc::RTM_NEWLINK));
        assert!(notification_type_invalidates_route(libc::RTM_DELLINK));
        assert!(!notification_type_invalidates_route(libc::RTM_NEWADDR));
    }

    #[test]
    fn scoped_multicast_requires_an_explicit_interface() {
        assert!(matches!(
            ensure_route_selectable_scope("ff31::8000:1234".parse().unwrap()),
            Err(MctxError::Ipv6ScopedMulticastRequiresInterface)
        ));
        assert!(matches!(
            ensure_route_selectable_scope("ff32::8000:1234".parse().unwrap()),
            Err(MctxError::Ipv6ScopedMulticastRequiresInterface)
        ));
        assert!(ensure_route_selectable_scope("ff3e::8000:1234".parse().unwrap()).is_ok());
    }

    #[test]
    fn route_candidates_prefer_longest_prefix_then_lowest_priority() {
        let default = RouteCandidate {
            prefix_len: 0,
            priority: 10,
            disposition: RouteDisposition::Interface(2),
        };
        let specific = RouteCandidate {
            prefix_len: 64,
            priority: 100,
            disposition: RouteDisposition::Interface(3),
        };
        let preferred_specific = RouteCandidate {
            prefix_len: 64,
            priority: 50,
            disposition: RouteDisposition::Interface(4),
        };

        assert!(default.is_better_than(None));
        assert!(specific.is_better_than(Some(default)));
        assert!(preferred_specific.is_better_than(Some(specific)));
        assert!(!specific.is_better_than(Some(preferred_specific)));
    }

    #[test]
    fn ipv6_prefix_matching_handles_partial_bytes() {
        let destination = "ff3e:1234:8fff::1".parse().unwrap();

        assert!(ipv6_prefix_matches(
            destination,
            "ff3e:1234:8000::".parse().unwrap(),
            33
        ));
        assert!(!ipv6_prefix_matches(
            destination,
            "ff3e:1234::".parse().unwrap(),
            34
        ));
        assert!(ipv6_prefix_matches(destination, Ipv6Addr::UNSPECIFIED, 0));
    }
}
