#!/bin/bash
set -ex

. "$(dirname $0)/util.sh"

main() {
    CFLAGS=$CFLAGS cargo check --locked --target "$TARGET" --verbose --all
    CFLAGS=$CFLAGS cargo build --target "$TARGET" --verbose --all
    RUST_BACKTRACE=1 CFLAGS=$CFLAGS cargo test --target "$TARGET" --verbose --all
}

main
