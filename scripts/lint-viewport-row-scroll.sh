#!/usr/bin/env bash
# Lint: viewport-row reads must dispatch on scroll offset.
#
# Background: the terminal keeps three vertical row spaces — grid row
# (the live grid), viewport row (what is painted now), and absolute
# screen row (unified scrollback + grid). A viewport row maps to a
# ghostty live-grid row only when `scroll_offset == 0`; once the user
# scrolls into history they diverge. So any `TerminalSession` method
# that hands a viewport-relative row to ghostty's grid-relative
# `self.terminal.dump_viewport_row*` MUST first dispatch on
# `scroll_offset` (or recompute the grid row from the unified frame via
# `line_buffer.wrapped_row_count`). `dump_viewport_row` once skipped
# that branch and painted live grid rows (an agent's input box) over
# scrolled-back content — the "input box afterimage" overlay bug.
#
# Rule: in crates/daruda_terminal/src/session/, any function whose body
# calls `self.terminal.dump_viewport_row` (any suffix) must also
# reference `scroll_offset` or `wrapped_row_count` in that same body.
# The type system can't enforce this — the dispatch lives inside the
# method, and the FFI takes a bare u16 — so this grep guard backstops it.
#
# See crates/daruda_terminal/src/view/CLAUDE.md "Coordinate spaces".
#
# Usage:   scripts/lint-viewport-row-scroll.sh
# Exit:    0 — clean   1 — a viewport-row read skips the scroll dispatch

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SCAN_DIR="crates/daruda_terminal/src/session"

# Walk each function body (tracked by brace depth, with string literals
# and line comments stripped first so format!("{}") braces don't skew the
# count) and flag any that reach the grid FFI without a scroll dispatch.
#
# The bad-pattern check runs against the *stripped* body so a comment that
# happens to mention `scroll_offset` ("// scroll_offset handled by caller")
# can't whitewash a body that never branches on it.
violations=$(
    find "$SCAN_DIR" -name '*.rs' -not -name 'tests.rs' -print0 \
    | xargs -0 awk '
        function flush(    bad) {
            bad = (stripped_body ~ /self\.terminal\.dump_viewport_row/) \
               && (stripped_body !~ /scroll_offset/) \
               && (stripped_body !~ /wrapped_row_count/)
            if (in_fn && bad) {
                printf "%s:%d: fn %s reaches self.terminal.dump_viewport_row* without a scroll_offset / wrapped_row_count dispatch\n", \
                    fname, fn_line, fn_name
            }
            in_fn = 0; stripped_body = ""; depth = 0; opened = 0
        }
        {
            fname = FILENAME
            stripped = $0
            gsub(/"[^"]*"/, "", stripped)   # drop string contents
            sub(/\/\/.*/, "", stripped)     # drop line comments
            o = gsub(/\{/, "{", stripped)   # count braces
            c = gsub(/\}/, "}", stripped)

            # Match every Rust fn modifier combination we use today:
            # `pub(crate)/pub(super)` parenthesized visibility, plus any
            # ordering of `unsafe / const / async / extern ["abi"]`.
            if (!in_fn) {
                if (stripped ~ /^[[:space:]]*(pub(\([^)]+\))?[[:space:]]+)?((unsafe|const|async|extern)[[:space:]]+("[^"]+"[[:space:]]+)?)*fn[[:space:]]/) {
                    in_fn = 1; fn_line = FNR; stripped_body = stripped; depth = 0; opened = 0
                    name = $0
                    sub(/^.*fn[[:space:]]+/, "", name); sub(/[^A-Za-z0-9_].*$/, "", name)
                    fn_name = name
                    depth += o - c
                    if (o > 0) opened = 1
                    if (opened && depth <= 0) flush()
                }
                next
            }

            stripped_body = stripped_body "\n" stripped
            depth += o - c
            if (o > 0) opened = 1
            if (opened && depth <= 0) flush()
        }
        END { if (in_fn) flush() }
    '
)

if [ -n "$violations" ]; then
    echo "Viewport-row reads must dispatch on scroll offset:"
    echo
    echo "$violations"
    echo
    echo "A function handing a viewport row to self.terminal.dump_viewport_row* must"
    echo "branch on self.scroll_offset (or recompute the grid row from"
    echo "line_buffer.wrapped_row_count, as dump_screen_row does). See"
    echo "crates/daruda_terminal/src/view/CLAUDE.md \"Coordinate spaces\"."
    exit 1
fi

echo "✓ All session viewport-row reads dispatch on scroll offset."
