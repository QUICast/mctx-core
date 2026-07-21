# Raw Packet Transmit

The optional `raw-packets` API injects complete multicast IP datagrams. It is
intended for AMT-style forwarding and other cases where receivers must observe
the original `(source, group)` tuple.

This API is separate from ordinary UDP publications:

- `Publication::send()` accepts a UDP payload and lets the kernel build IP/UDP
  headers.
- `RawPublication::send_raw()` accepts a complete IPv4 or IPv6 multicast
  datagram, including the caller-supplied IP header.
- The API never falls back to UDP when raw transmission is unavailable.

The AMT or QUIC address family carrying a datagram is the **outer transport**.
`RawPublicationConfig::family` describes only the **inner multicast IP
datagram** supplied to `send_raw()`.

## Features

Enable explicit-interface raw multicast transmission with:

```bash
cargo add mctx-core --features raw-packets
```

Enable route-selected egress in addition to the explicit API with:

```bash
cargo add mctx-core --features raw-route-egress
```

`raw-route-egress` depends on `raw-packets`. Explicit egress remains the
default, and neither feature changes the UDP API.

## API

`raw-packets` exports:

- `RawPublicationConfig`
- `RawContext`
- `RawPublication`
- `RawPublicationId`
- `RawSendReport`
- `RawIpv6EgressCapability` and `RawIpv6EgressCapabilities`
- `raw_ipv6_egress_capabilities()`

`raw-route-egress` additionally exports:

- `RawEgressMode`
- `RawRouteEgressCapability` and `RawRouteEgressCapabilities`
- `raw_route_egress_capabilities()`

## Capability Reporting

Query capabilities before choosing an IPv6 mode:

```rust
use mctx_core::{RawIpv6EgressCapability, raw_ipv6_egress_capabilities};

let capabilities = raw_ipv6_egress_capabilities();
if capabilities.explicit_interface
    == RawIpv6EgressCapability::ExplicitInterfaceFullHeader
{
    // Arbitrary source and complete-header IPv6 forwarding is available.
}
```

The IPv6 capability levels are:

- `Unsupported`: no faithful IPv6 backend is compiled for this mode.
- `LocalSourceOnly`: only a locally assigned source can be transmitted; the
  kernel may build the base header.
- `ExplicitInterfaceFullHeader`: explicit-interface transmission preserves the
  complete supplied IPv6 datagram and arbitrary source.
- `RouteSelectedFullHeader`: route-selected transmission preserves the complete
  supplied IPv6 datagram and arbitrary source.

`raw_ipv6_egress_capabilities().route_selected` reports `Unsupported` unless
the crate was built with `raw-route-egress`, even on Linux.

## Explicit Egress

Explicit mode requires `bind_addr`, `outgoing_interface`, or both:

```rust
use mctx_core::{RawContext, RawPublicationConfig};
use std::net::Ipv6Addr;

let local_egress: Ipv6Addr = "fd00::20".parse()?;
let mut context = RawContext::new();
let id = context.add_publication(
    RawPublicationConfig::ipv6()
        .with_bind_addr(local_egress)
        .with_outgoing_interface(local_egress)
        .with_loopback(false),
)?;

// This can carry a remote source. local_egress only selects the interface.
let report = context.send_raw(id, &complete_multicast_ipv6_datagram)?;
assert_eq!(report.bytes_sent, complete_multicast_ipv6_datagram.len());
# Ok::<(), mctx_core::MctxError>(())
```

For raw IPv6:

- The source observed by receivers always comes from the supplied IP header.
- `bind_addr` is a local address used to resolve and validate egress; it is not
  substituted into the datagram.
- `outgoing_interface` independently identifies the egress interface by local
  address or IPv6 interface index.
- A local header source and a remote header source use the same full-header
  link-layer backend on supported platforms.
- A configured hop-limit override may equal the supplied header value. A
  conflicting override returns `RawPacketTransmitUnsupported` rather than
  changing or ignoring the header.

The IPv6 bytes sent by the Linux and macOS full-header backends retain the
source, destination, traffic class/ECN, flow label, hop limit, extension and
fragmentation headers, and transport payload/checksum.

## Route-Selected Egress

Route-selected mode must not include explicit selectors:

