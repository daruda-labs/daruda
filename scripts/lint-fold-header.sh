#!/usr/bin/env bash
# Lint: confine every agent-chat fold header to one assembly.
#
# Background
# ----------
# A fold header is one row with three slots — `leading` (fixed label), `stretch`
# (the single slot that eats the leftover width), `trailing` (fixed,
# right-anchored). That grammar lives in exactly one place:
#
#   crates/app/src/workspace/main_area/agent_chat_pane/render/fold_header.rs
#
# It used to be two free `AnyElement` parameters on `foldable_block`, so each of
# the seven headers re-derived the geometry and picked its own two of the three
# slots. Commit b24f776 then fixed the collapsed preview in *one* of them —
# adding the label→summary gap and the inline-markdown flattening inside
# `collapsed_text_summary` — and the response bar, holding its own copy, missed
# both. Users saw a 6 px gap difference and raw `**` between "one agent reply"
# and "several replies" in the same pane.
#
# This script enforces the three invariants that make that class of drift
# unreachable:
#
#   (a) `AGENT_CHAT_SUMMARY_GAP` is read only by fold_header.rs (its definition
#       in the palette aside). The label→summary gap has one application site.
#
#   (b) `summary_preview_line` is called only by fold_header.rs — i.e. only
#       through `SummaryLine::from_markdown`. No header can hand-roll a
#       first-line extraction that skips the markdown flattening.
#
#   (c) No header inside agent_chat_pane builds its own disclosure chevron.
#       `disclosure(` / `Disclosure` belong to fold_header.rs, so a new
#       collapsible header cannot bypass the grammar by starting from scratch.
#       There is no exemption: the plan region's collapse lives on a view flag
#       rather than a `FoldKey`, and it reaches the same assembly through
#       `FoldToggle::External`.
#
#   (d) A `trailing(` badge is never gated on fold state. Trailing content reads
#       the same expanded or collapsed. The stretch slot is the only one that
#       varies with fold state — hidden when expanded (`StretchSlot::Summary`)
#       or showing a different value in each (`StretchSlot::Alternate`). A
#       `collapsed` / `!expanded` guard around a `.trailing(` call is the drift
#       this check catches.
#
# What this does NOT catch
# ------------------------
#   * A hand-rolled `flex_1().min_w_0().overflow_hidden()` truncation slot that
#     never touches the constant or the chevron. There is no reliable grep for
#     "an ad-hoc one-line preview"; review a new preview slot by hand against
#     `fold_header::stretch_container`.
#   * Check (d) only sees a guard on the same line or the line above a
#     `.trailing(` call. A fold-state branch several lines up that appends a
#     badge is review's job.
#
# Usage:
#   scripts/lint-fold-header.sh
#
# Exit codes:
#   0 — invariants hold
#   1 — at least one violation found
#   2 — script precondition not met (wrong directory / module moved)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PANE_DIR="crates/app/src/workspace/main_area/agent_chat_pane"
OWNER="$PANE_DIR/render/fold_header.rs"
SCAN_DIR="crates/app/src"

if [[ ! -f "$OWNER" ]]; then
    echo "lint-fold-header: $OWNER not found — run from the repo root, or update" >&2
    echo "  this lint if the fold-header assembly moved." >&2
    exit 2
fi

FAIL=0

# Strip full-line and trailing `// …` comments so a doc reference to a guarded
# name is not a violation, then re-match to keep only real code hits.
strip_comments() { sed -E 's#//.*##'; }

