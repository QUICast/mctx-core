# Usage Guide

`mctx-core` keeps the send path small:

- build a `Context`
- add one or more `PublicationConfig` values
- send payloads through the returned `PublicationId`

Basic usage:

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

Useful knobs:

- `with_interface(...)` chooses the multicast egress interface
- `with_source_addr(...)` pins the local IPv4 wire source
- `with_bind_addr(...)` pins both the local IPv4 and UDP source port
- `with_source_port(...)` binds a deterministic source UDP port
- `with_ttl(...)` controls multicast hop count
- `with_loopback(...)` toggles local host loopback delivery

If you already manage sockets externally, use `add_publication_with_socket(...)`.
