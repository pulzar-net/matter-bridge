"""Two Bumble virtual Bluetooth controllers on one simulated link, each
reachable as H4 over TCP: one for the controller under test (the Matter
commissioner), one for the device being commissioned.

    python link.py PORT_A PORT_B

Prints "ready" once both listen, then runs until killed.
"""

import asyncio
import sys

from bumble.controller import Controller
from bumble.link import LocalLink
from bumble.transport import open_transport


async def main():
    link = LocalLink()
    transports = []
    for i, port in enumerate(sys.argv[1:3]):
        transport = await open_transport(f"tcp-server:127.0.0.1:{port}")
        Controller(
            f"C{i}",
            host_source=transport.source,
            host_sink=transport.sink,
            link=link,
            public_address=f"F0:00:00:00:00:0{i + 1}",
        )
        transports.append(transport)
    print("ready", flush=True)
    await asyncio.Event().wait()


asyncio.run(main())
