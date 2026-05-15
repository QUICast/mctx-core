# Usage Guide

This guide covers the default UDP send API. Optional raw packet, metrics,
Python, and detailed IPv6 guidance live in their own docs.

## Core Flow

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();
let id = ctx.add_publication(
    PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
        .with_source_addr(Ipv4Addr::new(192, 168, 1, 10))
        .with_ttl(4),
)?;

let report = ctx.send(id, b"hello multicast")?;
println!("source {:?}", report.source_addr);
```

For IPv6:

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv6Addr;

let mut ctx = Context::new();
let id = ctx.add_publication(
    PublicationConfig::new("ff31::8000:1234".parse::<Ipv6Addr>()?, 5000)
        .with_source_addr(Ipv6Addr::LOCALHOST)
        .with_outgoing_interface(Ipv6Addr::LOCALHOST),
)?;

let report = ctx.send(id, b"hello multicast v6")?;
println!("source {:?}", report.source_addr);
```

See [IPv6 Multicast](ipv6.md) for scope, source, and interface-selection
rules.

## Useful Knobs

- `with_source_addr(...)` pins the exact local wire source
- `with_bind_addr(...)` pins both the local source address and UDP source port
- `with_source_port(...)` binds a deterministic source UDP port
- `with_outgoing_interface(...)` chooses the multicast egress interface by IP
  address
- `with_ipv6_interface_index(...)` chooses the IPv6 multicast egress interface
  by interface index
- `with_ttl(...)` controls IPv4 TTL or IPv6 multicast hop limit
- `with_loopback(...)` toggles local host loopback delivery

## Sending

Send on one publication:

```rust
let report = ctx.send(id, b"hello multicast")?;
println!("sent {} bytes", report.bytes_sent);
```

Send the same payload on every active publication:

```rust
let mut reports = Vec::new();
let count = ctx.send_all(b"hello multicast", &mut reports)?;
println!("sent on {count} publications");
```

`SendReport` includes the effective destination, the resolved local/source
address when available, and the publication ID.

## Existing Sockets

If you already manage sockets externally, use
`add_publication_with_socket(...)` or `add_publication_with_udp_socket(...)`.

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

The supplied socket is switched to non-blocking mode. `mctx-core` still
applies its publication configuration, validates family/source compatibility,
and keeps the socket connected to the multicast destination.

## Event Loop Integration

Borrow a live socket while the publication stays inside the `Context`:

```rust
let publication = ctx.get_publication(id).unwrap();
let socket = publication.socket();

#[cfg(unix)]
let raw = publication.as_raw_fd();
```

Extract ownership for handoff into another runtime:

```rust
let publication = ctx.take_publication(id).unwrap();
let parts = publication.into_parts();
let socket = parts.socket;
```

If you need the announce tuple used by wire formats that carry source/group/UDP
port explicitly:

```rust
let publication = ctx.get_publication(id).unwrap();
let (source, group, udp_port) = publication.announce_tuple()?;
```

## Optional Extensions

- Tokio: enable `tokio` and use `TokioPublication`; see [Demo Binaries](demo.md).
- Metrics: enable `metrics`; see [Metrics](metrics.md).
- Raw IP datagrams: enable `raw-packets`; see [Raw Packet Transmit](raw-packets.md).
- Python bindings: see [Python Bindings](python.md).

## Removing and Taking Ownership

```rust
ctx.remove_publication(id);
```

If you need the owned publication back:

```rust
if let Some(publication) = ctx.take_publication(id) {
    let socket = publication.into_socket();
    drop(socket);
}
```

## Multiple Publications

```rust
let mut ctx = Context::new();

let id1 = ctx.add_publication(PublicationConfig::new(group1, 5000))?;
let id2 = ctx.add_publication(PublicationConfig::new(group2, 5001))?;

ctx.send(id1, b"first payload")?;
ctx.send(id2, b"second payload")?;
```
