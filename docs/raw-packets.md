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

## Raw API Types

The feature adds a parallel transmit API:

- `RawPublicationConfig`
- `RawContext`
- `RawPublication`
- `RawSendReport`

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

## Raw Config Semantics

`RawPublicationConfig` keeps interface selection explicit:

- `family`: optional fixed IPv4 or IPv6 family, otherwise inferred per datagram
- `bind_addr`: local IP used to select and validate the egress interface
- `outgoing_interface`: explicit interface selection by IPv4/IPv6 local address
  or IPv6 interface index
- `ttl`: optional TTL or hop-limit override applied directly to the IP header
- `loopback`: optional loopback preference
- `validation_mode`: strict multicast-destination validation by default

Important distinction:

- the caller-supplied datagram header carries the source IP that receivers will
  observe
- `bind_addr` and `outgoing_interface` only select where the datagram is
  emitted

The raw path never silently falls back to ordinary UDP payload send.

## Test Harness

The repo includes a small convenience sender binary:

```bash
cargo run --features raw-packets --bin mctx_raw_send -- <group> <dst_port> <payload> [count] [interval_ms] --source <ip> [--source-port <port>] [--interface <ip>] [--interface-index <idx>] [--ttl <ttl>] [--no-loopback]
```

It builds a complete UDP-in-IP datagram and sends it through the raw API.

On macOS, that often makes it useful for validation with ordinary multicast UDP
receivers such as `mcrx_recv_meta`. On Linux and Windows, same-host UDP receive
is not a reliable raw-send validation method, because the current raw backends
do not guarantee that injected multicast datagrams are reflected back through
the local host's normal multicast receive path. For those platforms, prefer a
second machine or packet capture when validating raw transmit.

Example:

```bash
cargo run --features raw-packets --bin mctx_raw_send -- 239.255.12.34 5000 hello-raw 5 100 --source 192.168.1.20 --source-port 4000
```

The `--source` flag is required because it becomes the source IP encoded into
the raw IP header. It is also used as the default local bind address unless you
override egress with `--interface` or `--interface-index`.

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
cargo run --features raw-packets --bin mctx_raw_send -- ff3e::8000:1234 5000 hello-v6 5 100 --source <sender-ipv6> --source-port 4000 --interface <sender-ipv6-or-ifindex>
```

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

Current first-pass support:

- Linux: implemented for IPv4 and IPv6 with packet sockets
- macOS: implemented for IPv4 and IPv6 with raw IP sockets
- Windows: implemented for IPv4 with raw sockets
- other platforms: explicit unsupported error

### Observed IPv4 ASM Behavior

For the current `raw-packets` backend, the following IPv4 ASM behavior has been
observed with `mctx_raw_send` as sender and `mcrx-core` receivers:

| Sender  | macOS receiver | Windows receiver | Linux receiver |
|---------|----------------|------------------|----------------|
| macOS   | Seen           | Seen             | Seen           |
| Windows | Seen           | Seen             | Seen           |
| Linux   | Seen           | Seen             | Not seen       |

This matrix is specifically about IPv4 ASM testing so far. The only observed
gap is Linux same-host receive for packets sent from the same Linux machine.
Cross-host delivery from Linux has been observed to work.

### Linux

The current implementation uses Linux packet sockets and requires an explicit
egress interface selection.

Practical notes:

- raw packet transmit usually requires `CAP_NET_RAW` or root
- the current backend only supports Ethernet-like links
- non-Ethernet links return `MctxError::RawUnsupportedLinkType(...)`
- multicast destination MACs are derived directly from the multicast group:
  - IPv4: `01:00:5e:xx:xx:xx`
  - IPv6: `33:33:xx:xx:xx:xx`
- non-multicast raw transmit is not currently implemented
- TTL or hop-limit overrides are patched directly into the supplied IP header
- explicit raw multicast loopback control is not currently implemented on the
  Linux packet-socket backend
- packets emitted through the Linux packet-socket backend may be visible on the
  wire but not reflected back into same-host multicast UDP or raw receive
  sockets in a portable way
- in current observed IPv4 ASM testing, Linux-to-Linux same-host receive is the
  one combination that has not been observed to work, even though Linux-to-other
  hosts is visible on the wire and received by macOS and Windows peers
- for Linux raw-send validation, prefer a second host on the LAN or packet
  capture on the sender and receiver interfaces

`bind_addr` on Linux packet sockets does not rewrite the datagram source IP.
The source IP seen by receivers is the source IP already present in the caller's
datagram bytes.

### macOS

The macOS implementation currently uses raw IP sockets with header-included
mode.

Practical notes:

- raw packet transmit usually requires `root`
- IPv4 and IPv6 are both supported
- `bind_addr` is used as the exact local bind when provided
- if `outgoing_interface` is given as an IP address without `bind_addr`,
  `mctx-core` binds to that exact local address before sending
- IPv6 multicast still sets `IPV6_MULTICAST_IF` explicitly, and link-local
  binds keep their interface scope
- TTL or hop-limit overrides are patched directly into the supplied IP header

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

## Ignored Privileged Smoke Tests

The raw module also contains ignored privileged smoke tests:

Linux:

```bash
MCTX_RAW_TEST_SOURCE_V4=192.168.1.20 cargo test --features raw-packets raw::context::tests::linux_raw_ipv4_send_report_smoke_test -- --ignored --exact --nocapture
```

macOS:

```bash
MCTX_RAW_TEST_SOURCE_V4=192.168.1.20 cargo test --features raw-packets raw::context::tests::macos_raw_ipv4_send_smoke_test -- --ignored --exact --nocapture
```

Windows PowerShell:

```powershell
$env:MCTX_RAW_TEST_SOURCE_V4="192.168.1.20"
cargo test --features raw-packets raw::context::tests::windows_raw_ipv4_send_report_smoke_test -- --ignored --exact --nocapture
```

## Validation Rules

The raw path validates:

- the datagram must be a complete IPv4 or IPv6 packet
- IPv4 total length and IPv6 payload length must match the supplied buffer
- the destination must be multicast in strict mode
- the configured family, if fixed, must match the datagram family
- `bind_addr` and `outgoing_interface` must match the datagram family
- raw packet transmit requires an explicit interface selector

## AMT / SSM Note

This API is intended for AMT-style full-datagram forwarding and other cases
where source fidelity matters.

`mctx-core` does not add AMT logic itself. It simply provides a way to inject
the original datagram bytes without rewriting the IP header through a normal UDP
socket.
