#!/usr/bin/env bash
# Lint: daruda's AIR capability must still match what the installed ACP
# adapters gate on.
#
# Background: native subagent sessions are switched on by a vendor-private
# `_meta.jetbrains.air` block (see `daruda_acp::native_subagents`). The adapter
# decides with `version >= ITS OWN constant` plus an exact capability-key match,
# and every preset pins `@latest` (a deliberate policy — see
# `daruda_config::agent::preset`). So an upstream release can bump that constant
# or rename the key and the gate closes **silently**: the connection survives,
# the capability is simply ignored, and a subagent's work stops appearing with
# no error anywhere.
#
# Nothing else catches that. The unit test
# `client_capabilities_claim_native_subagent_sessions_in_the_shape_the_gate_reads`
# pins what daruda *sends*; this pins what the adapter *expects*.
#
# Local/reviewer check: it reads the installed adapter bundles, so it cannot run
# in CI. Skips (exit 0) with a message when no adapter is installed.
set -uo pipefail
cd "$(dirname "$0")/.."

src=crates/daruda_acp/src/native_subagents.rs
ours_version=$(grep -oE 'AIR_EXTENSION_VERSION: u64 = [0-9]+' "$src" | grep -oE '[0-9]+$')
ours_key=$(grep -oE 'NATIVE_SUBAGENT_SESSIONS_CAPABILITY: &str = "[^"]+"' "$src" | sed 's/.*"\(.*\)"/\1/')
ours_jetbrains=$(grep -oE 'JETBRAINS_META_KEY: &str = "[^"]+"' "$src" | sed 's/.*"\(.*\)"/\1/')
ours_air=$(grep -oE 'AIR_META_KEY: &str = "[^"]+"' "$src" | sed 's/.*"\(.*\)"/\1/')

bundles=$(find "$HOME/.npm/_npx" "$HOME/Library/Application Support/daruda/node/npx-cache" \
  -path "*-acp/dist/index.js" 2>/dev/null | sort -u)

if [ -z "$bundles" ]; then
  echo "· No ACP adapter installed — AIR gate contract not checked."
  exit 0
fi

fail=0
checked=0
for bundle in $bundles; do
  grep -q "AIR_NATIVE_SUBAGENT_SESSIONS_KEY" "$bundle" 2>/dev/null || continue
  checked=$((checked + 1))
  name=$(basename "$(dirname "$(dirname "$bundle")")")
  their_version=$(grep -oE 'AIR_EXTENSION_VERSION = [0-9]+' "$bundle" | head -1 | grep -oE '[0-9]+$')
  their_key=$(grep -oE 'AIR_NATIVE_SUBAGENT_SESSIONS_KEY = "[^"]+"' "$bundle" | head -1 | sed 's/.*"\(.*\)"/\1/')
  their_jetbrains=$(grep -oE 'JETBRAINS_META_KEY = "[^"]+"' "$bundle" | head -1 | sed 's/.*"\(.*\)"/\1/')
  their_air=$(grep -oE 'AIR_META_KEY = "[^"]+"' "$bundle" | head -1 | sed 's/.*"\(.*\)"/\1/')

  # The adapter accepts `>=`, so a daruda version below theirs is what closes
  # the gate; a daruda version above is fine.
  if [ -n "$their_version" ] && [ "$ours_version" -lt "$their_version" ]; then
    echo "  ✗ $name wants AIR version >= $their_version, daruda sends $ours_version"
    fail=1
  fi
  for pair in "capability key:$ours_key:$their_key" \
              "jetbrains meta key:$ours_jetbrains:$their_jetbrains" \
              "air meta key:$ours_air:$their_air"; do
    label=${pair%%:*}; rest=${pair#*:}; ours=${rest%%:*}; theirs=${rest#*:}
    if [ -n "$theirs" ] && [ "$ours" != "$theirs" ]; then
      echo "  ✗ $name $label is \"$theirs\", daruda sends \"$ours\""
      fail=1
    fi
  done
done

if [ "$checked" -eq 0 ]; then
  echo "· No installed adapter carries the AIR gate — nothing to check."
  exit 0
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "✗ AIR gate contract drifted in $checked adapter bundle(s)."
  echo "  Native subagent sessions are silently off until"
  echo "  crates/daruda_acp/src/native_subagents.rs matches."
  exit 1
fi

echo "✓ AIR gate contract matches $checked installed adapter bundle(s)."
