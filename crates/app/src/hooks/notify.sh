#!/usr/bin/env bash
# daruda Claude Code hook notifier — extracted on first install by
# `app/src/hooks/installer.rs`. Forwards the hook event (with the
# original JSON payload on stdin) to whichever `daruda` binary is
# resolvable, then exits.
#
# The wrapper exists so that the absolute path written into
# `~/.claude/settings.json` is stable even when the daruda app bundle
# moves, the user installs a new release, or the binary is invoked
# from a different location on a different machine that shares the
# same dotfiles.
set -euo pipefail

EVENT="${1:-unknown}"

# Resolution order: explicit override → PATH → standard .app bundle.
# The script silently no-ops if none of these resolve, so a missing
# daruda install never blocks Claude Code.
DARUDA_BIN="${DARUDA_BIN:-}"
if [ -z "$DARUDA_BIN" ]; then
  if command -v daruda >/dev/null 2>&1; then
    DARUDA_BIN="$(command -v daruda)"
  elif [ -x "/Applications/Daruda.app/Contents/MacOS/daruda" ]; then
    DARUDA_BIN="/Applications/Daruda.app/Contents/MacOS/daruda"
  else
    exit 0
  fi
fi

exec "$DARUDA_BIN" --hook "$EVENT" 2>/dev/null || true
