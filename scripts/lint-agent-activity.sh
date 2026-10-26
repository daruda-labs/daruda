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
# fires early and can double-fire. `Turn` is therefore module-private to the
# chat package's `view/` module tree (`packages/chat/src/view/`) (`view/mod.rs` + its submodules —
# `queue_ops/` owns the field, `session_ops.rs` / `apply_event/` /
# `tests.rs` touch it as `view`'s own descendants, which is exactly the
# access `pub(super)` / plain-private grants and nothing outside the tree
# does).
#
# This script enforces two invariants against regressing to the raw `Turn`:
#
#   (a) No `.turn` field access and no `Turn::{InFlight,Idle}` construction /
#       match in production code outside `view/`. External consumers must go
#       through `is_busy()` / `activity_state()` / `activity_elapsed()`.
#
#   (b) The file that owns `apply_event`'s `AcpEvent::TurnEnded` /
#       `AcpEvent::Error` match arms (view/apply_event/mod.rs) must not call the
#       completion sinks (`apply_agent_chat_task_ended` /
#       `maybe_notify_agent_completed`) directly — completion must route out
#       via `fire_activity_completion` at the settle edge. (agent_chat_ops/notify.rs
#       legitimately owns those sinks behind `fire_activity_completion`, and
#       uses only an `if let … = &event` on `AcpEvent::Error`, not a match
#       arm, so it is never a candidate here.)
#
# Test code is exempt from (a): the tests drive `Turn` through the sanctioned
# test hooks (`cfg(test)`, or the `test-support` feature for the app's tests) (`set_turn_in_flight` / `set_turn_idle` / `turn_is_idle`)
# on `AgentChatView`, which keep the field encapsulated.
#
# Usage:
#   scripts/lint-agent-activity.sh
#   scripts/lint-agent-activity.sh --self-test
#
# Exit codes:
#   0 — invariants hold
#   1 — at least one violation found
#   2 — script precondition not met (wrong directory)

set -euo pipefail

check_activity() (
    cd "$1"
    PANE_DIR="packages/chat/src"
    VIEW_DIR="$PANE_DIR/view"
    APPLY_EVENT_DIR="$VIEW_DIR/apply_event"
    APPLY_EVENT_RS="$APPLY_EVENT_DIR/mod.rs"
    SCAN_DIR="$PANE_DIR"

    if [[ ! -f "$VIEW_DIR/mod.rs" ]]; then
        echo "lint-agent-activity: $VIEW_DIR/mod.rs not found — run from the repo root." >&2
        exit 2
    fi

    FAIL=0

    # ── Check (a): no raw `Turn` reads/construction outside view/ ───────────────
    #
    # Scope: production `.rs` under the chat package, excluding the view/ module
    # tree (the owner — `Turn` is private to view/mod.rs's `PromptQueue`, and
    # view/'s submodules reach it only as view's own descendants) and test files (which use the sanctioned test hooks). Comments are stripped (both a full-line `// …` and a trailing
    # `// …` on a code line) before the match, so a doc-comment reference (e.g.
    # pane.rs mentioning `turn.is_in_flight()`) or a trailing comment mentioning
    # `turn`/`Turn::` does not false-positive.
    #
    # The pane tree is the whole risk zone, and scanning wider costs precision
    # rather than buying reach: `Turn` is module-private to view/, so code outside
    # the pane cannot name it and a hit there is always an unrelated `turn` — the
    # fold editor's turn *column* (`FoldEditorState::turn`) is one such name.

    # The narrowing above rests on `Turn` being unqualified-private, so confirm
    # that first: widening its visibility would silently un-defend the invariant
    # everywhere outside the scanned scope.
    if ! grep -qE '^enum Turn \{' "$VIEW_DIR/mod.rs"; then
        echo "" >&2
        echo "lint-agent-activity: FAIL — \`enum Turn\` is no longer private to view/:" >&2
        grep -nE '(pub(\([^)]*\))? )?enum Turn \{' "$VIEW_DIR/mod.rs" >&2 || true
        echo "" >&2
        echo "  Check (a) only scans $SCAN_DIR because nothing outside it can name" >&2
        echo "  \`Turn\`. A visibility wider than module-private breaks that premise —" >&2
        echo "  either restore it, or widen SCAN_DIR to every crate that could name it." >&2
        FAIL=1
    fi

    HITS_A=$(
        grep -rEn '\.turn\b|Turn::(InFlight|Idle|AwaitingCancelAck)\b' "$SCAN_DIR" \
            --include='*.rs' \
            | grep -vE '^packages/chat/src/view/' \
            | grep -vE '/tests?\.rs:|/tests/' \
            | sed -E 's#//.*##' \
            | grep -E '\.turn\b|Turn::(InFlight|Idle|AwaitingCancelAck)\b' \
            || true
    )

    if [[ -n "$HITS_A" ]]; then
        echo "" >&2
        echo "lint-agent-activity: FAIL — raw agent \`Turn\` accessed outside view/:" >&2
        echo "$HITS_A" >&2
        echo "" >&2
        echo "  The prompt \`Turn\` is not the activity signal — it settles busy→idle" >&2
        echo "  before trailing subagents finish. Read the derived predicates instead:" >&2
        echo "    is_busy() / activity_state() / activity_elapsed()" >&2
        echo "  Tests drive it through the test hooks on AgentChatView." >&2
        FAIL=1
    else
        echo "✓ No raw agent Turn access outside view/."
    fi

    # ── Check (b): apply_event's AcpEvent arms must not fire completion directly ──
    #
    # view/apply_event/mod.rs owns the `AcpEvent::TurnEnded` / `AcpEvent::Error`
    # match arms. Guard against wiring the completion sinks into those arms:
    # The lifecycle helpers are part of that reducer, so scan its complete tree.
    # Completion must route out via `fire_activity_completion` at the reconcile
    # settle edge. First confirm apply_event/mod.rs really holds the arms (so the
    # check fails loudly if apply_event moves again and this guard silently
    # stops covering it), then forbid sink calls throughout the reducer module.

    if ! grep -qE 'AcpEvent::(TurnEnded|Error)' "$APPLY_EVENT_RS"; then
        echo "" >&2
        echo "lint-agent-activity: FAIL — expected AcpEvent::TurnEnded/Error match arms" >&2
        echo "  in $APPLY_EVENT_RS but found none. If apply_event moved, update this" >&2
        echo "  lint so check (b) keeps covering the file that owns the event match." >&2
        FAIL=1
    else
        HITS_B=$(
            grep -rnE 'apply_agent_chat_task_ended\(|maybe_notify_agent_completed\(' "$APPLY_EVENT_DIR" --include='*.rs' \
                | sed -E 's#//.*##' \
                | grep -E 'apply_agent_chat_task_ended\(|maybe_notify_agent_completed\(' \
                || true
        )
        if [[ -n "$HITS_B" ]]; then
            echo "" >&2
            echo "lint-agent-activity: FAIL — completion fired from apply_event/:" >&2
            echo "$HITS_B" >&2
            echo "" >&2
            echo "  Completion side effects must not fire from a raw AcpEvent::TurnEnded /" >&2
            echo "  AcpEvent::Error arm — they fire early and can double-fire. Capture the" >&2
            echo "  outcome (pending_completion) and let reconcile_activity return it on the" >&2
            echo "  busy→idle edge, where fire_activity_completion emits it exactly once." >&2
            FAIL=1
        else
            echo "✓ apply_event/ does not fire completion from the AcpEvent match arms."
        fi
    fi

    # ── Summary ─────────────────────────────────────────────────────────────────
    if [[ $FAIL -ne 0 ]]; then
        exit 1
    fi
    echo "✓ Agent activity single-source invariants hold."
)

