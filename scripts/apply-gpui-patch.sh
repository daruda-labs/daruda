#!/usr/bin/env bash
# Apply GPUI patches to the cargo git checkout.
#
# Run this once after `cargo fetch` or whenever the cargo cache is cleared.
#
# Patches:
#   gpui-ime-cjk-path-a.patch     — route non-ASCII key_char (Korean jamo,
#                                   Japanese kana, …) through macOS IME-first
#                                   dispatch (PATH A).
#   gpui-notify-lost-wakeup.patch — ensure cached AnyView always tracks itself
#                                   in accessed_entities so cx.notify() is not
#                                   silently dropped after an out-of-element
#                                   entity read.
#
# Usage:
#   ./scripts/apply-gpui-patch.sh          # auto-detect checkout path
#   ./scripts/apply-gpui-patch.sh <path>   # explicit checkout path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
IME_PATCH="$REPO_DIR/patches/gpui-ime-cjk-path-a.patch"
NOTIFY_PATCH="$REPO_DIR/patches/gpui-notify-lost-wakeup.patch"

# Resolve the GPUI source checkout path from Cargo.lock
find_gpui_checkout() {
    # Extract git rev from Cargo.toml
    local rev
    rev=$(grep -A2 'git.*zed-industries/zed.*gpui' "$REPO_DIR/Cargo.toml" \
        | grep -oE 'rev = "[a-f0-9]+"' \
        | head -1 \
        | sed 's/rev = "//;s/"//')

    if [ -z "$rev" ]; then
        echo "ERROR: Could not find GPUI git rev in Cargo.toml" >&2
        return 1
    fi

    # Cargo stores git checkouts under ~/.cargo/git/checkouts/<repo-hash>/<short-rev>/
    local short_rev="${rev:0:7}"
    local checkout
    checkout=$(find "$HOME/.cargo/git/checkouts" -maxdepth 2 -type d -name "${short_rev}*" 2>/dev/null | head -1)

    if [ -z "$checkout" ]; then
        echo "ERROR: GPUI checkout not found. Run 'cargo fetch' first." >&2
        return 1
    fi

    echo "$checkout"
}

CHECKOUT="${1:-$(find_gpui_checkout)}"
cd "$CHECKOUT"

# ---- gpui-ime-cjk-path-a.patch ----
IME_TARGET="$CHECKOUT/crates/gpui/src/platform/mac/window.rs"

# gpui >= 1.5.x split the macOS platform into the `gpui_macos` crate and added
# native IME-first dispatch for printable keys while a CJK input source is
# active (`is_ime_printable_key`, opt-in via `prefers_ime_for_printable_keys`).
# That supersedes this CJK PATH-A patch, so on the new layout we skip cleanly.
if [ -f "$IME_TARGET" ]; then
    if ! grep -q 'has_non_ascii_key_char' "$IME_TARGET" 2>/dev/null; then
        echo "Applying GPUI IME patch…"
        if git apply --check "$IME_PATCH" 2>/dev/null; then
            git apply "$IME_PATCH"
            echo "✓ GPUI IME patch applied successfully."
        else
            # Fallback: direct sed-based patching for when git apply fails
            echo "git apply failed for IME patch, trying direct patch…"
            if sed -i.bak '
/if is_composing$/{
    i\
            // Also route non-ASCII key_char (e.g. Korean jamo, Japanese kana)\
            // through the IME-first path so the input method can properly start\
            // composition instead of falling back to insertText.\
            let has_non_ascii_key_char = key_down_event\
                .keystroke\
                .key_char\
                .as_ref()\
                .map_or(false, |c| !c.is_ascii());
    s/if is_composing$/if is_composing\n                || has_non_ascii_key_char/
}' "$IME_TARGET"; then
                rm -f "${IME_TARGET}.bak"
                echo "✓ GPUI IME patch applied (direct)."
            else
                echo "ERROR: Failed to apply IME patch." >&2
                exit 1
            fi
        fi
    else
        echo "✓ GPUI IME patch already applied."
    fi
else
    NEW_MAC_WINDOW="$CHECKOUT/crates/gpui_macos/src/window.rs"
    if [ -f "$NEW_MAC_WINDOW" ] && grep -q 'is_ime_printable_key' "$NEW_MAC_WINDOW" 2>/dev/null; then
        echo "✓ gpui_macos provides native IME-first dispatch (is_ime_printable_key); CJK PATH-A patch not needed."
    else
        echo "NOTE: $IME_TARGET not found; skipping IME patch."
    fi
fi

# ---- gpui-notify-lost-wakeup.patch ----
NOTIFY_TARGET="$CHECKOUT/crates/gpui/src/view.rs"
if grep -q 'accessed_entities.insert(self.entity_id())' "$NOTIFY_TARGET" 2>/dev/null; then
    echo "✓ GPUI notify-lost-wakeup patch already applied."
else
    echo "Applying GPUI notify-lost-wakeup patch…"
    if git apply --check "$NOTIFY_PATCH" 2>/dev/null; then
        git apply "$NOTIFY_PATCH"
        echo "✓ GPUI notify-lost-wakeup patch applied successfully."
    else
        echo "ERROR: Failed to apply notify-lost-wakeup patch." >&2
        exit 1
    fi
fi

# Force cargo to recompile gpui
echo "Cleaning gpui build artifacts…"
find "$REPO_DIR/target/debug" -name "libgpui-*" -type f -delete 2>/dev/null || true
find "$REPO_DIR/target/debug" -name "gpui-*" -type f -delete 2>/dev/null || true
echo "✓ Done. Run 'cargo build -p daruda' to rebuild."
