#!/usr/bin/env bash
# GraniteRust's suite (fmt, clippy, every test, the installer's included) on Linux arm64 in Apple's container: the
# process group, wait4's units (ru_maxrss is kilobytes there) and /tmp are the platform code it proves.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
command -v container >/dev/null || { echo "Apple's container is not installed: https://github.com/apple/container/releases" >&2; exit 2; }
container system status >/dev/null 2>&1 || container system start
container build --tag granite-rust:linux --file "$root/containers/Dockerfile.linux" "$root"
container run --rm --memory "${GRANITE_CHECK_MEMORY:-8g}" --cpus "${GRANITE_CHECK_CPUS:-8}" granite-rust:linux
