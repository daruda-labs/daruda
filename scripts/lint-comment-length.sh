#!/usr/bin/env bash
# Lint: comment-length budget warnings (CLAUDE.md "Comments" rule).
#
# Background: root `CLAUDE.md` §"Development rules" says comments should
# "Keep each to 2-3 lines — summarize, don't explain at length." That rule
# has no mechanical enforcement today, which is exactly the failure mode
# CLAUDE.md's own "Cross-profile data isolation" section warns about: a
# convention that depends only on generation-time discipline drifts, because
# nobody re-checks it once the surrounding code already reads a certain way.
# This script gives the rule the same kind of check every other convention
# in this repo already has (file size, inline literals, silent-update
# swallowing, agent-activity encapsulation, …).
#
# What it flags: a run of consecutive `///`, `//!`, or `//` comment lines
# (a blank line or a non-comment line ends the run) longer than the budget.
#
# What it exempts: a run containing one of the tags CLAUDE.md already
# recognizes as "this longer explanation is deliberate" —
# SAFETY: (also the `SAFETY(MVU):`-style qualified variant) / WORKAROUND: /
# INVARIANT: / SILENT-OK: / DIAG: — since those are already flagged by the
# author as a justified exception, not an oversight.
#
# Advisory, not a hard limit: some non-obvious WHY (a race condition, a
# multi-step invariant) genuinely needs more than 2-3 lines even without one
# of the tags above. This lint surfaces growth for a reviewer to notice, the
# same warn-only-by-default shape as scripts/lint-file-size.sh. Pass
# `--strict` to fail (used by CI gating that explicitly opts in).
#
# Usage:
#   scripts/lint-comment-length.sh           # warn-only (exit 0 even on hits)
#   scripts/lint-comment-length.sh --strict  # exit 1 on any over-budget block
#
# Exit codes:
#   0 — no over-budget comment blocks (or warn-only mode)
#   1 — at least one over-budget block (only with --strict)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STRICT=0
if [ "${1-}" = "--strict" ]; then
    STRICT=1
fi

BUDGET=6

# Skip directories — vendored crates and target builds aren't ours (see
# CLAUDE.md: gpui_component "is excluded from clippy/lint/comment-cleanup
# passes").
SKIP_DIRS=(
    "crates/gpui_component"
    "crates/gpui_component_assets"
    "crates/gpui_component_macros"
    "crates/ghostty_vt"
    "crates/ghostty_vt_sys"
    "vendor"
    "target"
)

is_in_skip_dir() {
    local path="$1"
    for d in "${SKIP_DIRS[@]}"; do
        case "$path" in
            "$d"/*) return 0 ;;
        esac
    done
    return 1
}

hits=()

check_file() {
    local file="$1"
    local block_start=0
    local block_len=0
    local block_tagged=0
    local lineno=0

    flush() {
        if [ "$block_len" -gt "$BUDGET" ] && [ "$block_tagged" -eq 0 ]; then
            local block_end=$((block_start + block_len - 1))
            hits+=("$file:$block_start-$block_end: $block_len-line comment block (budget $BUDGET)")
        fi
        block_len=0
        block_tagged=0
    }

    while IFS= read -r line; do
        lineno=$((lineno + 1))
        if [[ "$line" =~ ^[[:space:]]*// ]]; then
            if [ "$block_len" -eq 0 ]; then
                block_start=$lineno
            fi
            block_len=$((block_len + 1))
            if [[ "$line" =~ (SAFETY(\([A-Za-z]+\))?|WORKAROUND|INVARIANT|SILENT-OK|DIAG): ]]; then
                block_tagged=1
            fi
        else
            flush
        fi
    done <"$file"
    flush
}

while IFS= read -r f; do
    if is_in_skip_dir "$f"; then
        continue
    fi
    check_file "$f"
done < <(find crates -name '*.rs' -type f | sort)

if [ ${#hits[@]} -eq 0 ]; then
    echo "[lint-comment-length] OK — no over-budget comment blocks."
    exit 0
fi

echo "[lint-comment-length] over-budget comment blocks (CLAUDE.md \"Comments\" rule):"
printf '  %s\n' "${hits[@]}"
echo
echo "Action: trim to the non-obvious WHY in 2-3 lines. If the length is"
echo "genuinely load-bearing (a race condition, a multi-step invariant),"
echo "tag it SAFETY:/WORKAROUND:/INVARIANT:/SILENT-OK:/DIAG: to mark the"
echo "exception explicitly instead of leaving it unmarked."

if [ "$STRICT" -eq 1 ]; then
    exit 1
fi
echo
echo "(warn-only mode — pass --strict to fail.)"
exit 0
