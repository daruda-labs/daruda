#!/usr/bin/env bash
# Lint: direct `gpui_component` / `daruda_ui` access outside `crate::ui::*`.
#
# Background: `packages/ui/` (`daruda_ui`) is the wrapper home for the
# vendored `gpui_component` crate, and `packages/app/src/ui/` is the app's
# facade over it. App code must always go through `crate::ui::*` so that:
#   - `xsmall()` is auto-applied at one place (CLAUDE.md §10),
#   - widget defaults / variants stay consistent across the app,
#   - future re-styling lives in one module instead of N call sites,
#   - localized wrappers in the facade are not bypassed for their
#     label-taking `daruda_ui` originals.
#
# A small allow-list covers infrastructure files that legitimately
# touch `gpui_component` directly (init / Root wrapping / Theme):
#   - src/ui/                     — the facade (the entire point)
#   - src/main.rs                 — `gpui_component::init(cx)` at startup
#   - src/test_support.rs         — `init(cx)` for unit tests
#   - src/windows.rs              — `gpui_component::Root::new(...)`
#   - src/workspace/render/mod.rs — `Root::render_*_layer` in Workspace
#
# Usage:
#   scripts/lint-direct-gpui-component.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOW='\bpackages/app/src/ui/|\bpackages/app/src/(main|test_support|windows)\.rs|\bpackages/app/src/workspace/render/mod\.rs'

violations=$(
    grep -rnE --include="*.rs" '(^[[:space:]]*use[[:space:]]+(gpui_component|daruda_ui)\b|\bdaruda_ui::)' packages/app/src \
        | grep -Ev "$ALLOW" \
        || true
)

if [ -n "$violations" ]; then
    echo "Direct gpui_component / daruda_ui access outside crate::ui::*:"
    echo "$violations"
    echo
    echo "Route through crate::ui::*. See packages/app/src/ui/mod.rs."
    exit 1
fi
