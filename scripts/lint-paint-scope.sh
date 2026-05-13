#!/usr/bin/env bash
# CI guard for daruda CLAUDE.md Pitfall #8.
#
# `window.text_style()` and `window.rem_size()` return GPUI's root
# defaults outside the paint walk. Reading them in event handlers,
# IME callbacks, or resize paths makes the event-side numbers drift
# from the paint-side numbers — the original cause of the
# "scrollback bottom won't select" cursor-drift bug.
#
# All terminal-view metric reads must instead route through
# `TerminalView::cell_layout(window)` (which itself goes through
# `text_metrics::cell_metrics_at`, the single entry point that calls
# `window.text_style()` legitimately during paint).
#
# This script greps `crates/daruda_terminal/src/view/` for direct
# `window.text_style()` / `window.rem_size()` calls and exits
# non-zero on any hit outside the whitelist:
#
#   * `text_metrics.rs` — definition of `cell_metrics_at`, the only
#     legitimate primitive caller.
#   * `element/prepaint.rs::prepaint` — runs inside the paint walk, so
#     `window.text_style()` returns the real per-pane font there.
#
# Run from the daruda crate root.
#
# Exit codes:
#   0 — no offending callers
#   1 — at least one offending caller (printed)

set -euo pipefail

VIEW_DIR="crates/daruda_terminal/src/view"

if [[ ! -d "$VIEW_DIR" ]]; then
    echo "lint-paint-scope: $VIEW_DIR not found — run from the daruda crate root." >&2
    exit 2
fi

# Whitelist: paths that legitimately call window.text_style() inside
# the paint walk or as the primitive definition.
WHITELIST_REGEX='(text_metrics\.rs|element/(mod|prepaint)\.rs)'

# Match line content but exclude comments and the whitelisted files.
# The grep result has the form `path:line:content`; we then drop
# whitelisted paths and any line that is a Rust comment (`//`).
HITS=$(grep -RHn -E 'window\.(text_style|rem_size)\(\)' "$VIEW_DIR" \
    --include='*.rs' \
    | grep -v -E "$WHITELIST_REGEX" \
    | grep -v -E '^[^:]+:[0-9]+:\s*//' \
    || true)

if [[ -n "$HITS" ]]; then
    echo "lint-paint-scope: forbidden window.text_style() / window.rem_size() calls" >&2
    echo "Use TerminalView::cell_layout(window) (or the appropriate state field)" >&2
    echo "instead. See daruda CLAUDE.md Pitfall #8." >&2
    echo "" >&2
    echo "$HITS" >&2
    exit 1
fi

echo "✓ No paint-scope window reads outside text_metrics.rs / element prepaint."
