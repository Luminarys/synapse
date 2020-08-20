#!/bin/bash
set -ex

export PATH="$PATH:$HOME/.cargo/bin"

. "$(dirname $0)/util.sh"

install_rustup() {
    curl https://sh.rustup.rs -sSf \
      | sh -s -- -y --default-toolchain="$TRAVIS_RUST_VERSION"
    rustc -V
    cargo -V
}

install_targets() {
    if [ $(host) != "$TARGET" ]; then
        rustup target add $TARGET
    fi
}

main() {
    install_rustup
    install_targets
}

main
