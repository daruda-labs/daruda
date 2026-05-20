#!/usr/bin/env bash
# Forbid direct mark_dirty_and_save calls outside durable.rs and the
# definition in workspace/mod.rs. Wrappers in durable.rs delegate to the
# definition; callers must use mutate_durable / mutate_durable_in.
set -euo pipefail

# Allowed files: definition + wrappers. Anything else with a
# mark_dirty_and_save reference (other than the fn signature itself) is
# a violation.
offenders=$(rg -n --type rust "mark_dirty_and_save" crates/app/src \
    | rg -v "fn mark_dirty_and_save" \
    | rg -v "^crates/app/src/workspace/(mod|durable)\.rs:" \
    | rg -v "^\S+:\d+:\s*(//[!/]?|///?)") || true

if [ -n "$offenders" ]; then
    echo "Direct mark_dirty_and_save calls outside durable.rs / workspace/mod.rs definition:" >&2
    echo "$offenders" >&2
    exit 1
fi
echo "✓ No direct mark_dirty_and_save calls outside the durable wrappers."
