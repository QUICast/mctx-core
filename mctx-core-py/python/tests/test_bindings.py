from __future__ import annotations

import asyncio
import socket
import struct
import unittest

from mctx_core import AsyncPublication, Context


def _multicast_receiver(group: str) -> tuple[socket.socket, int]:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
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


class BindingsTest(unittest.TestCase):
    def test_context_publication_sends_packet(self) -> None:
        receiver, port = _multicast_receiver("239.1.2.30")
        with receiver:
            ctx = Context()
            publication = ctx.add_publication("239.1.2.30", port)

            payload = b"python-binding-packet"
            report = ctx.send(publication.id, payload)
            data, _sender = receiver.recvfrom(2048)

            self.assertEqual(data, payload)
            self.assertEqual(publication.group, "239.1.2.30")
            self.assertEqual(publication.dst_port, port)
            self.assertEqual(publication.family, "ipv4")
            self.assertEqual(publication.destination, ("239.1.2.30", port))
            self.assertEqual(report.destination, ("239.1.2.30", port))
            self.assertEqual(report.bytes_sent, len(payload))
            self.assertEqual(publication.local_addr()[0], publication.source_addr())
            self.assertEqual(
                publication.announce_tuple(),
                (publication.source_addr(), "239.1.2.30", port),
            )

            if hasattr(publication, "fileno"):
                self.assertGreaterEqual(publication.fileno(), 0)

    def test_async_publication_send(self) -> None:
        async def run() -> None:
            receiver, port = _multicast_receiver("239.1.2.31")
            with receiver:
                ctx = Context()
                publication = ctx.add_publication("239.1.2.31", port)
                async_publication = AsyncPublication(publication)

                payload = b"async-python-binding-packet"
                report = await asyncio.wait_for(
                    async_publication.send(payload),
                    timeout=1.0,
                )
                data, _sender = receiver.recvfrom(2048)

                self.assertEqual(data, payload)
                self.assertEqual(report.bytes_sent, len(payload))
                self.assertEqual(report.publication_id, publication.id)

        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
