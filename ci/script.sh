#!/bin/bash
set -ex

. "$(dirname $0)/util.sh"

main() {
    CFLAGS=$CFLAGS cargo check --features="mmap" --target "$TARGET" --verbose --all
    CFLAGS=$CFLAGS cargo build --target "$TARGET" --verbose --all
    CFLAGS=$CFLAGS cargo test --target "$TARGET" --verbose --all
}

main
