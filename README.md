# mctx-core

`mctx-core` is a runtime-agnostic and portable IPv4 multicast sender library.

It is built for applications and integrations that want a small multicast send
core with explicit socket ownership, a non-blocking send path, and optional
async or metrics add-ons.

## Highlights

- IPv4 multicast send support with configurable interface, loopback, and TTL
- Non-blocking send API
- Immediate-ready publications with caller-owned context and socket extraction
- Caller-provided socket support
- Event-loop friendly socket borrowing and extraction APIs
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

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();

let config = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
    .with_ttl(8);
let id = ctx.add_publication(config)?;

let report = ctx.send(id, b"hello multicast")?;
println!("sent {} bytes to {}", report.bytes_sent, report.destination);
```

## Existing Sockets

Use `add_publication_with_socket()` when you need to create or bind the socket
yourself:

```rust
use mctx_core::{Context, PublicationConfig};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::Ipv4Addr;

let mut ctx = Context::new();
let config = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
    .with_source_port(5001);

let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
let id = ctx.add_publication_with_socket(config, socket)?;
ctx.send(id, b"hello from an existing socket")?;
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
cargo run --features tokio --bin mctx_tokio_send -- 239.1.2.3 5000 hello
```

## Optional Metrics

If you need send counters, enable the `metrics` feature and query snapshots:

```rust
let publication = ctx.get_publication(id).unwrap();
let metrics = publication.metrics_snapshot();

println!("packets sent: {}", metrics.packets_sent);
println!("bytes sent: {}", metrics.bytes_sent);
```

## Demo Binaries

Basic sender:

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello
```

Burst sender with a fixed count:

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello 1000
```

Tokio sender:

```bash
cargo run --features tokio --bin mctx_tokio_send -- 239.1.2.3 5000 hello
```

## Documentation

- [Usage Guide](docs/usage.md)
- [Architecture](docs/architecture.md)
- [Demo Binaries](docs/demo.md)
- [Metrics](docs/metrics.md)
- [Design Decisions](docs/design-decisions.md)

## Platform Support

| OS      | ASM send | Notes              |
|---------|----------|--------------------|
| macOS   | ✅        | Intended support   |
| Linux   | ✅        | Intended support   |
| Windows | ✅        | Intended support   |

## License

BSD 2-Clause
