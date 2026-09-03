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
# Reading someone else's build output is inherently brittle, so the failure
# modes are deliberate: a package with no AIR constants at all is not a
# participant and is skipped in silence, but one that *is* a participant and
# still exposes no recognizable capability constant fails — that is exactly the
# upstream-rename case this exists to catch. Same on our side: an unreadable
# constant fails rather than comparing an empty string against everything.
#
# Local/reviewer check: it reads the installed adapter bundles, so it cannot run
# in CI. Skips (exit 0) with a message when no adapter is installed.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

src=crates/daruda_acp/src/native_subagents.rs

# Every extraction may legitimately find nothing, which `pipefail` would turn
# into an abort under `-e`; emptiness is judged explicitly instead.
rust_num() { grep -oE "$1: u64 = [0-9]+" "$src" | grep -oE '[0-9]+$' | head -1 || true; }
rust_str() { grep -oE "$1: &str = \"[^\"]+\"" "$src" | sed 's/.*"\(.*\)"/\1/' | head -1 || true; }

ours_version=$(rust_num AIR_EXTENSION_VERSION)
ours_capability=$(rust_str NATIVE_SUBAGENT_SESSIONS_CAPABILITY)
ours_jetbrains=$(rust_str JETBRAINS_META_KEY)
ours_air=$(rust_str AIR_META_KEY)
ours_version_key=$(rust_str AIR_EXTENSION_VERSION_KEY)
ours_capabilities_key=$(rust_str AIR_EXTENSION_CAPABILITIES_KEY)

unreadable=()
for pair in "AIR_EXTENSION_VERSION=$ours_version" \
            "NATIVE_SUBAGENT_SESSIONS_CAPABILITY=$ours_capability" \
            "JETBRAINS_META_KEY=$ours_jetbrains" \
            "AIR_META_KEY=$ours_air" \
            "AIR_EXTENSION_VERSION_KEY=$ours_version_key" \
            "AIR_EXTENSION_CAPABILITIES_KEY=$ours_capabilities_key"; do
  [ -n "${pair#*=}" ] || unreadable+=("${pair%%=*}")
done
if [ ${#unreadable[@]} -ne 0 ]; then
  echo "✗ Could not read from $src: ${unreadable[*]}"
  echo "  Those constants moved or were renamed, so this lint would compare"
  echo "  an empty string against the adapter and pass on anything."
  exit 1
fi

# `-print0` because one of the two roots contains a space.
dists=()
while IFS= read -r -d '' dist; do dists+=("$dist"); done < <(
  find "$HOME/.npm/_npx" "$HOME/Library/Application Support/daruda/node/npx-cache" \
    -type d -path "*-acp/dist" -print0 2>/dev/null | sort -zu
)

if [ ${#dists[@]} -eq 0 ]; then
  echo "· No ACP adapter installed — AIR gate contract not checked."
  exit 0
fi

fail=0
checked=0
seen=""

# The constants are spread across a package's `dist/` (claude-agent-acp keeps
# them in `air-extension.js`, codex-acp inlines them in `index.js`), so the
# whole tree is searched rather than one entry file.
their_num() { grep -rhoE "$1 = [0-9]+" "$dist" --include='*.js' | grep -oE '[0-9]+$' | head -1 || true; }
their_str() { grep -rhoE "$1 = \"[^\"]+\"" "$dist" --include='*.js' | sed 's/.*"\(.*\)"/\1/' | head -1 || true; }

compare() { # label ours theirs
  if [ -n "$3" ] && [ "$2" != "$3" ]; then
    echo "  ✗ $name $1 is \"$3\", daruda sends \"$2\""
    fail=1
  fi
  return 0
}

for dist in "${dists[@]}"; do
  # A package with no AIR block at all (zed's claude-code-acp) is not party to
  # this contract; saying so on every run would be noise.
  their_air=$(their_str AIR_META_KEY)
  [ -n "$their_air" ] || continue

  pkg=$(dirname "$dist")
  name=$(basename "$pkg")
  version=$(grep -oE '"version" *: *"[^"]+"' "$pkg/package.json" 2>/dev/null |
    head -1 | sed 's/.*"\(.*\)"/\1/' || true)
  name="$name@${version:-unknown}"
  # The same package resolves into both npx caches; check it once.
  case "$seen" in *"|$name|"*) continue ;; esac
  seen="$seen|$name|"
  checked=$((checked + 1))

  # Both spellings: codex says `..._KEY`, claude-agent-acp says `..._CAPABILITY`.
  their_capability=$(their_str 'AIR_NATIVE_SUBAGENT_SESSIONS_(KEY|CAPABILITY)')
  if [ -z "$their_capability" ]; then
    echo "  ✗ $name carries an AIR block but no recognizable"
    echo "    AIR_NATIVE_SUBAGENT_SESSIONS_* constant — renamed upstream, or the"
    echo "    build stopped emitting readable identifiers. Re-read the adapter."
    fail=1
    continue
  fi

  their_version=$(their_num AIR_EXTENSION_VERSION)
  # The adapter accepts `>=`, so a daruda version below theirs is what closes
  # the gate; a daruda version above is fine.
  if [ -n "$their_version" ] && [ "$ours_version" -lt "$their_version" ]; then
    echo "  ✗ $name wants AIR version >= $their_version, daruda sends $ours_version"
    fail=1
  fi

  compare "capability key" "$ours_capability" "$their_capability"
  compare "jetbrains meta key" "$ours_jetbrains" "$(their_str JETBRAINS_META_KEY)"
  compare "air meta key" "$ours_air" "$their_air"
  compare "version field" "$ours_version_key" "$(their_str AIR_EXTENSION_VERSION_KEY)"
  compare "capabilities field" "$ours_capabilities_key" \
    "$(their_str AIR_EXTENSION_CAPABILITIES_KEY)"
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
