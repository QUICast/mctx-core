from __future__ import annotations

import asyncio
import errno
import os
import socket
import struct
import sys
import unittest

from mctx_core import AsyncPublication, Context


class _BlockedPublication:
    def __init__(self) -> None:
        self._reader, self._writer = os.pipe()
        os.set_blocking(self._writer, False)
        self.removed = False

        try:
            while True:
                os.write(self._writer, b"x" * 4096)
        except BlockingIOError:
            pass

    def fileno(self) -> int:
        return self._writer

    def send(self, _payload: bytes):
        if self.removed:
            raise LookupError("publication removed")
        raise BlockingIOError()

    def close(self) -> None:
        os.close(self._writer)
        os.close(self._reader)


class _OneBlockPublication:
    def __init__(self, result: object) -> None:
        self._reader, self._writer = os.pipe()
        self._result = result
        self.calls = 0

    def fileno(self) -> int:
        return self._writer

    def send(self, _payload: bytes):
        self.calls += 1
        if self.calls == 1:
            raise BlockingIOError()
        return self._result

    def close(self) -> None:
        os.close(self._writer)
        os.close(self._reader)


class _NoWriterLoop:
    def __init__(self, loop: asyncio.AbstractEventLoop) -> None:
        self._loop = loop

    def create_future(self):
        return self._loop.create_future()

    def add_writer(self, _fd: int, _callback) -> None:
        raise NotImplementedError

    def remove_writer(self, _fd: int) -> bool:
        raise NotImplementedError

    def is_closed(self) -> bool:
        return self._loop.is_closed()


def _multicast_test_error_is_unavailable(error: OSError) -> bool:
    """Return whether a hosted runner lacks the multicast path this test needs."""
    error_number = error.errno
    message = str(error).lower()

    if error_number == errno.ENETUNREACH or "network is unreachable" in message:
        return True

    return (
        sys.platform in {"darwin", "win32"}
        and (error_number == errno.EPIPE or "broken pipe" in message)
    )


def _skip_unavailable_multicast_test(error: OSError) -> None:
    if _multicast_test_error_is_unavailable(error):
        raise unittest.SkipTest(f"multicast unavailable on this runner: {error}")

    raise error


def _multicast_receiver(group: str) -> tuple[socket.socket, int]:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        reuse_port = getattr(socket, "SO_REUSEPORT", None)
        if reuse_port is not None:
            try:
                sock.setsockopt(socket.SOL_SOCKET, reuse_port, 1)
            except OSError:
                pass

        sock.bind(("", 0))
        port = sock.getsockname()[1]
        membership = struct.pack(
            "=4s4s",
            socket.inet_aton(group),
            socket.inet_aton("0.0.0.0"),
        )
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, membership)
        sock.settimeout(1.0)
        return sock, port
    except Exception:
        sock.close()
        raise


