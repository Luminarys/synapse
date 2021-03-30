# synapse
[![Build Status](https://travis-ci.org/Luminarys/synapse.svg?branch=master)](https://travis-ci.org/Luminarys/synapse)

Synapse is a flexible and fast BitTorrent daemon.

It currently supports most systems which implement epoll or kqueue, with a focus on 64-bit linux servers.

## About
* Event based RPC using websockets
* HTTP downloads and TLS for easy server usage
* Can be used via web client with minimal setup - see [receptor](https://web.synapse-bt.org)
* See [this wiki page](https://github.com/Luminarys/synapse/wiki/Feature-Stability) for an overview of stability

## Installation
### Package
A list of packages can be found on [this wiki page](https://github.com/Luminarys/synapse/wiki/Third-party-packages).

### Compiling
Install dependencies:

- rustc >= 1.39.0
- cargo >= 0.18
- gcc | clang

Synapse and sycli can be installed with:
```
cargo build --release --all
cargo install
cargo install --path ./sycli/
```

If you'd just like to install sycli:
```
cargo build --release -p sycli
cargo install --path ./sycli/
```

## Configuration
Synapse expects its configuration file to be present at `$XDG_CONFIG_DIR/synapse.toml`,
or `~/.config/synapse.toml`.
If it is not present or invalid, a default configuration will be used.
These defaults are given in `example_config.toml`.

Sycli can be configured in a similar manner, using `sycli.toml`.

### Desktop application

Copy [`share/synapse/applications/synapse.desktop`] to `$XDG_DATA_HOME/applications` or `~/.local/share/applications`.

[`share/synapse/applications/synapse.desktop`]: share/synapse/applications/synapse.desktop

[XDG MIME Applications] example configuration:

`~/.config/mimeapps.list`

``` ini
[Default Applications]
x-scheme-handler/magnet=synapse.desktop
```

[XDG MIME Applications]: https://wiki.archlinux.org/index.php/XDG_MIME_Applications

## Development
Please see [this issue](https://github.com/Luminarys/synapse/issues/1) for details on development status.
If you're interested in developing a client for synapse, see `doc/RPC` for the current RPC spec.
if you'd like to contribute to synapse, see `doc/HACKING`.
