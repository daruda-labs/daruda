#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if [[ "${1:-}" == "clean" ]]; then
    cargo clean
else
    rm -rf target/debug/deps target/debug/incremental
fi
cargo run -p daruda 
