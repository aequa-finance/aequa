#!/usr/bin/env bash
# Runs the program's LiteSVM suites. The precompiles LiteSVM needs for ed25519 link OpenSSL, so on
# Windows this runs inside WSL (`wsl --exec bash scripts/test.sh`); the .so comes from
# `cargo build-sbf` (README), which runs on either side.
set -euo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${AEQUA_TEST_TARGET:-$HOME/.cache/aequa-test-target}"
cargo test -p aequa-tests "$@" -- --test-threads=4
