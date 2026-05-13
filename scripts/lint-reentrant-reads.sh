#!/usr/bin/env bash
# CI guard for GPUI Entity re-entrancy (CLAUDE.md Pitfall #4).
#
# Background
# ----------
# GPUI uses exclusive borrowing: while entity.update(cx, closure) or
# Render::render executes, the entity is in EntityState::Mut.  Any
# .read(cx) on the *same* entity from anywhere on that call stack panics:
#
#   "cannot read daruda::workspace::dock::Dock
#    while it is already being updated"
#
# cx.listener closures registered inside Dock::render run inside the Dock
# entity's update cycle.  If a listener reaches persist_state() synchronously,
# save_state() calls left_dock.read(cx) while Dock is still EntityState::Mut.
#
# Concrete call chain that caused the original bug:
#
#   Dock::render  →  cx.listener(sidebar click)
#     [Dock is EntityState::Mut]
#   ws.update(cx, |ws,cx| ws.set_sidebar_view(...))
#   ws.mark_dirty_and_save(cx)
#   ws.persist_state(cx)  →  save_state()
#   self.left_dock.read(cx)   ← PANIC
#
# Fix: mark_dirty_and_save always wraps persist_state in cx.defer so the
# read happens in the next effect cycle after all borrows are released.
#
# This script enforces two invariants:
#
#   Check 1 — mark_dirty_and_save must call cx.defer.
#             Removing the defer turns every listener-reachable caller into a
#             latent panic. This check guards the fix from being accidentally
#             reverted during refactoring.
#
#   Check 2 — persist_state must only be invoked via mark_dirty_and_save
#             (deferred) or from a call site annotated with:
#               // lint-reentrant-reads: <reason>
#             A raw self.persist_state(cx) in any event handler or
#             listener-reachable path bypasses the defer and re-introduces
#             the panic.
#
# Run from the daruda crate root.
#
# Exit codes:
#   0 — invariants hold
#   1 — at least one violation (printed to stderr)
#   2 — script precondition not met (wrong directory)

set -euo pipefail

WORKSPACE_DIR="crates/app/src/workspace"
MOD_RS="$WORKSPACE_DIR/mod.rs"

if [[ ! -f "$MOD_RS" ]]; then
    echo "lint-reentrant-reads: $MOD_RS not found — run from the daruda crate root." >&2
    exit 2
fi

FAIL=0

# ── Check 1: mark_dirty_and_save must call cx.defer ────────────────────────

BODY=$(awk '/fn mark_dirty_and_save/{found=1} found{print} found && /^    \}$/{exit}' "$MOD_RS")

if ! echo "$BODY" | grep -q "cx\.defer"; then
    echo "lint-reentrant-reads: FAIL — mark_dirty_and_save no longer calls cx.defer." >&2
    echo "  File: $MOD_RS" >&2
    echo "  Removing cx.defer makes every listener-reachable caller a latent panic." >&2
    echo "  See CLAUDE.md Pitfall #4." >&2
    FAIL=1
else
    echo "✓ mark_dirty_and_save contains cx.defer."
fi

# ── Check 2: persist_state callsites must be deferred or annotated ──────────
#
# Allowed patterns (grep -v to exclude):
#   fn persist_state    — function definition
#   weak.update(...)    — the deferred call inside mark_dirty_and_save
#   lint-reentrant-reads:  — explicit same-line safety annotation
#   /tests/             — test files run outside any entity update cycle
#   ^\s*///             — doc-comment references, not call sites

HITS=$(grep -rn "persist_state(" "$WORKSPACE_DIR" --include="*.rs" \
    | grep -v "fn persist_state"            \
    | grep -v "weak\.update"                \
    | grep -v "lint-reentrant-reads:"       \
    | grep -v "/tests/"                     \
    | grep -v "^\s*//"                      \
    | grep -v ":[[:space:]]*//"             \
    || true)

if [[ -n "$HITS" ]]; then
    echo "" >&2
    echo "lint-reentrant-reads: FAIL — persist_state called outside a deferred context:" >&2
    echo "$HITS" >&2
    echo "" >&2
    echo "  Route through mark_dirty_and_save(cx) instead." >&2
    echo "  If the call site is provably safe (e.g. no dock entity is in EntityState::Mut)," >&2
    echo "  add a same-line annotation:" >&2
    echo "    self.persist_state(cx); // lint-reentrant-reads: <reason>" >&2
    echo "  See CLAUDE.md Pitfall #4." >&2
    FAIL=1
else
    echo "✓ No bare persist_state calls outside deferred context."
fi

# ── Summary ─────────────────────────────────────────────────────────────────
if [[ $FAIL -ne 0 ]]; then
    exit 1
fi
echo "✓ No GPUI entity re-entrancy violations detected."
