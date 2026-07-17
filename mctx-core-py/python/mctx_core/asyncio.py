from __future__ import annotations

import asyncio
import math
import os

from ._mctx_core import Publication, SendReport


def _validate_poll_interval(poll_interval: float) -> float:
    if not math.isfinite(poll_interval) or poll_interval <= 0:
        raise ValueError("poll_interval must be a positive finite number")
    return poll_interval


def _duplicate_writer_fd(publication: Publication) -> int | None:
    try:
        return os.dup(publication.fileno())
    except OSError:
        return None


def _close_writer_fd(loop: asyncio.AbstractEventLoop, writer_fd: int) -> None:
    try:
        if not loop.is_closed():
            loop.remove_writer(writer_fd)
    except NotImplementedError:
        pass
    except (RuntimeError, ValueError):
        if not loop.is_closed():
            raise
    finally:
        os.close(writer_fd)


class AsyncPublication:
    def __init__(
        self,
        publication: Publication,
        *,
        loop: asyncio.AbstractEventLoop | None = None,
        poll_interval: float = 0.001,
    ) -> None:
        self.publication = publication
        self._loop = loop
        self._poll_interval = _validate_poll_interval(poll_interval)

    async def send(self, payload: bytes) -> SendReport:
        running_loop = self._loop or asyncio.get_running_loop()
        selector_available = hasattr(running_loop, "add_writer") and hasattr(
            self.publication, "fileno"
        )

        while True:
            try:
                return self.publication.send(payload)
            except BlockingIOError:
                if not selector_available:
                    await asyncio.sleep(self._poll_interval)
                    continue

                writer_fd = _duplicate_writer_fd(self.publication)
                if writer_fd is None:
                    selector_available = False
                    await asyncio.sleep(self._poll_interval)
                    continue

                future = running_loop.create_future()

                def on_writable() -> None:
                    if not future.done():
                        future.set_result(None)

                try:
                    running_loop.add_writer(writer_fd, on_writable)
                except (AttributeError, NotImplementedError):
                    selector_available = False
                    _close_writer_fd(running_loop, writer_fd)
                    await asyncio.sleep(self._poll_interval)
                    continue

                try:
                    await asyncio.wait_for(future, timeout=self._poll_interval)
                except asyncio.TimeoutError:
                    # The duplicate remains valid after Publication.remove()
                    # closes the original socket. Retry periodically so the
                    # resulting LookupError is observed without readiness.
                    pass
                finally:
                    _close_writer_fd(running_loop, writer_fd)