class BindingsTest(unittest.TestCase):
    def test_async_publication_rejects_invalid_poll_interval(self) -> None:
        for poll_interval in (0, -0.1, float("nan"), float("inf")):
            with self.subTest(poll_interval=poll_interval):
                with self.assertRaises(ValueError):
                    AsyncPublication(  # type: ignore[arg-type]
                        None,
                        poll_interval=poll_interval,
                    )

    def test_async_send_observes_removal_without_writer_readiness(self) -> None:
        async def run() -> None:
            publication = _BlockedPublication()
            task = asyncio.create_task(
                AsyncPublication(  # type: ignore[arg-type]
                    publication,
                    poll_interval=0.001,
                ).send(b"payload")
            )

            try:
                await asyncio.sleep(0.02)
                publication.removed = True
                with self.assertRaises(LookupError):
                    await asyncio.wait_for(task, timeout=0.2)
            finally:
                if not task.done():
                    task.cancel()
                    try:
                        await task
                    except asyncio.CancelledError:
                        pass
                publication.close()

        asyncio.run(run())

    def test_async_send_falls_back_when_add_writer_is_unsupported(self) -> None:
        async def run() -> None:
            expected = object()
            publication = _OneBlockPublication(expected)
            loop = _NoWriterLoop(asyncio.get_running_loop())

            try:
                result = await asyncio.wait_for(
                    AsyncPublication(  # type: ignore[arg-type]
                        publication,
                        loop=loop,  # type: ignore[arg-type]
                        poll_interval=0.001,
                    ).send(b"payload"),
                    timeout=0.2,
                )
                self.assertIs(result, expected)
                self.assertEqual(publication.calls, 2)
            finally:
                publication.close()

        asyncio.run(run())

    def test_os_errors_preserve_errno(self) -> None:
        with self.assertRaises(OSError) as caught:
            Context().add_publication(
                "239.1.2.32",
                5000,
                source="192.0.2.1",
            )

        self.assertIsInstance(caught.exception.errno, int)

    def test_multicast_runner_error_classification(self) -> None:
        self.assertTrue(
            _multicast_test_error_is_unavailable(
                OSError(errno.ENETUNREACH, "Network is unreachable")
            )
        )
        self.assertEqual(
            _multicast_test_error_is_unavailable(OSError(errno.EPIPE, "Broken pipe")),
            sys.platform in {"darwin", "win32"},
        )
        self.assertEqual(
            _multicast_test_error_is_unavailable(OSError("Broken pipe (os error 32)")),
            sys.platform in {"darwin", "win32"},
        )
        self.assertFalse(
            _multicast_test_error_is_unavailable(
                OSError(errno.ECONNREFUSED, "Connection refused")
            )
        )

    def test_context_publication_sends_packet(self) -> None:
        receiver, port = _multicast_receiver("239.1.2.30")
        with receiver:
            ctx = Context()
            try:
                publication = ctx.add_publication("239.1.2.30", port)
            except OSError as error:
                _skip_unavailable_multicast_test(error)

            payload = b"python-binding-packet"
            try:
                report = ctx.send(publication.id, payload)
            except OSError as error:
                _skip_unavailable_multicast_test(error)
            data, _sender = receiver.recvfrom(2048)

            self.assertEqual(data, payload)
            self.assertEqual(publication.group, "239.1.2.30")
            self.assertEqual(publication.dst_port, port)
            self.assertEqual(publication.family, "ipv4")
            self.assertEqual(publication.destination, ("239.1.2.30", port))
            self.assertIsNone(publication.destination_scope_id)
            self.assertEqual(report.destination, ("239.1.2.30", port))
            self.assertIsNone(report.destination_scope_id)
            self.assertIsNone(report.local_scope_id)
            self.assertEqual(report.bytes_sent, len(payload))
            self.assertEqual(publication.local_addr()[0], publication.source_addr())
            self.assertIsNone(publication.local_scope_id())
            self.assertEqual(
                publication.announce_tuple(),
                (publication.source_addr(), "239.1.2.30", port),
            )

            if hasattr(publication, "fileno"):
                self.assertGreaterEqual(publication.fileno(), 0)
            else:
                self.assertGreater(publication.socket_handle(), 0)

    def test_async_publication_send(self) -> None:
        async def run() -> None:
            receiver, port = _multicast_receiver("239.1.2.31")
            with receiver:
                ctx = Context()
                try:
                    publication = ctx.add_publication("239.1.2.31", port)
                except OSError as error:
                    _skip_unavailable_multicast_test(error)
                async_publication = AsyncPublication(publication)

                payload = b"async-python-binding-packet"
                try:
                    report = await asyncio.wait_for(
                        async_publication.send(payload),
                        timeout=1.0,
                    )
                except OSError as error:
                    _skip_unavailable_multicast_test(error)
                data, _sender = receiver.recvfrom(2048)

                self.assertEqual(data, payload)
                self.assertEqual(report.bytes_sent, len(payload))
                self.assertEqual(report.publication_id, publication.id)

        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
