#!/usr/bin/env bash
# Refresh the committed ACP registry snapshot from the live registry, then
# regenerate the preset block in `daruda_config` from it.
#
# Division of duties: this script is the only network path — it answers "is the
# snapshot still fresh?". `cargo run -p gen_acp_presets -- --check` never
# touches the network and answers a different question: "do the committed
# snapshot and the committed preset block still agree?".
#
# Usage:
#   scripts/sync-acp-registry.sh
#
# Exit codes:
#   0 — snapshot and preset block updated (or already current)
#   1 — fetch failed, or the response could not be generated from

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PRESET_FILE="crates/daruda_config/src/agent/preset.rs"
SNAPSHOT="tools/gen_acp_presets/registry-snapshot.json"

# The URL lives with the presets it seeds, so there is only one copy of it.
URL="$(grep -A1 'pub const ACP_REGISTRY_URL' "$PRESET_FILE" | grep -o 'https://[^"]*' | head -1)"
if [ -z "$URL" ]; then
    echo "Could not read ACP_REGISTRY_URL from $PRESET_FILE" >&2
    exit 1
fi

TMP="$(mktemp -t acp-registry)"
trap 'rm -f "$TMP"' EXIT

echo "fetching $URL"
curl -fsSL "$URL" -o "$TMP"

# Generate from the download before replacing the snapshot: a truncated or
# non-JSON response fails here instead of landing in the committed file.
cargo run -q -p gen_acp_presets -- --input "$TMP"
cp "$TMP" "$SNAPSHOT"
echo "updated $SNAPSHOT"

cargo run -q -p gen_acp_presets -- --check
