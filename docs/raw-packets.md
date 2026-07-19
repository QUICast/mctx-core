# Raw Packet Transmit

`mctx-core` can optionally inject complete multicast IP datagrams instead of
just UDP payloads.

This support is gated behind the `raw-packets` Cargo feature so ordinary UDP
users do not pay for extra API surface, platform checks, or privileged socket
requirements.

## Why It Exists

The default send path is UDP-centric:

- `PublicationConfig` targets a multicast group and UDP destination port
- `Publication::send()` transmits only the UDP payload bytes
- the kernel supplies the effective socket source address and source port

That is the right shape for ordinary multicast applications. It is not enough
for AMT and RFC 7450 gateway forwarding, where the Multicast Data message
contains the original multicast IP datagram and the gateway must inject that
datagram onto the local network intact so receivers observe the original
`(source, group)` tuple.

## Feature Enablement

```bash
cargo add mctx-core --features raw-packets
```

Route-selected raw IPv4 egress is a separate additive feature:

```bash
cargo add mctx-core --features raw-route-egress
```

`raw-route-egress` depends on `raw-packets`. Without it, the raw API and its
explicit-interface requirement are unchanged.

## Raw API Types

The feature adds a parallel transmit API:

- `RawPublicationConfig`
- `RawContext`
- `RawPublication`
- `RawSendReport`

With `raw-route-egress`, it also exports:

- `RawEgressMode`
- `RawRouteEgressCapability` and `RawRouteEgressCapabilities`
- `raw_route_egress_capabilities()`

Enabling `raw-packets` does not change the behavior of `Context`,
`Publication`, `PublicationConfig`, or `SendReport`.

## Example

```rust
use mctx_core::{RawContext, RawPublicationConfig};
use std::net::Ipv4Addr;

let mut ctx = RawContext::new();
let id = ctx.add_publication(
    RawPublicationConfig::ipv4().with_bind_addr(Ipv4Addr::new(192, 168, 1, 20)),
)?;

// `ip_datagram` must contain a complete IPv4 or IPv6 datagram, including
// its original IP header.
let report = ctx.send_raw(id, &ip_datagram)?;
println!("sent {} raw bytes", report.bytes_sent);
println!("source ip: {:?}", report.source_ip);
println!("group ip: {:?}", report.destination_ip);
println!("ip protocol: {:?}", report.ip_protocol);
```

Route-selected IPv4 example:

```rust
use mctx_core::{RawContext, RawPublicationConfig};

let mut ctx = RawContext::new();
let id = ctx.add_publication(
    RawPublicationConfig::ipv4().with_route_selected_egress(),
)?;

// The original source belongs only to this complete caller-built header.
// The unbound socket follows the OS route for the multicast destination.
let report = ctx.send_raw(id, &ip_datagram)?;
assert_eq!(report.local_bind_addr, None);
assert_eq!(report.outgoing_interface_index, None);
# Ok::<(), mctx_core::MctxError>(())
```

## Raw Config Semantics

`RawPublicationConfig` uses explicit egress by default:

- `family`: optional fixed IPv4 or IPv6 family, otherwise inferred from the
  local bind or interface selector at publication creation
- `bind_addr`: local IP used to select and validate the egress interface
- `outgoing_interface`: explicit interface selection by IPv4/IPv6 local address
  or IPv6 interface index
- `ttl`: optional TTL or hop-limit override applied directly to the IP header
- `loopback`: optional loopback preference
- `validation_mode`: strict multicast-destination validation by default
- `egress_mode`: `Explicit` by default; `RouteSelected` is available only with
  `raw-route-egress`

Important distinction:

- the caller-supplied datagram header carries the source IP that receivers will
  observe
- `bind_addr` and `outgoing_interface` only select where the datagram is
  emitted
- a matching IPv6 `bind_addr` selects the host-stack raw socket, which supports
  same-host receive
- on Linux, a distinct datagram source selects packet-socket injection and
  preserves the complete IPv6 header for remote-source forwarding
- on macOS, a distinct IPv6 datagram source returns an explicit unsupported
  error because no link-layer transmit backend is currently provided

The raw path never silently falls back to ordinary UDP payload send.

## Route-Selected IPv4 Egress

