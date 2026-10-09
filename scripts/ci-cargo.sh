#!/usr/bin/env bash
# Shared Cargo gates for all platform jobs. OS setup and runtime checks stay
# in ci.yml; package coverage is defined here once.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ "$#" -eq 0 ]; then
    echo "Usage: bash scripts/ci-cargo.sh {clippy|app-features|test|doc} [cargo options]" >&2
    exit 2
fi
gate="$1"
shift

packages=(
    ghostty_vt ghostty_vt_sys daruda_terminal daruda daruda_config
    daruda_store daruda_agent daruda_update daruda_acp daruda_core
    daruda_flow daruda_project daruda_ui daruda_control_types daruda_content
    daruda_flow_edit ferrum_flow vendor_gpui test_process strings_gen
)

case "$gate" in
    clippy) command=(cargo clippy --locked "$@") ;;
    test) command=(cargo test --locked "$@"); packages+=(gpui_component) ;;
    app-features)
        exec cargo clippy --locked "$@" -p daruda --all-features --all-targets -- -D warnings
        ;;
    doc)
        command=(cargo doc --locked "$@" --no-deps)
        packages=(daruda_flow daruda_project daruda_ui daruda_control_types daruda_content
            daruda_flow_edit daruda_core daruda_update ghostty_vt_sys ghostty_vt daruda_agent)
        export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings"
        ;;
    *) echo "Unknown Cargo gate: $gate" >&2; exit 2 ;;
esac

for package in "${packages[@]}"; do
    command+=(-p "$package")
done
if [ "$gate" = clippy ]; then
    command+=(--all-targets -- -D warnings)
fi
exec "${command[@]}"
