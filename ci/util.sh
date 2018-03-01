#!/bin/bash
set -ex

is_linux() {
    case "$TRAVIS_OS_NAME" in
        linux) return 0 ;;
        *)     return 1 ;;
    esac
}

is_osx() {
    case "$TRAVIS_OS_NAME" in
        osx) return 0 ;;
        *)   return 1 ;;
    esac
}