`RawPublicationConfig::ipv4().with_route_selected_egress()` creates an
unbound, unconnected raw IPv4 publication on supported platforms. It does not
bind a local address, set a multicast interface, retain an interface index, or
monitor network interfaces itself. Every call uses `send_to`, leaving the OS
routing table and its normal route cache responsible for choosing egress.
Route invalidation or replacement can therefore affect a later send without
replacing the publication.

Route-selected mode:

- requires `family = IPv4`
- requires `bind_addr` and `outgoing_interface` to be absent
- rejects a configured TTL override so the supplied header TTL is retained
- allows `loopback` socket policy and normal multicast-destination validation
- returns `RawPacketTransmitUnsupported` for IPv6, Windows, and unsupported
  platforms

The socket remains usable after a failed send. Routing and temporary network
errors are returned as `MctxError::RawSendFailed` containing the original
`io::Error`, including its native error number and `ErrorKind`; callers may
retry the same publication after network state changes.

Linux transmits through an unbound `IP_HDRINCL` socket. Linux documents that it
fills the IPv4 total length and checksum and fills a zero identification field;
nonzero source, destination, TTL, DSCP/ECN, and identification values remain
caller-controlled. macOS requires its BSD host-order `ip_len`/`ip_off`
conversion and computes the IPv4 header checksum, while preserving the other
supplied fields. These are OS raw-socket semantics, not mctx rewrites.

Use runtime capability reporting before selecting this mode:

```rust
use mctx_core::{RawRouteEgressCapability, raw_route_egress_capabilities};

let capabilities = raw_route_egress_capabilities();
if capabilities.ipv4 == RawRouteEgressCapability::KernelRouteSelected {
    // Route-selected IPv4 is available in this build on this platform.
}
```

## Transactional Replacement

`RawContext::replace_publication(id, config)` validates duplicate state and
fully initializes the replacement socket before swapping it into the context.
Success preserves `RawPublicationId`; failure leaves the old publication
untouched and usable. Replacing with another publication's config still returns
`DuplicatePublication`.

Raw publications currently have no built-in metrics counters, so there are no
internal totals to reset or migrate. Caller-side metrics keyed by the stable
publication ID can continue across replacement.

## Test Harness

The repo includes a small convenience sender binary:

```bash
cargo run --features raw-packets --bin mctx_raw_send -- <group> <dst_port> <payload> [count] [interval_ms] --source <ip> [--bind <local-ip>] [--source-port <port>] [--interface <ip>] [--interface-index <idx>] [--ttl <ttl>] [--no-loopback] [--quiet]
```

It builds a complete UDP-in-IP datagram and sends it through the raw API.

That makes it useful for validation with ordinary multicast UDP receivers such
as `mcrx_recv_meta`.

For matching local IPv6 sources, Linux and macOS use raw IPv6 sockets so the
local host stack can observe same-host multicast in practical UDP-in-IP tests.
Linux switches to link-layer injection when the supplied source is remote;
macOS reports that case as unsupported. Windows raw IPv6 transmit is not
implemented.

Example:

```bash
cargo run --features raw-packets --bin mctx_raw_send -- 239.255.12.34 5000 hello-raw 5 100 --source 192.168.1.20 --source-port 4000
```

The `--source` flag is required because it becomes the source IP encoded into
the raw IP header. It is also used as the default local bind address. Use
`--bind <local-ip>` when forwarding a datagram whose original source is remote.
`--interface` or `--interface-index` independently selects egress.

Route-selected IPv4 example:

```bash
cargo run --features raw-route-egress --bin mctx_raw_send -- \
  232.1.2.3 5000 hello-routed 5 100 \
  --source 198.51.100.10 \
  --source-port 4000 \
  --route-selected-egress
```

Here `--source` is encoded only into the complete IP header. It is not used as
a local bind. `--bind`, `--interface`, and `--interface-index` conflict with
`--route-selected-egress`. `--ttl` is encoded into the generated header rather
than configured as an mctx override.

## Receiver Pairings

IPv4 ASM test with `mcrx-core`:

macOS same-host:

```bash
# receiver
cargo run --bin mcrx_recv_meta -- 239.255.12.34 5000

# sender
cargo run --features raw-packets --bin mctx_raw_send -- 239.255.12.34 5000 hello-raw 5 100 --source 192.168.1.20 --source-port 4000
```

Linux or Windows cross-host:

