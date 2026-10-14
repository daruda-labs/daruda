#!/usr/bin/env bash
# Read changed paths from stdin and select dependency/native-package checks.
set -euo pipefail

select_changes() {
    local path selected=false
    while IFS= read -r path || [ -n "$path" ]; do
        case "$path" in
            Cargo.toml | Cargo.lock | */Cargo.toml | */Cargo.lock | */build.rs | \
            .gitmodules | vendor/* | patches/*.patch | \
            scripts/build-app.sh | scripts/build-dmg.sh | scripts/package-* | \
            scripts/linux/* | scripts/Info.plist.tmpl | scripts/rust-version.sh | \
            scripts/check-packaging-changes.sh | .github/actions/setup-build/* | \
            .github/workflows/ci.yml | .github/workflows/release.yml | \
            assets/icon.* | LICENSE | LICENSES/*) selected=true ;;
        esac
    done
    printf '%s\n' "$selected"
}

if [ "${1:-}" = "--self-test" ]; then
    for path in Cargo.lock packages/acp/Cargo.toml packages/ghostty/build.rs \
        vendor/zed/UPSTREAM patches/gpui-window.patch scripts/package-linux.sh \
        scripts/build-app.sh scripts/linux/build-deps.txt .gitmodules \
        .github/actions/setup-build/action.yml .github/workflows/ci.yml \
        assets/icon.icns LICENSES/zed.txt; do
        [ "$(printf '%s\n' "$path" | select_changes)" = true ]
    done
    for path in README.md packages/app/src/main.rs packages/acp/AGENTS.md; do
        [ "$(printf '%s\n' "$path" | select_changes)" = false ]
    done
    [ "$(printf '' | select_changes)" = false ]
    [ "$(printf 'README.md\nCargo.lock\npackages/app/src/main.rs' | select_changes)" = true ]
    echo "Packaging change selector self-test passed."
    exit 0
fi

[ $# -eq 0 ] || { echo "usage: $0 [--self-test]" >&2; exit 2; }
select_changes
