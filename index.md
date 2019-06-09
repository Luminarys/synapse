---
layout: base
---

# synapse

Synapse is a lightweight bittorrent daemon.

Synapse is only useful when paired with RPC clients. We ship `sycli` with
synapse; use `sycli -h` to learn how to use it.

Our peer ID is `SY`.

## Features

- Websocket-based RPC
- HTTP(S) and UDP trackers
- DHT (& magnet links)
- Sequential downloading
- [etc](https://github.com/Luminarys/synapse/issues/1)

## RPC Clients

- [receptor](https://github.com/SirCmpwn/receptor): web client
- [axon](https://github.com/ParadoxSpiral/axon): curses client
- [broca](https://broca.synapse-bt.org): translates synapse RPC to transmission RPC

For your convenience, [web.synapse-bt.org](https://web.synapse-bt.org) is
running the latest version of receptor. Note: this requires you to set up
synapse independently.

## Releases
- 2019-06-09: **1.0** is now available (latest): [tar.gz](https://github.com/Luminarys/synapse/archive/1.0.tar.gz) [synapse](https://github.com/Luminarys/synapse/releases/download/1.0/synapse) [sycli](https://github.com/Luminarys/synapse/releases/download/1.0/sycli)
- 2018-04-24: **1.0-rc2** is now available (latest): [tar.gz](https://github.com/Luminarys/synapse/archive/1.0-rc2.tar.gz) [synapse](https://github.com/Luminarys/synapse/releases/download/1.0-rc2/synapse) [sycli](https://github.com/Luminarys/synapse/releases/download/1.0-rc2/sycli)
- 2018-03-02: **1.0-rc1** is now available: [tar.gz](https://github.com/Luminarys/synapse/archive/1.0-rc1.tar.gz)

## Development

Synapse development is organized on
[GitHub](https://github.com/Luminarys/synapse). To contribute, send pull
requests. To report bugs, open GitHub issues. RPC documentation is available
[here](https://github.com/Luminarys/synapse/blob/master/doc/RPC).
