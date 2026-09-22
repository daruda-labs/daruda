#!/usr/bin/env bash
# Lint: the Landing view must not read the recent list from disk.
#
# Background: GPUI has no partial redraw, so anything `render` does is
# paid on every frame of the whole window tree. `load_recent_in` opens
# and parses a JSON file. The recent list therefore reaches the Landing
# view through `menus::RecentSnapshot`, a global written at the one
# place the menu bar is set — the same write that already loads the list.
#
# Rule: crates/app/src/workspace/render/landing.rs may not name
# `load_recent_in` or any other `daruda_store::project::load_*` /
# `std::fs` entry point. It reads `RecentSnapshot` instead.
#
# This cannot be a type-level guard: `load_recent_in` is a plain public
# function and `render` has an `App` in scope, so calling it compiles.
#
# See AGENTS.md "Pitfall 10 — Render-cost containment".
#
# Usage:   scripts/lint-landing-no-disk-read.sh
# Exit:    0 — clean   1 — a disk read reached the Landing render path

set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="crates/app/src/workspace/render/landing.rs"

if [[ ! -f "$TARGET" ]]; then
  echo "lint-landing-no-disk-read: $TARGET is missing" >&2
  exit 1
fi

# Strip comments so a doc comment explaining the ban is not itself a hit.
BODY="$(sed -e 's://.*::' "$TARGET")"

FAILED=0
for pattern in 'load_recent_in' 'load_workspace_state_in' 'load_project_state_in' 'std::fs' 'fs::read' 'File::open'; do
  if grep -q -- "$pattern" <<<"$BODY"; then
    echo "lint-landing-no-disk-read: $TARGET names '$pattern' — render must not touch the disk." >&2
    echo "  Read the recent list from crate::menus::RecentSnapshot instead." >&2
    FAILED=1
  fi
done

# The positive half: the snapshot is how it is supposed to get the list.
# Without this, deleting the recent section entirely would pass silently.
if ! grep -q 'RecentSnapshot' <<<"$BODY"; then
  echo "lint-landing-no-disk-read: $TARGET no longer reads RecentSnapshot." >&2
  echo "  If the recent list moved, update this guard to match." >&2
  FAILED=1
fi

if [[ "$FAILED" -ne 0 ]]; then
  exit 1
fi

echo "lint-landing-no-disk-read: clean"
