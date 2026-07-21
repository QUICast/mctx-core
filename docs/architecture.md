# Architecture

The crate is centered on three types:

- `PublicationConfig`: immutable configuration for one multicast destination
- `Publication`: one ready-to-send non-blocking UDP socket
- `Context`: a lightweight owner for multiple publications

Design choices:

- publications are connected UDP sockets, so repeated sends do not rebuild the
  destination address on the hot path
- sockets are non-blocking by default, which keeps them easy to integrate into
  event loops and async adapters
- IPv4 and IPv6 socket setup stay in separate implementation branches
- the resolved destination is stored on the `Publication`, which matters for
  IPv6 scope-ID handling
- the default feature set stays small; metrics, Tokio, and raw packet support
  are opt-in
- `raw-packets` owns multicast forwarding semantics, while `raw-ip` is a
  separate caller-built IP datagram transport primitive for control traffic
- `raw-route-egress` extends only `raw-packets`; IPv4 remains unconnected and
  OS-routed, while Linux IPv6 caches a main-table egress resolved through
  NETLINK_ROUTE and invalidated by route/link notifications
- source-preserving IPv6 uses link-layer backends: AF_PACKET on Linux and BPF
  on macOS; these preserve the supplied datagram but bypass local IP loopback
