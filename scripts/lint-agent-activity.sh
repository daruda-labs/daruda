#!/usr/bin/env bash
# Lint: confine the agent-chat activity signal to a single source.
#
# Background
# ----------
# The agent-chat pane has exactly one notion of "is this pane working / did it
# just finish", and it is derived, not the raw prompt-`Turn`:
#
#   * `is_busy()` / `activity_state()` / `activity_elapsed()` — the sanctioned
#     predicates every consumer (lane badge, status pulse, working indicator,
#     snapshots) must read. They fold the foreground prompt turn *and* any
#     trailing background subagents into one signal.
#   * `fire_activity_completion` — the single place completion side effects
#     (desktop notification + backing-task "done") are emitted, fired on the
#     busy→idle settle edge detected by `reconcile_activity`.
#
# The prompt `Turn` enum is an internal sequencing detail of the view
# (Send↔Stop affordance + one-turn-at-a-time prompt queue). It settles busy→idle
# *before* trailing subagents finish, so reading `turn.is_in_flight()` as the
# activity signal reports "idle" while work is still running, and firing
# completion straight from an `AcpEvent::TurnEnded` / `AcpEvent::Error` match arm
# fires early and can double-fire. `Turn` is therefore module-private to
# `agent_chat_pane/view.rs`.
#
# This script enforces two invariants against regressing to the raw `Turn`:
#
#   (a) No `.turn` field access and no `Turn::{InFlight,Idle}` construction /
#       match in production code outside `view.rs`. External consumers must go
#       through `is_busy()` / `activity_state()` / `activity_elapsed()`.
#
#   (b) The file that owns `apply_event`'s `AcpEvent::TurnEnded` /
#       `AcpEvent::Error` match arms (view.rs) must not call the completion
#       sinks (`apply_agent_chat_task_ended` / `maybe_notify_agent_completed`)
#       directly — completion must route out via `fire_activity_completion` at
#       the settle edge. (agent_chat_ops.rs legitimately owns those sinks behind
#       `fire_activity_completion`, and uses only an `if let … = &event` on
#       `AcpEvent::Error`, not a match arm, so it is never a candidate here.)
#
# Test code is exempt from (a): the tests drive `Turn` through the sanctioned
# `#[cfg(test)]` hooks (`set_turn_in_flight` / `set_turn_idle` / `turn_is_idle`)
# on `AgentChatView`, which keep the field encapsulated.
#
# Usage:
#   scripts/lint-agent-activity.sh
#
# Exit codes:
#   0 — invariants hold
#   1 — at least one violation found
#   2 — script precondition not met (wrong directory)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VIEW_RS="crates/app/src/workspace/main_area/agent_chat_pane/view.rs"
SCAN_DIR="crates/app/src"

if [[ ! -f "$VIEW_RS" ]]; then
    echo "lint-agent-activity: $VIEW_RS not found — run from the repo root." >&2
    exit 2
fi

FAIL=0

# ── Check (a): no raw `Turn` reads/construction outside view.rs ─────────────
#
# Scope: production `.rs` under crates/app/src, excluding view.rs (the owner)
# and test files (which use the sanctioned #[cfg(test)] hooks). Comments are
# stripped (both a full-line `// …` and a trailing `// …` on a code line) before
# the match, so a doc-comment reference (e.g. pane.rs mentioning
# `turn.is_in_flight()`) or a trailing comment mentioning `turn`/`Turn::` does
# not false-positive. `Turn::` targets the agent enum specifically, so the
# unrelated `is_in_flight` boolean in settings_window is not matched.

HITS_A=$(
    grep -rEn '\.turn\b|Turn::(InFlight|Idle)\b' "$SCAN_DIR" \
        --include='*.rs' \
        | grep -vE '/agent_chat_pane/view\.rs:' \
        | grep -vE '/tests?\.rs:|/tests/' \
        | sed -E 's#//.*##' \
        | grep -E '\.turn\b|Turn::(InFlight|Idle)\b' \
        || true
)

if [[ -n "$HITS_A" ]]; then
    echo "" >&2
    echo "lint-agent-activity: FAIL — raw agent \`Turn\` accessed outside view.rs:" >&2
    echo "$HITS_A" >&2
    echo "" >&2
    echo "  The prompt \`Turn\` is not the activity signal — it settles busy→idle" >&2
    echo "  before trailing subagents finish. Read the derived predicates instead:" >&2
    echo "    is_busy() / activity_state() / activity_elapsed()" >&2
    echo "  Tests drive it through the #[cfg(test)] hooks on AgentChatView." >&2
    FAIL=1
else
    echo "✓ No raw agent Turn access outside view.rs."
fi

# ── Check (b): apply_event's AcpEvent arms must not fire completion directly ──
#
# view.rs owns the `AcpEvent::TurnEnded` / `AcpEvent::Error` match arms. Guard
# against wiring the completion sinks into those arms: completion must route out
# via `fire_activity_completion` at the reconcile settle edge. First confirm
# view.rs really holds the arms (so the check fails loudly if apply_event moves
# and this guard silently stops covering it), then forbid the sink calls there.

if ! grep -qE 'AcpEvent::(TurnEnded|Error)' "$VIEW_RS"; then
    echo "" >&2
    echo "lint-agent-activity: FAIL — expected AcpEvent::TurnEnded/Error match arms" >&2
    echo "  in $VIEW_RS but found none. If apply_event moved, update this lint so" >&2
    echo "  check (b) keeps covering the file that owns the event match." >&2
    FAIL=1
else
    HITS_B=$(
        grep -nE 'apply_agent_chat_task_ended\(|maybe_notify_agent_completed\(' "$VIEW_RS" \
            | sed -E 's#//.*##' \
            | grep -E 'apply_agent_chat_task_ended\(|maybe_notify_agent_completed\(' \
            || true
    )
    if [[ -n "$HITS_B" ]]; then
        echo "" >&2
        echo "lint-agent-activity: FAIL — completion fired from apply_event in view.rs:" >&2
        echo "$HITS_B" >&2
        echo "" >&2
        echo "  Completion side effects must not fire from a raw AcpEvent::TurnEnded /" >&2
        echo "  AcpEvent::Error arm — they fire early and can double-fire. Capture the" >&2
        echo "  outcome (pending_completion) and let reconcile_activity return it on the" >&2
        echo "  busy→idle edge, where fire_activity_completion emits it exactly once." >&2
        FAIL=1
    else
        echo "✓ view.rs does not fire completion from the AcpEvent match arms."
    fi
fi

# ── Summary ─────────────────────────────────────────────────────────────────
if [[ $FAIL -ne 0 ]]; then
    exit 1
fi
echo "✓ Agent activity single-source invariants hold."
