from __future__ import annotations

import asyncio

from ._mctx_core import Publication, SendReport


class AsyncPublication:
    def __init__(
        self,
        publication: Publication,
        *,
        loop: asyncio.AbstractEventLoop | None = None,
        poll_interval: float = 0.01,
    ) -> None:
        self.publication = publication
        self._loop = loop
        self._poll_interval = poll_interval

    async def send(self, payload: bytes) -> SendReport:
        running_loop = self._loop or asyncio.get_running_loop()

        while True:
            try:
                return self.publication.send(payload)
            except BlockingIOError:
                if hasattr(running_loop, "add_writer") and hasattr(self.publication, "fileno"):
                    future = running_loop.create_future()
                    fd = self.publication.fileno()

                    def on_writable() -> None:
                        if not future.done():
                            future.set_result(None)

                    running_loop.add_writer(fd, on_writable)
                    try:
                        await future
                    finally:
                        running_loop.remove_writer(fd)
                else:
                    await asyncio.sleep(self._poll_interval)
