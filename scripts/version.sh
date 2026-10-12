#!/usr/bin/env bash
# Print the daruda package version, the one value every packaging script and
# the release tag check must agree on. `--no-deps` needs no network.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo metadata --locked --no-deps --format-version 1 \
    | jq -er '.packages[] | select(.name == "daruda") | .version'
