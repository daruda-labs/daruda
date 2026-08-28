#!/usr/bin/env bash
# Lint: forbid the vendored declarative `.context_menu(...)` attachment.
#
# Background: `gpui_component`'s `ContextMenuExt::context_menu` renders its
# menu with `deferred()` from inside the attaching element's subtree. A
# deferred draw does not escape an ancestor's clip — gpui captures the ambient
# `content_mask` into `DeferredDraw` and re-applies it on paint, and
# `Window::with_content_mask` only intersects — and `Frame::hit_test`
# intersects each hitbox with that same mask. Every daruda dock body and pane
# clips, so such a menu is cut at the container's edge both visually and for
# hit-testing: the overflowing part is invisible *and* unclickable.
#
# Right-click menus go through `crate::workspace::root_menu`'s
# `RootContextMenuExt::root_context_menu`, which deploys at the workspace root
# via `Workspace::open_context_menu`.
#
# `crate::ui` does not re-export `ContextMenuExt`, so the broken form is
# already a compile error in app code (and `lint-direct-gpui-component.sh`
# blocks reaching past `crate::ui` for it). This lint is the readable
# statement of intent — a compile error says "no such method", not "use
# root_context_menu instead".
#
# Usage:
#   scripts/lint-declarative-context-menu.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The wrapper home is the only place allowed to name the vendored API, on the
# same terms every lint in this family grants it. Nothing there uses it today.
ALLOW='\bcrates/app/src/ui/'

# Anchored after leading whitespace so `///` doc mentions don't trip it.
violations=$(
    grep -rn --include="*.rs" -E '^[[:space:]]*\.context_menu\(' crates/app/src \
        | grep -Ev "$ALLOW" || true
)

if [[ -n "$violations" ]]; then
    echo "error: declarative .context_menu(...) is forbidden — it is clipped by"
    echo "       the caller's container (see crates/app/src/workspace/root_menu.rs)."
    echo "       Use .root_context_menu(workspace, builder) instead."
    echo
    echo "$violations"
    exit 1
fi

echo "ok: no declarative .context_menu(...) attachments"
