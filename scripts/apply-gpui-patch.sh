#!/usr/bin/env bash
# Compatibility entry point: verify the checked-in override without mutation.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --locked -p vendor_gpui -- --check
