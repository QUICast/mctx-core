# mctx-core

`mctx-core` is a runtime-agnostic multicast sender library for IPv4 and IPv6
ASM/SSM-style traffic.

The default API focuses on lightweight UDP multicast send with explicit socket
ownership, deterministic source/interface control, and non-blocking operation.
Optional features add Tokio integration, metrics, multicast raw-IP forwarding,
route-selected raw egress, generic raw-IP control transmit, and Python bindings
without changing the default UDP send path.

## Highlights

- IPv4 and IPv6 multicast send support
- Explicit separation between sender source address and outgoing interface
- Exact IPv4 or IPv6 local bind control for announce-style senders
- Predictable IPv6 destination scope handling for `ff31` / `ff32` vs `ff35` /
  `ff38` / `ff3e`
- Non-blocking send API with caller-owned context and socket extraction
- Caller-provided socket support
- Optional `tokio`, `metrics`, `raw-packets`, `raw-route-egress`, and `raw-ip`
  features
- Optional Python bindings in the sibling `mctx-core-py` crate

## Install

```bash
cargo add mctx-core
```

The minimum supported Rust version is 1.88.

Optional feature examples:

```bash
cargo add mctx-core --features tokio
cargo add mctx-core --features metrics
cargo add mctx-core --features raw-packets
cargo add mctx-core --features raw-route-egress
cargo add mctx-core --features raw-ip
```

Python bindings are covered in the [Python Bindings](docs/python.md) guide; the
binding crate lives in the repository's `mctx-core-py` workspace directory.
The repository does not currently ship a stable C ABI wrapper.

## Quick Start

```rust
use mctx_core::{Context, PublicationConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();
let id = ctx.add_publication(
    PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000)
        .with_source_addr(Ipv4Addr::new(192, 168, 1, 10))
        .with_ttl(8),
)?;

let report = ctx.send(id, b"hello multicast")?;
println!("sent {} bytes to {}", report.bytes_sent, report.destination);
println!("wire source {:?}", report.source_addr);
```

For IPv6 examples, source/interface rules, and CLI commands, see
[IPv6 Multicast](docs/ipv6.md) and [Demo Binaries](docs/demo.md).

## Feature Map

- `tokio`: async send wrapper for extracted publications.
- `metrics`: snapshots, deltas, samplers, and Heimdall-style JSONL helpers.
- `raw-packets`: complete multicast IP datagram transmit for AMT-style use
  cases.
- `raw-route-egress`: adds route-selected IPv4 egress on Linux/macOS and
  source-preserving IPv6 roaming on Linux to `raw-packets`.
- `raw-ip`: complete unicast or multicast IP datagram transmit for caller-built
  control traffic such as ICMP Packet Too Big.
- `mctx-core-py`: sibling workspace crate with Python and asyncio bindings.

## Documentation

- [Usage Guide](docs/usage.md): core Rust sender API flow.
- [IPv6 Multicast](docs/ipv6.md): source vs interface, scopes, and SSM group rules.
- [Raw Packet Transmit](docs/raw-packets.md): `raw-packets` API and platform limits.
- [Raw IP Control Transmit](docs/raw-ip.md): `raw-ip` API and platform limits.
- [Demo Binaries](docs/demo.md): sender CLI commands and metrics examples.
- [Metrics](docs/metrics.md): snapshot, delta, and JSONL semantics.
- [Python Bindings](docs/python.md): Python API and asyncio helper.
- [Architecture](docs/architecture.md): main types and module layout.
- [Design Decisions](docs/design-decisions.md): why the API is shaped this way.

## Platform Support

| OS      | IPv4 send | IPv6 ASM send | IPv6 SSM-style send | Notes |
|---------|-----------|---------------|---------------------|-------|
| macOS   | ✅         | ✅             | ✅                   | `ff32::/16` should use a `fe80::` source |
| Linux   | ✅         | ✅             | ✅                   | intended support |
| Windows | ✅         | ✅             | ✅                   | keep scope ID only for `ff31` / `ff32` |

The default UDP send path supports IPv4 and IPv6 multicast on the same
platforms.

Raw multicast IP datagram transmit is available behind `raw-packets`. Linux
uses AF_PACKET and macOS uses BPF for explicit, source-preserving full-header
IPv6 egress on Ethernet-like interfaces. With `raw-route-egress`, Linux also
tracks IPv6 main-table route/link changes while retaining a publication ID.
macOS route-selected IPv6 and all Windows IPv6 raw transmit remain explicitly
unsupported. Link-layer IPv6 injection is visible on the wire but does not
naturally feed the sender's local IP receive path. See [Raw Packet
Transmit](docs/raw-packets.md) for exact capabilities and privilege rules.

Generic raw-IP control transmit is available behind the independent `raw-ip`
feature. It accepts a complete caller-supplied unicast or multicast datagram.
Linux and macOS support IPv4 `IP_HDRINCL`-style transmit and an explicit
kernel-built IPv6 base-header path. Windows supports IPv4 only; raw IPv6
returns an explicit unsupported error. See [Raw IP Control Transmit](docs/raw-ip.md)
for source-preservation and privilege requirements.

## License

BSD 2-Clause