```rust
use mctx_core::{RawContext, RawPublicationConfig};

let mut context = RawContext::new();
let id = context.add_publication(
    RawPublicationConfig::ipv6()
        .with_route_selected_egress()
        .with_loopback(false),
)?;

let report = context.send_raw(id, &complete_multicast_ipv6_datagram)?;
assert!(report.outgoing_interface_index.is_some());
# Ok::<(), mctx_core::MctxError>(())
```

Route-selected configuration:

- requires an explicit inner family (`ipv4()` or `ipv6()`)
- requires `bind_addr` and `outgoing_interface` to be absent
- rejects TTL/hop-limit overrides
- remains unconnected and does not pin a local source
- preserves the supplied source rather than deriving it from routing state

Linux route-selected IPv6 reads the kernel's IPv6 `main` routing table and
chooses the longest matching prefix, then the lowest route priority. The
automatic per-interface `local ff00::/8` entries are intentionally excluded;
otherwise a destination-only lookup can select an arbitrary interface.
`RTMGRP_IPV6_ROUTE` and link notifications invalidate the cached destination,
interface, and AF_PACKET socket. Route scanning and socket creation therefore
happen after invalidation or a destination change, not for every packet.

This backend follows the kernel `main` table; it does not implement policy-rule,
VRF, or non-main-table selection. Use explicit mode plus
`RawContext::replace_publication()` when those routing domains must be selected
by the application.

Interface-local and link-local IPv6 multicast (`ff31::/16` and `ff32::/16`)
cannot be route-selected because their scope requires an unambiguous
interface. They return `Ipv6ScopedMulticastRequiresInterface`; use explicit
egress instead.

Temporary route or interface failures return `MctxError::RawSendFailed` with
the original `io::Error`, native error number, and `ErrorKind`. They do not
poison the publication. A later send re-resolves routing and can recover.

## Transactional Replacement

`RawContext::replace_publication(id, config)` performs duplicate checking and
initializes the replacement before swapping it into the context. On success it
preserves `RawPublicationId`; on failure the original publication remains
untouched and usable.

Raw publications currently have no built-in metric counters. Caller metrics
keyed by the stable publication ID can therefore continue across replacement.
Destination-dependent Linux route and link-layer initialization remains lazy
because the publication config does not contain a multicast destination;
send-time failures remain recoverable.

## Platform Matrix

| Platform | Explicit IPv4 | Explicit IPv6 | Route-selected IPv4 | Route-selected IPv6 |
|----------|---------------|---------------|---------------------|---------------------|
| Linux | Raw socket | Full header, AF_PACKET/Ethernet | Supported | Full header, AF_PACKET/Ethernet |
| macOS | Raw socket | Full header, BPF/Ethernet | Supported | Unsupported |
| Windows | Raw socket | Unsupported | Unsupported | Unsupported |
| iOS | Unsupported | Unsupported | Unsupported | Unsupported |
| Android | Unsupported | Unsupported | Unsupported | Unsupported |
| Other | Unsupported | Unsupported | Unsupported | Unsupported |

### Linux

- IPv6 explicit and route-selected sends use `AF_PACKET/SOCK_DGRAM`.
- The supplied IPv6 datagram is not copied or rewritten; the kernel adds the
  Ethernet header using the selected interface and multicast destination MAC.
- Only Ethernet-like interfaces are supported. Other link types return
  `RawUnsupportedLinkType`.
- Raw transmission normally requires root or `CAP_NET_RAW`; the namespace
  roaming test also requires `CAP_NET_ADMIN`.
- Link-layer transmission does not naturally re-enter the sender's local IP
  receive path. Validate with a peer interface, another host, or packet capture.
- `loopback = true` is unsupported for full-header IPv6. `false` or an omitted
  preference is accepted.
- Route-selected IPv4 continues to use an unbound, unconnected `IP_HDRINCL`
  socket and the normal kernel route lookup.

### macOS

- Explicit IPv6 uses `/dev/bpf` with `BIOCSETIF`, verifies `DLT_EN10MB`, and
  enables `BIOCSHDRCMPLT`.
- The Ethernet destination is `33:33` plus the low 32 bits of the multicast
  group; the source MAC comes from the selected local interface.
- BPF writes the complete supplied IPv6 datagram after that Ethernet header.
- Root or suitable `/dev/bpf*` permissions are normally required.
- Non-Ethernet BPF data-link types return `RawUnsupportedLinkType`.
- BPF injection does not provide local IP multicast loopback.
- Route-selected IPv6 remains unsupported because this release does not claim
  a proven PF_ROUTE-based route-tracking implementation.