# ── (a) the label→summary gap has one application site ──────────────────────
HITS_A=$(
    grep -rn 'AGENT_CHAT_SUMMARY_GAP' "$SCAN_DIR" \
        | grep -v "^$OWNER:" \
        | grep -v '^crates/app/src/ui/theme/palette.rs:' \
        | strip_comments \
        | grep 'AGENT_CHAT_SUMMARY_GAP' \
        || true
)
if [[ -n "$HITS_A" ]]; then
    echo "" >&2
    echo "lint-fold-header: FAIL — AGENT_CHAT_SUMMARY_GAP applied outside the" >&2
    echo "  fold-header assembly:" >&2
    echo "$HITS_A" >&2
    echo "" >&2
    echo "  The label→summary gap is part of the stretch slot's geometry. Build the" >&2
    echo "  header with FoldHeader::with_summary / with_title instead of restating" >&2
    echo "  the margin — that is exactly how the response bar drifted 6 px." >&2
    FAIL=1
else
    echo "✓ AGENT_CHAT_SUMMARY_GAP applied in fold_header.rs only."
fi

# ── (b) markdown previews go through SummaryLine::from_markdown ─────────────
HITS_B=$(
    grep -rn 'summary_preview_line' "$SCAN_DIR" \
        | grep -v "^$OWNER:" \
        | grep -v "^$PANE_DIR/agent_chat_helpers.rs:" \
        | grep -v "^$PANE_DIR/agent_chat_helpers/tests.rs:" \
        | strip_comments \
        | grep 'summary_preview_line' \
        || true
)
if [[ -n "$HITS_B" ]]; then
    echo "" >&2
    echo "lint-fold-header: FAIL — summary_preview_line called outside" >&2
    echo "  SummaryLine::from_markdown:" >&2
    echo "$HITS_B" >&2
    echo "" >&2
    echo "  Every collapsed preview must be a SummaryLine, so the flattening is" >&2
    echo "  applied once. Pass a builder to FoldHeader::with_summary." >&2
    FAIL=1
else
    echo "✓ Markdown previews route through SummaryLine::from_markdown."
fi

# ── (c) no second disclosure scaffold inside the pane ───────────────────────
HITS_C=$(
    grep -rnE '\bdisclosure\(|\bDisclosure\b' "$PANE_DIR" \
        | grep -v "^$OWNER:" \
        | strip_comments \
        | grep -E '\bdisclosure\(|\bDisclosure\b' \
        || true
)
if [[ -n "$HITS_C" ]]; then
    echo "" >&2
    echo "lint-fold-header: FAIL — a disclosure chevron built outside the" >&2
    echo "  fold-header assembly:" >&2
    echo "$HITS_C" >&2
    echo "" >&2
    echo "  A collapsible header is a FoldRow::section (content is sibling list" >&2
    echo "  rows) or a FoldRow::block (owns its body); collapse state that is not" >&2
    echo "  a FoldKey arrives as FoldToggle::external. Starting from a bare" >&2
    echo "  chevron re-opens the drift this module exists to prevent." >&2
    FAIL=1
else
    echo "✓ No disclosure scaffold outside fold_header.rs."
fi

# ── (d) trailing badges are not gated on fold state ─────────────────────────
#
# Match a `.trailing(` call whose own line, or the line immediately above it,
# tests the fold state. `grep -B1` gives both lines; the guard names are narrow
# (`collapsed` / `expanded`) so an unrelated conditional does not trip it.
HITS_D=$(
    grep -rn --include='*.rs' -B1 '\.trailing(' "$PANE_DIR" \
        | strip_comments \
        | grep -E '\bif +!?(collapsed|expanded)\b|\b(collapsed|expanded) +&&' \
        || true
)
if [[ -n "$HITS_D" ]]; then
    echo "" >&2
    echo "lint-fold-header: FAIL — a trailing badge gated on fold state:" >&2
    echo "$HITS_D" >&2
    echo "" >&2
    echo "  Trailing content reads the same expanded or collapsed — a status" >&2
    echo "  badge that vanishes on expand is the inconsistency this grammar" >&2
    echo "  removes. Only the stretch slot's summary is collapsed-only, and" >&2
    echo "  FoldHeader::with_summary already handles that without a guard." >&2
    FAIL=1
else
    echo "✓ No trailing badge gated on fold state."
fi

exit "$FAIL"
