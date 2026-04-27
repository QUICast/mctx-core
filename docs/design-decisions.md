# Design Decisions

## No Join or Leave Lifecycle

The receiver crate needs explicit group membership management. The sender side
does not, so `mctx-core` collapses that complexity into immediate-ready
publications.

## Connected UDP Sockets

Each publication connects its UDP socket to a single multicast destination.
That keeps repeated sends simple and avoids rebuilding the same destination
address on every call.

When a caller requests `with_source_addr(...)` or `with_bind_addr(...)`,
`mctx-core` binds that exact local IPv4 before connecting so the resulting wire
source is deterministic for announce-style protocols.

## Optional Add-Ons

The base crate keeps only the essentials:

- `socket2`
- `thiserror`

Async support and metrics are gated behind features so lightweight embeddings do
not pay for them by default.