### Windows

Windows retains its existing IPv4 raw-socket backend. IPv6 raw multicast
transmission returns `RawPacketTransmitUnsupported`. Built-in raw IPv6 APIs are
not claimed to preserve an arbitrary remote source and complete header, and
the crate does not add Npcap, WinDivert, or another external driver.

iOS, Android, and other targets compile the API but report raw multicast
transmission as unsupported.

## Same-Host Behavior

Full-header IPv6 egress is link-layer injection on Linux and macOS. It is
visible on the wire but does not naturally feed a local UDP or raw-IP receiver
on the sender. This is deliberate: switching to a host-stack raw socket for a
local source would rebuild header fields and make the full-header capability
conditional.

Ordinary UDP `Publication` loopback behavior is unchanged.

## CLI Examples

The `mctx_raw_send` binary builds a complete UDP-in-IP datagram for testing.

Explicit IPv6 forwarding with an original remote source:

```bash
sudo cargo run --features raw-packets --bin mctx_raw_send -- \
  ff3e::8000:1234 5000 hello-v6 5 100 \
  --source 2001:db8:ffff::10 \
  --source-port 4000 \
  --bind fd00::20 \
  --interface fd00::20 \
  --ttl 16 \
  --no-loopback
```

Here `--source` is encoded into the inner header. `--bind` and `--interface`
must identify the actual local egress.

Linux route-selected IPv6:

```bash
sudo ip -6 route replace ff3e::8000:1234/128 dev eth0

sudo cargo run --features raw-route-egress --bin mctx_raw_send -- \
  ff3e::8000:1234 5000 hello-routed 5 100 \
  --source 2001:db8:ffff::10 \
  --source-port 4000 \
  --route-selected-egress \
  --ttl 16 \
  --no-loopback
```

`--ttl` is encoded into the generated header in route-selected mode; it is not
installed as a publication override.

Scoped IPv6 uses an explicit interface:

```bash
sudo cargo run --features raw-packets --bin mctx_raw_send -- \
  ff32::8000:1234 5000 hello-link 5 100 \
  --source fe80::1234 \
  --source-port 4000 \
  --interface-index 7 \
  --no-loopback
```

Use a peer receiver or packet capture for these IPv6 examples, not a same-host
UDP receiver.

## Reports and Errors

`RawSendReport` includes the publication ID, parsed family, source,
destination, next-header/protocol, complete IP byte count, configured local
selector, caller interface selector, and resolved interface index when known.

The raw path validates complete IPv4/IPv6 lengths, configured family, selector
families, and multicast destination before transmission. Important typed
failures include:

- `InvalidRawIpDatagram`
- `InvalidRawMulticastDestination`
- `RawInterfaceRequired`
- `Ipv6ScopedMulticastRequiresInterface`
- `RawUnsupportedLinkType`
- `RawPacketTransmitUnsupported`
- `RawSocketCreateFailed`, `RawSocketBindFailed`, and `RawSendFailed`

## Privileged Tests

Linux IPv4 route roaming:

```bash
sudo cargo test --features raw-route-egress \
  --test raw_route_egress_linux_namespace \
  one_route_selected_publication_follows_ipv4_route_changes \
  -- --ignored --exact --nocapture
```

Linux full-header IPv6 route roaming and recovery:

```bash
sudo cargo test --features raw-route-egress \
  --test raw_route_ipv6_linux_namespace \
  one_ipv6_publication_roams_and_recovers_without_rewriting_the_packet \
  -- --ignored --exact --nocapture
```

The IPv6 test uses two veth pairs, changes and removes the multicast route,
retains one publication ID, and compares the complete captured datagram byte
for byte, including traffic class, flow label, hop limit, extension header,
source/group, payload, and transport checksum.

macOS BPF smoke test:

```bash
sudo MCTX_RAW_TEST_INTERFACE_V6=fd00::20 \
  MCTX_RAW_TEST_SOURCE_V6=2001:db8:ffff::10 \
  cargo test --features raw-packets \
  raw::context::tests::macos_bpf_ipv6_full_header_send_smoke_test \
  -- --ignored --exact --nocapture
```

The macOS smoke test verifies send/report behavior. Use packet capture on the
selected interface to verify wire output.
