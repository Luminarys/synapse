---
layout: base
---

# synapse

Synapse is a bittorrent client daemon. Use synapse with caution; it is still
under development. Synapse is only useful when paired with RPC clients. We ship
`sycli` with synapse; use `sycli -h` to learn how to use it. For your
convenience, [web.synapse-bt.org](http://web.synapse-bt.org) is running the
latest version of [receptor](https://github.com/SirCmpwn/receptor), a web-based
RPC client (this requires you to set up synapse independently).

Our peer ID is `SY`.

## Features

- Websocket-based RPC
- HTTP(S) and UDP trackers
- DHT (& magnet links)
- Sequential downloading
- [etc](https://github.com/Luminarys/synapse/issues/1)

## Releases

Synapse is under development and releases are not currently available.

## Development

Synapse development is organized on
[GitHub](https://github.com/Luminarys/synapse). To contribute, send pull
requests. To report bugs, open GitHub issues. RPC documentation is available
[here](https://github.com/Luminarys/synapse/blob/master/doc/RPC).
