# mctx-core

`mctx-core` is a runtime-agnostic and portable IPv4 and IPv6 multicast sender
library.

It is built for applications that want a small multicast send core with
explicit socket ownership, a non-blocking send path, and optional async or
metrics add-ons.

## Highlights

- IPv4 multicast send support
- IPv6 multicast send support for ASM and SSM-oriented testing
- Explicit separation between sender source address and outgoing interface
- Exact IPv4 or IPv6 local bind control for announce-style senders
- Predictable IPv6 destination scope handling for `ff31` / `ff32` vs `ff35` /
  `ff38` / `ff3e`
- Non-blocking send API
- Immediate-ready publications with caller-owned context and socket extraction
- Caller-provided socket support
- Optional Tokio adapter via the `tokio` feature
- Optional send metrics via the `metrics` feature

## Install

```bash
cargo add mctx-core
```

With the optional Tokio adapter:

```bash
cargo add mctx-core --features tokio
```

With optional metrics:

```bash
cargo add mctx-core --features metrics
```

## Quick Start

IPv4:

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();

let config = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
    .with_source_addr(Ipv4Addr::new(192, 168, 1, 10))
    .with_ttl(8);
let id = ctx.add_publication(config)?;

let report = ctx.send(id, b"hello multicast")?;
println!("sent {} bytes to {}", report.bytes_sent, report.destination);
println!("wire source: {:?}", report.source_addr);
```

IPv6 same-host SSM-style send:

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv6Addr;

let mut ctx = Context::new();

let config = PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>()?, 5000)
    .with_source_addr(Ipv6Addr::LOCALHOST)
    .with_outgoing_interface(Ipv6Addr::LOCALHOST);
let id = ctx.add_publication(config)?;

let report = ctx.send(id, b"hello multicast v6")?;
println!("sent {} bytes to {}", report.bytes_sent, report.destination);
println!("wire source: {:?}", report.source_addr);
```

## Source Address vs Outgoing Interface

`mctx-core` keeps these concepts distinct:

- source address: the exact local IP the sender binds before transmitting
- outgoing interface: the interface used for multicast egress

For IPv4, these map to the usual bind-address and `IP_MULTICAST_IF` behavior.

For IPv6, the distinction matters much more:

- if you set `with_source_addr(...)` to an IPv6 address, `mctx-core` binds that
  exact local IPv6 address
- it also resolves that address to an interface index and sets
  `IPV6_MULTICAST_IF`
- if you set `with_outgoing_interface(...)` to an IPv6 address and do not set
  `with_source_addr(...)`, `mctx-core` auto-binds to that exact IPv6 address
- if you use `with_ipv6_interface_index(...)`, `mctx-core` uses that interface
  for multicast egress without inventing a source address for you

This keeps IPv6 SSM-style sender behavior predictable across macOS, Linux, and
Windows.

## IPv6 SSM Notes

Receiver-side source filtering keys off the exact sender IP, so the sender's
bound source address matters.

Group rules:

- valid IPv6 SSM groups are in `ff3x::/32`
- `ff31::/16` is interface-local and works well for same-host tests
- `ff32::/16` is link-local and only works on the local L2 link
- `ff35::/16` is site-local
- `ff38::/16` is organization-local
- `ff3e::/16` is global scope
- do not treat `ff12::...` as an IPv6 SSM group

Practical rules:

- for `ff32::/16`, send from a link-local `fe80::...` source
- wider-scope groups such as `ff35::...`, `ff38::...`, and `ff3e::...` should
  use a routable ULA or global source valid on that network
- destination scope IDs are only kept for interface-local and link-local
  groups; they are cleared for wider scopes so Windows does not reject them

## Existing Sockets

Use `add_publication_with_socket()` when you need to create or bind the socket
yourself:

```rust
use mctx_core::{Context, PublicationConfig};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::Ipv6Addr;

let mut ctx = Context::new();
let config = PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>()?, 5000)
    .with_source_addr(Ipv6Addr::LOCALHOST)
    .with_outgoing_interface(Ipv6Addr::LOCALHOST);

let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
let id = ctx.add_publication_with_socket(config, socket)?;
ctx.send(id, b"hello from an existing socket")?;
```

Or hand in a `std::net::UdpSocket` directly:

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::{Ipv4Addr, UdpSocket};

let mut ctx = Context::new();
let config = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000);
let socket = UdpSocket::bind("0.0.0.0:0")?;

let id = ctx.add_publication_with_udp_socket(config, socket)?;
ctx.send(id, b"hello from std::net::UdpSocket")?;
```

## Event Loop Integration

Borrow the live socket from a publication:

```rust
let publication = ctx.get_publication(id).unwrap();
let socket = publication.socket();

#[cfg(unix)]
let raw = publication.as_raw_fd();
```

Or extract the publication and move it into another loop or runtime:

```rust
let publication = ctx.take_publication(id).unwrap();
let parts = publication.into_parts();
let socket = parts.socket;
```

If you need the exact announce tuple used by the wire format:

```rust
let publication = ctx.get_publication(id).unwrap();
let (source, group, udp_port) = publication.announce_tuple()?;
```

## Tokio Integration

With the `tokio` feature enabled, you can wrap an extracted publication and
send asynchronously:

```rust
use mctx_core::TokioPublication;

let publication = ctx.take_publication(id).unwrap();
let publication = TokioPublication::new(publication)?;
publication.send(b"hello from tokio").await?;
```

Run the Tokio example with:

```bash
cargo run --features tokio --bin mctx_tokio_send -- ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1
```

## Demo Binaries

Basic IPv4 send:

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello
```

IPv6 same-host SSM-style send:

```bash
cargo run --bin mctx_send -- ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1
```

IPv6 cross-machine SSM-style send on the same network:

```bash
cargo run --bin mctx_send -- ff3e::8000:1234 5000 hello-v6 --source fd00::10
```

IPv6 link-local send:

```bash
cargo run --bin mctx_send -- ff32::8000:1234 5000 hello-v6 --source fe80::1234 --interface-index 7
```

## Optional Metrics

If you need send counters, enable the `metrics` feature and query snapshots:

```rust
let publication = ctx.get_publication(id).unwrap();
let metrics = publication.metrics_snapshot();

println!("packets sent: {}", metrics.packets_sent);
println!("bytes sent: {}", metrics.bytes_sent);
```

## Documentation

- [Usage Guide](docs/usage.md)
- [Architecture](docs/architecture.md)
- [Demo Binaries](docs/demo.md)
- [Metrics](docs/metrics.md)
- [Design Decisions](docs/design-decisions.md)

## Platform Support

| OS      | IPv4 send | IPv6 ASM send | IPv6 SSM-style send | Notes |
|---------|-----------|---------------|---------------------|-------|
| macOS   | ✅         | ✅             | ✅                   | `ff32::/16` should use a `fe80::` source |
| Linux   | ✅         | ✅             | ✅                   | intended support |
| Windows | ✅         | ✅             | ✅                   | keep scope ID only for `ff31` / `ff32` |

## License

BSD 2-Clause
