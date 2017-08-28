# synapse
[![Build Status](https://travis-ci.org/Luminarys/synapse.svg?branch=master)](https://travis-ci.org/Luminarys/synapse)

Synapse is a flexible and lightweight BitTorrent daemon.

## Compiling

Install dependencies:

- rustc (stable 1.19)
- cargo (stable 0.20)
- c-ares *
- openssl **

_\*Only required for synapse_

_\**Only required for sycli_

Synapse and sycli can be installed with:
```
cargo build --all --release
cargo install
```

If you'd just like to install one or the other:
```
cargo build -p sycli --release
cargo install
```
or vice versa.

## Configuration

Synapse expects the configuration file to be passed as `argv[1]`.
If it is not present or invalid, a default configuration will be used.
These defaults are given in `example_config.toml`.

## Development

Please see [this issue](https://github.com/Luminarys/synapse/issues/1) for details on development status.
If you're interested in developing a client for synapse, see `doc/RPC` for the current RPC spec.
