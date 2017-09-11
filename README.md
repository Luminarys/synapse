# synapse
[![Build Status](https://travis-ci.org/Luminarys/synapse.svg?branch=master)](https://travis-ci.org/Luminarys/synapse)

Synapse is a flexible and lightweight BitTorrent daemon.

## Compiling

Install dependencies:

- rustc >= 1.19
- cargo >= 0.18
- openssl
- c-ares 1.13 | build tools *

_\*Only required for synapse_

_\**Only required for sycli_

Synapse and sycli can be installed with:
```
cargo build --release --all
cargo install
cargo install --path ./sycli/
```

If you'd just like to install sycli:
```
cd sycli
cargo build --release -p sycli
cargo install --path ./sycli/
```

## Configuration

Synapse expects the configuration file to be present at `$XDG_CONFIG_DIR/synapse.toml`,
or `~/.config/synapse.toml`.
If it is not present or invalid, a default configuration will be used.
These defaults are given in `example_config.toml`.

## Development

Please see [this issue](https://github.com/Luminarys/synapse/issues/1) for details on development status.
If you're interested in developing a client for synapse, see `doc/RPC` for the current RPC spec.
