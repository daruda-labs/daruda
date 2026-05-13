#!/usr/bin/env bash
# Lint: forbid `eprintln!` in user-error paths.
#
# Background: errors must flow through the 3-layer pipeline (toast →
# details modal → on-disk NDJSON log) so users running the `.app`
# bundle can see them. Bare `eprintln!` only reaches stderr, which is
# invisible unless the user launched daruda from a terminal. See the
# project root CLAUDE.md §"Error reporting" for the full rule set
# (`self.report_error(report, cx)` / `LogWriter::log(report)`).
#
# A small allow-list covers four legitimate `eprintln!` callers (do
# not extend this list without updating CLAUDE.md):
#   - bootstrap.rs panic-hook stderr fallback (must survive a dead LogWriter)
#   - watchers.rs pump-exit log (normal-shutdown signal, not an error)
#   - hooks/handler.rs external hook subprocess stderr (boundary)
#   - daruda_store/src/observability/log_writer.rs bootstrap fallbacks
#     (LogWriter cannot log to itself)
#
# Usage:
#   scripts/lint-no-eprintln.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOW='\bcrates/app/src/bootstrap\.rs|\bcrates/app/src/watchers\.rs|\bcrates/app/src/hooks/handler\.rs|\bcrates/daruda_store/src/observability/log_writer\.rs'

# Scan only first-party crates. Vendored gpui_component is excluded —
# it is upstream code with its own logging conventions.
SCAN_DIRS=(
    "crates/app/src"
    "crates/daruda_store/src"
    "crates/daruda_terminal/src"
    "crates/daruda_config/src"
    "crates/daruda_claude/src"
    "crates/ghostty_vt/src"
    "crates/ghostty_vt_sys/src"
)

# Match `eprintln!` outside test code (`#[cfg(test)] mod tests`,
# `#[test]` fns, files ending in `tests.rs`, files under `tests/`).
# False-positive tolerance: doc comments and the file's own use of
# `eprintln!` inside a `#[cfg(test)]` block will both be skipped at
# call-site review (the regex below intentionally only catches the
# call form `eprintln!(`).
violations=$(
    grep -rEn '^[[:space:]]*eprintln!\(' "${SCAN_DIRS[@]}" \
        --include='*.rs' \
        | grep -Ev "$ALLOW" \
        | grep -Ev '/tests?\.rs:|/tests/' \
        || true
)

if [ -n "$violations" ]; then
    echo "Forbidden eprintln! call sites — route errors through report_error / LogWriter::log:"
    echo "$violations"
    echo
    echo "See CLAUDE.md §'Error reporting' for the decision tree."
    echo "Allowed exceptions are listed in this script's header."
    exit 1
fi

echo "✓ No forbidden eprintln! calls found in tracked source."
