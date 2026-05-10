# Usage Guide

`mctx-core` keeps the send path small:

- build a `Context`
- add one or more `PublicationConfig` values
- send payloads through the returned `PublicationId`

## Basic IPv4 Usage

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

## Basic IPv6 Usage

Same-host IPv6 SSM-style testing:

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

Wider-scope IPv6 send:

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv6Addr;

let mut ctx = Context::new();
let id = ctx.add_publication(
    PublicationConfig::new("ff3e::8000:1234".parse::<Ipv6Addr>()?, 5000)
        .with_source_addr("fd00::10".parse::<Ipv6Addr>()?),
)?;

let report = ctx.send(id, b"hello multicast v6")?;
println!("source {:?}", report.source_addr);
```

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

## Source Address vs Outgoing Interface

These are different settings:

- source address: which local IP the sender binds before transmitting
- outgoing interface: which interface multicast egress uses

For IPv6, `mctx-core` makes the relationship explicit:

- if you provide an IPv6 source address, `mctx-core` binds that exact address
  and resolves its interface index for `IPV6_MULTICAST_IF`
- if you provide an IPv6 outgoing interface address without a source address,
  `mctx-core` binds to that exact address automatically
- if you provide only an IPv6 interface index, `mctx-core` uses it for
  multicast egress but does not invent a source address for you

This matters for IPv6 SSM-style tests because the receiver's source filter uses
the exact observed sender IP.

## IPv6 Group Guidance

- use `ff3x::/32` groups for IPv6 SSM-oriented testing
- `ff31::/16` is interface-local and is the easiest choice for same-host tests
- `ff32::/16` is link-local and should be paired with a `fe80::...` source
- `ff35::/16` is site-local
- `ff38::/16` is organization-local
- `ff3e::/16` is global scope
- do not treat `ff12::...` as an IPv6 SSM group

`mctx-core` keeps the destination scope ID only for interface-local or
link-local multicast destinations. Wider-scope groups such as `ff35`,
`ff38`, and `ff3e` are connected with destination scope ID `0`.

## Platform Notes

- Windows: do not stuff the interface index into wider-scope IPv6 destination
  addresses; use the bound source plus `IPV6_MULTICAST_IF`
- macOS: link-local groups such as `ff32::/16` should send from `fe80::...`
- Cross-platform: choosing only an interface index is not enough for IPv6
  SSM-style verification when the receiver filters on the exact source IP

## Existing Sockets

If you already manage sockets externally, use
`add_publication_with_socket(...)` or `add_publication_with_udp_socket(...)`.

## Python Bindings

If you want to drive the sender from Python, build the sibling `mctx-core-py`
crate. It exposes `Context`, `Publication`, `SendReport`, and a small
`AsyncPublication` helper.

Build and packaging details live in [Python Bindings](python.md).
