#!/usr/bin/env bash
# Apply GPUI IME patches to the cargo git checkout.
#
# Run this once after `cargo fetch` or whenever the cargo cache is cleared.
# The patch routes non-ASCII key_char (Korean jamo, Japanese kana, etc.)
# through macOS IME-first dispatch (PATH A) so composition works reliably
# even during IMK Mach Port initialization delays.
#
# Usage:
#   ./scripts/apply-gpui-patch.sh          # auto-detect checkout path
#   ./scripts/apply-gpui-patch.sh <path>   # explicit checkout path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
PATCH_FILE="$REPO_DIR/patches/gpui-ime-cjk-path-a.patch"

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
TARGET_FILE="$CHECKOUT/crates/gpui/src/platform/mac/window.rs"

if [ ! -f "$TARGET_FILE" ]; then
    echo "ERROR: Target file not found: $TARGET_FILE" >&2
    exit 1
fi

# Check if patch is already applied
if grep -q 'has_non_ascii_key_char' "$TARGET_FILE" 2>/dev/null; then
    echo "✓ GPUI IME patch already applied."
    exit 0
fi

# Apply the patch
echo "Applying GPUI IME patch to: $CHECKOUT"
cd "$CHECKOUT"
if git apply --check "$PATCH_FILE" 2>/dev/null; then
    git apply "$PATCH_FILE"
    echo "✓ GPUI IME patch applied successfully."
else
    # Fallback: direct sed-based patching for when git apply fails
    echo "git apply failed, trying direct patch..."
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
}' "$TARGET_FILE"; then
        rm -f "${TARGET_FILE}.bak"
        echo "✓ GPUI IME patch applied (direct)."
    else
        echo "ERROR: Failed to apply patch." >&2
        exit 1
    fi
fi

# Force cargo to recompile gpui
echo "Cleaning gpui build artifacts..."
find "$REPO_DIR/target/debug" -name "libgpui-*" -type f -delete 2>/dev/null || true
find "$REPO_DIR/target/debug" -name "gpui-*" -type f -delete 2>/dev/null || true
echo "✓ Done. Run 'cargo build -p daruda' to rebuild."
