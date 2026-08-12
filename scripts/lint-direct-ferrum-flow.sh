#!/usr/bin/env bash
# Lint: direct `ferrum_flow` access outside `crate::ui::flow_canvas`.
#
# Same rule as lint-direct-gpui-component.sh, for the other vendored
# crate. App code reaches the node-graph canvas only through
# `crate::ui::flow_canvas`, so replacing or pruning the vendored crate
# touches one file instead of every call site.
#
# Unlike the gpui_component lint this matches more than `use` lines: a
# fully-qualified `ferrum_flow::Graph::new()` needs no import and would
# otherwise walk straight past the boundary.
set -euo pipefail

cd "$(dirname "$0")/.."

# The wrapper module itself, and nothing else.
ALLOW='\bcrates/app/src/ui/flow_canvas\.rs'

violations=$(
    grep -rnE --include="*.rs" '(^[[:space:]]*use[[:space:]]+ferrum_flow\b|\bferrum_flow::)' crates/app/src \
        | grep -Ev "$ALLOW" \
        || true
)

if [ -n "$violations" ]; then
    echo "Direct ferrum_flow access outside crate::ui::flow_canvas:"
    echo "$violations"
    echo
    echo "Route it through crate::ui::flow_canvas and re-export what you need there."
    exit 1
fi

echo "[lint-direct-ferrum-flow] ok"
