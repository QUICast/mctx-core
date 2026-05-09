# Design Decisions

## No Join or Leave Lifecycle

The receiver crate needs explicit group membership management. The sender side
does not, so `mctx-core` collapses that complexity into immediate-ready
publications.

## Separate IPv4 and IPv6 Send Paths

`mctx-core` keeps the IPv4 and IPv6 socket configuration branches separate.

That avoids hiding important IPv6 behavior behind a single generic multicast
send implementation and keeps platform-specific fixes easy to audit.

## Connected UDP Sockets

Each publication connects its UDP socket to a single multicast destination.
That keeps repeated sends simple and avoids rebuilding the same destination
address on every call.

## Explicit Source Address vs Outgoing Interface

The sender source address and the outgoing interface are treated as distinct
configuration choices.

For IPv4 this preserves the expected bind-vs-`IP_MULTICAST_IF` split.

For IPv6:

- binding the exact source matters for SSM-style receiver verification
- selecting the outgoing interface still matters independently
- an IPv6 address used as the sender selector also resolves to an interface
  index and sets `IPV6_MULTICAST_IF`

## IPv6 Destination Scope IDs

`mctx-core` only keeps a destination scope ID for interface-local and
link-local IPv6 multicast groups.

For wider scopes such as `ff35::/16`, `ff38::/16`, and `ff3e::/16`, the
destination scope ID is cleared and the bound source plus multicast interface
socket option drive the route selection instead. This matches the practical
Windows behavior observed in `mcrx-core`.

## Optional Add-Ons

The base crate keeps only the essentials:

- `socket2`
- `thiserror`
- lightweight OS support for IPv6 interface resolution

Async support and metrics are gated behind features so lightweight embeddings do
not pay for them by default.