expect_activity() {
    local expected="$1" fixture="$2" label="$3" output status
    if output="$(check_activity "$fixture" 2>&1)"; then
        status=0
    else
        status=$?
    fi
    if [ "$status" -ne "$expected" ]; then
        echo "lint-agent-activity: self-test failed: $label (expected $expected, got $status)" >&2
        echo "$output" >&2
        return 2
    fi
}

self_test() {
    SELF_TEST_DIR="$(mktemp -d)"
    trap 'rm -rf "$SELF_TEST_DIR"' EXIT
    local view="$SELF_TEST_DIR/packages/chat/src/view"
    mkdir -p "$view/apply_event/lifecycle"
    printf '%s\n' 'enum Turn {' '    Idle,' '}' > "$view/mod.rs"
    printf '%s\n' 'match event { AcpEvent::TurnEnded => {} }' > "$view/apply_event/mod.rs"
    local nested="$view/apply_event/lifecycle/completion.rs"
    printf '%s\n' '// maybe_notify_agent_completed();' > "$nested"
    expect_activity 0 "$SELF_TEST_DIR" "nested comments are allowed"
    printf '%s\n' 'maybe_notify_agent_completed();' > "$nested"
    expect_activity 1 "$SELF_TEST_DIR" "nested completion sink is rejected"
    printf '%s\n' 'apply_agent_chat_task_ended();' > "$nested"
    expect_activity 1 "$SELF_TEST_DIR" "nested task completion sink is rejected"
    printf '%s\n' 'fire_activity_completion();' > "$nested"
    expect_activity 0 "$SELF_TEST_DIR" "reconciled completion is allowed"
    printf '%s\n' '// no event owner here' > "$view/apply_event/mod.rs"
    expect_activity 1 "$SELF_TEST_DIR" "missing reducer owner is rejected"
    echo "lint-agent-activity: self-test passed."
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1-}" in
    "") check_activity "$ROOT" ;;
    --self-test) self_test ;;
    *) echo "usage: scripts/lint-agent-activity.sh [--self-test]" >&2; exit 2 ;;
esac
