#!/usr/bin/env bash
# The repo's one check: formatting, lints as errors, the whole suite. `GRANITE_AGENT_BUILD=1` adds the installer test,
# which builds fixtures/hello-agent in release (a minute).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
echo "granite: check passed"
