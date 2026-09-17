#!/usr/bin/env bash
# Lint: keep the agent-chat row list told by one adapter, and keep the
# transcript's structural mutations where the invalidation duty is written down.
#
# Background
# ----------
# Two separate things in the agent-chat pane are keyed by position, and both
# break quietly rather than loudly when a new code path forgets them.
#
# 1. The gpui row list. `ListState` has two ways to say "heights changed", and
#    they differ in a way no call site makes visible: both keep the scroll-top
#    *item*, but `remeasure_items` keeps the offset inside it in pixels while
#    `remeasure` keeps it as a fraction of that item's height. Nineteen call
#    sites were each re-arguing that choice in prose. They now name the change
#    (`view/list_sync.rs`'s `ListSync`) and one adapter picks the anchor.
#
# 2. `items` indices. Fold keys (`FoldKey::Assistant(ix)` and friends) and the
#    per-turn records (`turn_records`, keyed by a run's first index) point into
#    `items` by position, so removing or inserting an item mid-transcript moves
#    state that nothing re-maps. The three production sites that mutate the
#    transcript structurally each carry that reasoning in a comment; a fourth
#    added without it is the failure this guards.
#
# Invariants
# ----------
#   A. `list_state.splice` / `.remeasure` / `.remeasure_items` appear only in
#      `view/list_sync.rs`. Everywhere else goes through `apply_list_sync` or
#      `resync_all_row_heights`. Read-only and scroll calls are unrestricted.
#   B. Structural `self.items` mutation appears only at the sites listed in
#      STRUCTURAL_ALLOW below. Adding one means adding it there — which is the
#      prompt to decide what it does to fold keys and turn records.
#
# Both invariants are scoped to production code; test modules are exempt.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PANE="crates/app/src/workspace/main_area/agent_chat_pane"
ADAPTER="$PANE/view/list_sync.rs"

# Files allowed to mutate `items` structurally, with what each is.
STRUCTURAL_ALLOW=(
    "$PANE/view/session_ops.rs" # settle_stop_marker (remove+push), teardown (clear), seed
)

status=0

# Strip `#[cfg(test)]` module bodies so a fixture building a transcript is not
# mistaken for a production mutation. Brace-counting from the attribute is
# enough here: every test module in this tree is a top-level `mod tests {`.
strip_tests() {
    awk '
        /^#\[cfg\(test\)\]/ { skipping = 1 }
        skipping {
            depth += gsub(/\{/, "{")
            depth -= gsub(/\}/, "}")
            if (depth <= 0 && /\}/) { skipping = 0; depth = 0 }
            next
        }
        { print FNR ": " $0 }
    ' "$1"
}

echo "== A: list_state mutations outside the adapter"
while IFS= read -r -d '' file; do
    [ "${file#"$ROOT_DIR"/}" = "$ADAPTER" ] && continue
    hits=$(strip_tests "$file" | grep -E 'list_state[[:space:]]*$|list_state\.(splice|remeasure)' || true)
    # `list_state` alone at end of line catches the rustfmt-wrapped
    # `self.list_state\n    .splice(..)` form.
    if [ -n "$hits" ]; then
        while IFS= read -r line; do
            # Only complain when a mutating method actually follows.
            if printf '%s' "$line" | grep -qE '\.(splice|remeasure)'; then
                echo "  ${file#"$ROOT_DIR"/}:${line}"
                status=1
            fi
        done <<<"$hits"
    fi
done < <(find "$ROOT_DIR/$PANE" -name '*.rs' -print0)

echo "== B: structural items mutation outside the allow-list"
while IFS= read -r -d '' file; do
    rel="${file#"$ROOT_DIR"/}"
    allowed=0
    for ok in "${STRUCTURAL_ALLOW[@]}"; do
        [ "$rel" = "$ok" ] && allowed=1
    done
    [ "$allowed" = 1 ] && continue
    hits=$(strip_tests "$file" |
        grep -E 'self\.items\.(remove|insert|drain|truncate|clear)\(|self\.items[[:space:]]*=' || true)
    if [ -n "$hits" ]; then
        echo "  $rel:"
        printf '    %s\n' "$hits"
        status=1
    fi
done < <(find "$ROOT_DIR/$PANE" -name '*.rs' -print0)

if [ "$status" = 0 ]; then
    echo "agent-list-sync OK"
else
    echo
    echo "A: name the change and let \`apply_list_sync\` pick the scroll anchor."
    echo "B: a structural \`items\` mutation moves fold keys and turn records —"
    echo "   decide what happens to them, then add the site to STRUCTURAL_ALLOW."
fi
exit "$status"
