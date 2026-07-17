# mctx-core-py

`mctx-core-py` is the Python binding crate for
[`mctx-core`](https://github.com/QUICast/mctx-core).

It provides:

- `Context` for multicast sender context and publication management
- `Publication` for publication-level send operations and socket details
- `SendReport` for per-send results
- `AsyncPublication` for await-style `asyncio` integration

The bindings currently expose the normal UDP publication API. Metrics, raw
packet/raw-IP transmit, caller-provided sockets, and bulk `send_all()` remain
Rust-only APIs for now.

## Build

Install from the repository root:

```bash
pip install ./mctx-core-py
```

For local development:

```bash
cd mctx-core-py
maturin develop
```

## Example

```python
import asyncio

from mctx_core import AsyncPublication, Context

ctx = Context()
publication = ctx.add_publication(
    "ff31::8000:1234",
    5000,
    source="::1",
    interface="::1",
)

async def main() -> None:
    report = publication.send(b"hello multicast")
    print(report.source_addr, report.destination, report.bytes_sent)

    async_publication = AsyncPublication(publication)
    report = await async_publication.send(b"hello again")
    print(report.bytes_sent)

asyncio.run(main())
```

`AsyncPublication` retries a non-blocking send when the socket reports
`BlockingIOError`. On selector-based event loops it registers a duplicated
publication descriptor and periodically rechecks publication lifetime. This
prevents a removed publication from leaving a stale descriptor registration or
hung send. On loops where writer readiness is unavailable, such as the default
Windows asyncio loop, it falls back to a thin async polling layer over the same
non-blocking send call.

IPv6 `destination_scope_id` and `local_scope_id` values are exposed separately
so the existing two-item address tuples remain stable.

Generated extension modules and wheels are intentionally ignored by Git. Build
a fresh wheel before distribution rather than reusing an artifact already in a
working tree.
