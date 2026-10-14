#!/usr/bin/env bash
# Check each supported target separately so incompatible platform edges do not mix.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

targets=(
    aarch64-apple-darwin
    x86_64-apple-darwin
    x86_64-unknown-linux-gnu
    x86_64-pc-windows-msvc
)

if [ "${1:-}" = "--list-targets" ]; then
    printf '%s\n' "${targets[@]}"
    exit 0
fi

for target in "${targets[@]}"; do
    cargo deny --locked --all-features --target "$target" check \
        --hide-inclusion-graph --warn unmaintained "$@"
done
