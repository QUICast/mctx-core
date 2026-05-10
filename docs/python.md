# Python Bindings

The Python bindings live in the sibling workspace crate `mctx-core-py`.

## Build

From the repository root:

```bash
pip install ./mctx-core-py
```

For local development:

```bash
cd mctx-core-py
maturin develop
```

## Binding Shape

The Python API is intentionally centered on the multicast sender use case
rather than on a direct transliteration of the Rust ownership model.

Core objects:

- `Context`
- `Publication`
- `SendReport`

Async helper:

- `AsyncPublication`

That gives Python callers the four pieces that tend to matter most:

1. create and manage a multicast sender context
2. configure publications with explicit source and outgoing-interface control
3. send packets and inspect the resolved source/destination details
4. integrate a non-blocking sender into `asyncio`

## Basic Example

```python
from mctx_core import Context

ctx = Context()
publication = ctx.add_publication(
    "239.1.2.3",
    5000,
    source="192.168.1.20",
    interface="192.168.1.20",
)

report = publication.send(b"hello multicast")
print(report.source_addr, report.destination, report.bytes_sent)
```

For IPv6 same-host SSM-style testing:

```python
publication = ctx.add_publication(
    "ff31::8000:1234",
    5000,
    source="::1",
    interface="::1",
)

report = publication.send(b"hello ipv6 multicast")
print(publication.announce_tuple())
```

## Explicit Source vs Outgoing Interface

The binding keeps the same distinction as the Rust crate:

- `source`: exact local sender IP to bind before transmitting
- `interface`: outgoing multicast interface selected by local IP address
- `interface_index`: outgoing IPv6 multicast interface selected by interface index

If you provide an IPv6 `source`, `mctx-core` binds that exact address and uses
it for the effective sender IP. If you provide an IPv6 `interface` address and
do not provide a `source`, `mctx-core` binds to that exact interface address.

For IPv6 SSM-oriented testing, the receiver's source filter keys off the exact
observed sender IP, so `source=` is usually the most important knob.

## Asyncio

For direct await-style use:

```python
from mctx_core import AsyncPublication

async_publication = AsyncPublication(publication)
report = await async_publication.send(b"hello from asyncio")
```

### Event Loop Behavior

On selector-based loops, `AsyncPublication` uses `loop.add_writer()` and the
publication file descriptor directly after a non-blocking send reports
`BlockingIOError`.

On platforms or loops where `add_writer()` is not available, such as the
default Windows asyncio loop, it falls back to a thin async polling loop over
the same `Publication.send()` call.

There is intentionally no callback-style `add_writer()` helper here. For UDP
sender sockets, write readiness is usually level-triggered and effectively
always-on, which makes a long-lived callback registration noisy and
surprising.

## Notes

- `mctx-core` remains a pure Rust crate with no PyO3 or `cdylib` packaging.
- `mctx-core-py` depends on `mctx-core` by path inside the same workspace.
- The Python bindings are layered on top of the same non-blocking send path as
  the Rust API.
- `Publication.source_addr()` returns the effective local sender IP selected by
  the socket, while `Publication.configured_source_addr` exposes the explicit
  configured bind address when one was requested.