```bash
# receiver on another machine
cargo run --bin mcrx_recv_meta -- 239.255.12.34 5000

# sender
cargo run --features raw-packets --bin mctx_raw_send -- 239.255.12.34 5000 hello-raw 5 100 --source 192.168.1.20 --source-port 4000 --interface 192.168.1.20 --ttl 16
```

IPv4 SSM test with `mcrx-core`:

macOS same-host or cross-host:

```bash
# receiver
cargo run --bin mcrx_recv_meta -- 232.1.2.3 5000 --source 192.168.1.20

# sender
cargo run --features raw-packets --bin mctx_raw_send -- 232.1.2.3 5000 hello-ssm 5 100 --source 192.168.1.20 --source-port 4000
```

Linux or Windows should prefer a second machine or packet capture for raw-send
validation.

IPv6 same-host SSM-style smoke test on Linux or macOS:

```bash
# receiver
cargo run --bin mcrx_recv_meta -- ff31::8000:1234 5000 --source ::1 --interface ::1

# sender
cargo run --features raw-packets --bin mctx_raw_send -- ff31::8000:1234 5000 hello-v6 5 100 --source ::1 --source-port 4000 --interface ::1
```

IPv6 cross-machine SSM-style test on Linux or macOS:

```bash
# receiver
cargo run --bin mcrx_recv_meta -- ff3e::8000:1234 5000 --source <sender-ipv6> --interface <receiver-ipv6-or-ifindex>

# sender
cargo run --features raw-packets --bin mctx_raw_send -- ff3e::8000:1234 5000 hello-v6 5 100 --source <sender-ipv6> --source-port 4000 --interface <sender-ipv6>
```

Linux AMT-style forwarding with an original remote source uses a distinct local
bind and the packet-socket backend:

```bash
cargo run --features raw-packets --bin mctx_raw_send -- ff3e::8000:1234 5000 hello-v6 5 100 \
  --source <original-remote-source> \
  --bind <local-egress-ipv6> \
  --interface <local-egress-ipv6>
```

The packet is expected on the network and is not reinjected into the sender's
local receive path.

## Raw Send Report

`RawSendReport` includes:

- `publication_id`
- parsed `source_ip`
- parsed `destination_ip`
- parsed `ip_protocol`
- `bytes_sent`
- `local_bind_addr`
- `outgoing_interface`
- `outgoing_interface_index`

## Platform Support

| Platform | Explicit IPv4 | Explicit IPv6 | Route-selected IPv4 | Route-selected IPv6 |
|----------|---------------|---------------|---------------------|---------------------|
| Linux | Supported | Supported | `KernelRouteSelected` | `Unsupported` |
| macOS | Supported | Supported with local source | `KernelRouteSelected` | `Unsupported` |
| Windows | Supported | Unsupported | `Unsupported` | `Unsupported` |
| Other | Unsupported | Unsupported | `Unsupported` | `Unsupported` |

The route-selected columns describe
`raw_route_egress_capabilities()`. Windows remains unsupported because this
release does not claim faithful route-selected source/header preservation
through its constrained raw IPv4 stack.

### Observed IPv4 ASM Behavior

For the current `raw-packets` backend, the following IPv4 ASM behavior has been
observed with `mctx_raw_send` as sender and `mcrx-core` receivers:

| Sender  | macOS receiver | Windows receiver | Linux receiver |
|---------|----------------|------------------|----------------|
| macOS   | Seen           | Seen             | Seen           |
| Windows | Seen           | Seen             | Seen           |
| Linux   | Seen           | Seen             | Seen           |

This matrix is specifically about IPv4 ASM testing so far. At the moment all
three sender platforms have been observed to reach all three receivers.

### Linux

The current implementation uses raw IP sockets for IPv4 and local-source IPv6.
Remote-source IPv6 uses a packet socket so the complete supplied header can be
injected without requiring the original source to exist locally.

Practical notes:

- raw packet transmit usually requires `CAP_NET_RAW` or root
- the caller-supplied IPv4 header is transmitted with `IP_HDRINCL`
- when the datagram source matches `bind_addr`, Linux rebuilds the base IPv6
  header from the source, destination, next-header, and hop limit; this keeps
  same-host multicast delivery working
- when the datagram source differs, Linux uses packet-socket injection and
  preserves the complete IPv6 datagram for AMT/SSM forwarding
- packet-socket injection requires an Ethernet-like interface and does not feed
  the transmitted packet back through the local IP receive path
