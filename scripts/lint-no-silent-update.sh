#!/usr/bin/env bash
# Lint: forbid silent GPUI update results.
#
# Background: `cx.update_window(...)`, `cx.update(...)`, and the
# `entity.update_in(...)` family all return `Result<T>` / similar
# fallible shapes — the call can fail when the target window /
# context has gone away mid-flight (modal teardown race, dropped
# window, etc.). `let _ = cx.update_window(...)` discards that
# failure: no toast, no log, no panic — just a silent no-op that
# leaves the user with "the button did nothing" and the developer
# with zero diagnostics. The May-2026 add-project regression
# (chooser-modal callback re-entering update_window on the same
# window) sat undiagnosed for a full debugging cycle because of this
# pattern.
#
# Required form:
#   - `match cx.update_window(handle, ...) { Ok(_) => ..., Err(e) => ... }`
#   - `cx.update_window(handle, ...)?` (when the surrounding fn returns Result)
#   - `crate::windows::try_update_workspace_window(...)` helper (auto-logs)
#   - explicit ignore with `// SILENT-OK: <reason>` on the line above
#
# `// SILENT-OK:` is reserved for cases where the failure genuinely
# doesn't matter (test fixtures, focus-restore on a window that may
# already be closed, etc.). Every use must give a concrete reason —
# unjustified markers will fail review.
#
# Usage:
#   scripts/lint-no-silent-update.sh
#
# Exit codes:
#   0 — clean (no unmarked silent results)
#   1 — at least one unmarked `let _ = …update*(...)` found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Scan only first-party crates. Vendored gpui_component follows its
# own conventions.
SCAN_DIRS=(
    "crates/app/src"
    "crates/daruda_store/src"
    "crates/daruda_terminal/src"
    "crates/daruda_config/src"
    "crates/daruda_agent/src"
)

# Match `let _ = <expr>.update*(...)` shapes. Catches:
#   let _ = cx.update_window(...)
#   let _ = cx.update(|...| ...)
#   let _ = app_cx.update_window(...)
#   let _ = async_cx.update(...)
#   let _ = some_cx.update_in(...)
#   let _ = some_cx.update_global(...)
#   let _ = entity.update(...)
#   let _ = entity.update_in(...)
#
# Anchored to start-of-line whitespace + `let` so doc / inline comments
# that happen to contain the literal text `let _ = cx.update_window(...)`
# (e.g. when explaining the rule itself in a doc comment) don't trip
# the linter.
PATTERN='^[[:space:]]*let[[:space:]]+_[[:space:]]*=[[:space:]]*[A-Za-z0-9_.]*\.update(_window|_in|_global)?\('

# Tests / dev tooling are exempt — they may legitimately ignore
# update results (the panic on a broken test fixture is enough
# diagnostic). Vendored gpui_component too.
violations=$(
    grep -rEn "$PATTERN" "${SCAN_DIRS[@]}" \
        --include='*.rs' \
        | grep -Ev '/tests?\.rs:|/tests/' \
        || true
)

# Filter out lines whose preceding line carries the `// SILENT-OK:`
# marker. We do this by walking the violation list with awk against
# the actual file content so multi-violation runs stay accurate.
unmarked=""
while IFS= read -r line; do
    [ -z "$line" ] && continue
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    if [ "$lineno" -gt 1 ]; then
        prev=$(sed -n "$((lineno - 1))p" "$file")
        if echo "$prev" | grep -Eq '//[[:space:]]*SILENT-OK:'; then
            continue
        fi
    fi
    if [ -z "$unmarked" ]; then
        unmarked="$line"
    else
        unmarked="$unmarked"$'\n'"$line"
    fi
done <<< "$violations"

if [ -n "$unmarked" ]; then
    echo "Forbidden silent update results — route failures through a Result match,"
    echo "the \`try_update_workspace_window\` helper, or mark with \`// SILENT-OK: <reason>\`:"
    echo
    echo "$unmarked"
    echo
    echo "See CLAUDE.md §\"Error reporting\" / app/CLAUDE.md §\"Debugging policy\"."
    exit 1
fi

echo "✓ No unmarked silent update results found in tracked source."
