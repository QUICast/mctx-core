# IPv6 Multicast

IPv6 multicast send works best when source selection, interface selection, and
group scope are all explicit.

## Source vs Interface

For `mctx-core`, these are different concepts:

- `source` is the exact local sender IP that receivers will observe.
- `interface` is the local multicast egress interface.

On one machine they may be the same. Across machines they often differ. For
IPv6 SSM-style testing, the receiver filters on the exact observed sender IP,
so the source address is not optional in practice.

```rust
use mctx_core::PublicationConfig;
use std::net::Ipv6Addr;

let config = PublicationConfig::new("ff3e::8000:1234".parse::<Ipv6Addr>()?, 5000)
    .with_source_addr("fd00::10".parse::<Ipv6Addr>()?)
    .with_outgoing_interface("fd00::10".parse::<Ipv6Addr>()?);
```

If you provide only an interface address:

- `mctx-core` binds to that exact IPv6 address automatically
- it resolves that address to an interface index
- it sets `IPV6_MULTICAST_IF` from that interface index

If you provide only an interface index:

- `mctx-core` uses it for multicast egress
- it does not invent a source address for you

## SSM Groups

Use `ff3x::/32` groups for IPv6 SSM-oriented testing. The `x` nibble is the
multicast scope:

- `ff31::/16` for interface-local tests on one host
- `ff32::/16` for link-local tests on one L2 link
- `ff35::/16` for site-local tests
- `ff38::/16` for organization-local tests
- `ff3e::/16` for global scope

Prefer dynamic group IDs such as `ff31::8000:1234` or `ff3e::8000:1234`.

Do not treat `ff12::...` as an IPv6 SSM group. That is ASM.

## Practical Rules

- For `ff31::/16`, same-host tests with `::1` are a good first smoke test.
- For `ff32::/16`, send from a link-local `fe80::...` source.
- For wider scopes such as `ff35::/16`, `ff38::/16`, or `ff3e::/16`, use a
  ULA or global IPv6 source valid on that network.
- The configured source address must match the actual packet source the
  receiver sees, especially for SSM-style validation.

## Destination Scope IDs

`mctx-core` keeps the destination scope ID only for interface-local and
link-local multicast destinations.

- interface-local and link-local groups keep the interface index in the
  destination address
- wider-scope groups such as `ff35`, `ff38`, and `ff3e` are connected with
  destination scope ID `0`

That avoids Windows rejecting wider-scope destinations while still keeping
scoped groups deterministic.

## CLI Forms

Sender binaries accept:

- `--source ::1`
- `--source fe80::1234`
- `--interface ::1`
- `--interface fe80::1234`
- `--interface-index 7`

Same-host SSM-style send:

```bash
cargo run --bin mctx_send -- ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1
```

Cross-machine SSM-style send:

```bash
cargo run --bin mctx_send -- ff3e::8000:1234 5000 hello-v6 --source fd00::10
```

Link-local send:

```bash
cargo run --bin mctx_send -- ff32::8000:1234 5000 hello-v6 --source fe80::1234 --interface-index 7
```

## Platform Notes

- Windows: keep scope IDs only for `ff31` / `ff32`; wider scopes should rely on
  the bound source plus `IPV6_MULTICAST_IF`
- macOS: link-local groups such as `ff32::/16` should send from `fe80::...`
- Cross-platform: choosing only an interface index is not enough for SSM-style
  verification when the receiver filters on the exact source IP

More runnable examples live in [Demo Binaries](demo.md).