- packet-socket injection cannot enable local loopback; an explicit
  `loopback = true` returns an unsupported error
- explicit loopback control is supported for the IPv4 and IPv6 raw-socket paths
- non-multicast raw transmit is not currently implemented
- route-selected IPv4 is unbound and does not set `IP_MULTICAST_IF`

### macOS

The macOS implementation uses raw IP sockets for IPv4 and raw IPv6 sockets for
IPv6.

Practical notes:

- raw packet transmit usually requires `root`
- IPv4 and IPv6 are both supported
- `bind_addr` is used as the exact local bind when provided
- if `outgoing_interface` is given as an IP address without `bind_addr`,
  `mctx-core` binds to that exact local address before sending
- IPv4 uses `IP_HDRINCL`
- IPv6 multicast still sets `IPV6_MULTICAST_IF` explicitly, and link-local
  binds keep their interface scope
- the IPv6 datagram source must match the local bind address
- for IPv6, the kernel rebuilds the base IPv6 header on transmit instead of
  accepting a caller-supplied full IPv6 header byte-for-byte
- a distinct remote IPv6 source returns `RawPacketTransmitUnsupported` rather
  than silently sending with a rewritten source
- route-selected IPv4 is unbound and does not set `IP_BOUND_IF` or
  `IP_MULTICAST_IF`

### Windows

The Windows implementation currently supports IPv4 raw transmit only.

Practical notes:

- raw packet transmit usually requires Administrator privileges
- the implementation uses IPv4 raw sockets with `IP_HDRINCL`
- `bind_addr` is used as the exact local IPv4 bind when provided
- `IP_MULTICAST_IF` is still set explicitly for multicast egress
- same-host multicast receive is not a reliable validation method for this raw
  backend; prefer a second host or packet capture
- IPv6 full-header transmit is not currently implemented and returns
  `MctxError::RawPacketTransmitUnsupported(...)`
- route-selected raw egress is not claimed and returns the same typed
  unsupported error

## Ignored Privileged Smoke Tests

The raw module also contains ignored privileged smoke tests:

Linux:

```bash
MCTX_RAW_TEST_SOURCE_V4=192.168.1.20 cargo test --features raw-packets raw::context::tests::linux_raw_ipv4_send_report_smoke_test -- --ignored --exact --nocapture
cargo test --features raw-packets raw::context::tests::linux_raw_ipv6_same_host_smoke_test -- --ignored --exact --nocapture
```

macOS:

```bash
MCTX_RAW_TEST_SOURCE_V4=192.168.1.20 cargo test --features raw-packets raw::context::tests::macos_raw_ipv4_send_smoke_test -- --ignored --exact --nocapture
cargo test --features raw-packets raw::context::tests::macos_raw_ipv6_same_host_smoke_test -- --ignored --exact --nocapture
```

Windows PowerShell:

```powershell
$env:MCTX_RAW_TEST_SOURCE_V4="192.168.1.20"
cargo test --features raw-packets raw::context::tests::windows_raw_ipv4_send_report_smoke_test -- --ignored --exact --nocapture
```

Linux route-change namespace test:

```bash
sudo cargo test --features raw-route-egress \
  --test raw_route_egress_linux_namespace \
  -- --ignored --exact --nocapture
```

This requires `iproute2`, `CAP_NET_ADMIN`, and `CAP_NET_RAW`. It uses two veth
paths, starts with an unreachable multicast route, and then moves the route
between interfaces while retaining one publication ID.

## Validation Rules

The raw path validates:

- the datagram must be a complete IPv4 or IPv6 packet
- IPv4 total length and IPv6 payload length must match the supplied buffer
- the destination must be multicast in strict mode
- the configured family, if fixed, must match the datagram family
- `bind_addr` and `outgoing_interface` must match the datagram family
- explicit mode requires a local bind or interface selector
- route-selected mode requires explicit IPv4 family and no bind/interface
  selector or TTL override

## AMT / SSM Note

This API is intended for AMT-style full-datagram forwarding and other cases
where source fidelity matters.

`mctx-core` does not add AMT logic itself. Linux can inject a remote-source IPv6
datagram through its packet-socket path with complete-header fidelity. The
Linux and macOS host-stack IPv6 paths are intended for matching local sources;
the kernel rebuilds the base IPv6 header there. macOS and Windows return an
explicit unsupported error where remote-source IPv6 fidelity is unavailable.
