#!/usr/bin/env bash
# Print the shared Rust floor and reject first-party/vendor MSRV drift and a
# rust-toolchain.toml channel that differs from the floor.
set -euo pipefail

# The pinned channel from a rust-toolchain.toml on stdin.
toolchain_channel() {
    sed -n 's/^channel = "\(.*\)"$/\1/p' | grep -E '^[0-9]+\.[0-9]+(\.[0-9]+)?$'
}

select_version() {
    jq -er '
        [.packages[] | select(.name == "daruda")] as $app
        | if ($app | length) != 1 then error("expected one daruda package") else . end
        | $app[0].rust_version as $version
        | if ($version | type) != "string" then error("daruda has no Rust floor")
          elif ($version | test("^[0-9]+\\.[0-9]+(\\.[0-9]+)?$")) | not
          then error("invalid Rust floor") else . end
        | [.packages[]
           | select((.name | startswith("gpui_component")) | not)
           | select(.rust_version != $version) | .name] as $drift
        | if ($drift | length) > 0
          then error("Rust floor differs in: " + ($drift | join(", ")))
          else $version end
    '
}

if [ "${1:-}" = "--self-test" ]; then
    for version in 1.95 1.95.0; do
        fixture=$(jq -n --arg version "$version" '{packages: [
            {name: "daruda", rust_version: $version},
            {name: "daruda_core", rust_version: $version},
            {name: "ferrum_flow", rust_version: $version},
            {name: "gpui_component", rust_version: "1.85"}
        ]}')
        [ "$(printf '%s' "$fixture" | select_version)" = "$version" ]
    done
    for fixture in \
        '{"packages": []}' \
        '{"packages": [{"name": "daruda"}]}' \
        '{"packages": [{"name": "daruda", "rust_version": "stable"}]}' \
        '{"packages": [{"name": "daruda", "rust_version": "1.95"}, {"name": "daruda_core", "rust_version": "1.94"}]}' \
        '{"packages": [{"name": "daruda", "rust_version": "1.95"}, {"name": "test_process", "rust_version": "1.94"}]}' \
        '{"packages": [{"name": "daruda", "rust_version": "1.95"}, {"name": "ferrum_flow", "rust_version": "1.94"}]}'; do
        if printf '%s' "$fixture" | select_version >/dev/null 2>&1; then
            echo "Rust floor check accepted invalid metadata: $fixture" >&2
            exit 1
        fi
    done
    for channel in 1.95 1.95.0; do
        [ "$(printf '[toolchain]\nchannel = "%s"\n' "$channel" | toolchain_channel)" = "$channel" ]
    done
    for fixture in '[toolchain]' 'channel = "stable"' 'channel = "1.95-x86_64-apple-darwin"'; do
        if printf '%s\n' "$fixture" | toolchain_channel >/dev/null 2>&1; then
            echo "Toolchain check accepted invalid channel: $fixture" >&2
            exit 1
        fi
    done
    echo "Rust floor self-test passed."
    exit 0
fi

[ $# -eq 0 ] || { echo "usage: $0 [--self-test]" >&2; exit 2; }
cd "$(dirname "${BASH_SOURCE[0]}")/.."
version=$(cargo metadata --locked --no-deps --format-version 1 | select_version)
channel=$(toolchain_channel < rust-toolchain.toml) || {
    echo "rust-toolchain.toml has no numeric channel" >&2
    exit 1
}
if [ "$channel" != "$version" ]; then
    echo "rust-toolchain.toml channel $channel differs from the Rust floor $version" >&2
    exit 1
fi
echo "$version"
