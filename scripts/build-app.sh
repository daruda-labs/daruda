#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Read version from [workspace.package] — single source of truth.
DARUDA_VERSION=$(grep -A20 '^\[workspace\.package\]' Cargo.toml \
  | grep '^version' \
  | head -1 \
  | sed 's/version = "\(.*\)"/\1/')

echo "Building daruda ${DARUDA_VERSION} (release)..."
cargo build -p daruda --release

APP_DIR="target/release/daruda.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp target/release/daruda "$APP_DIR/Contents/MacOS/daruda"

ICON_SRC="$ROOT_DIR/assets/icon.icns"
if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$APP_DIR/Contents/Resources/icon.icns"
fi

# Inject version into Info.plist template.
# @@DARUDA_VERSION@@ avoids conflicts with shell variable syntax in the file.
sed "s/@@DARUDA_VERSION@@/${DARUDA_VERSION}/g" \
  scripts/Info.plist.tmpl \
  > "$APP_DIR/Contents/Info.plist"

# Ad-hoc sign so Gatekeeper shows "unidentified developer" (right-click → Open
# works) instead of "damaged and can't be opened". No Apple Developer ID needed.
codesign --force --deep --sign - "$APP_DIR"

echo ""
echo "Built:   $APP_DIR"
echo "Version: $DARUDA_VERSION"
echo "Size:    $(du -sh "$APP_DIR" | cut -f1)"
echo ""
echo "Run:     open $APP_DIR"
echo "DMG:     ./scripts/build-dmg.sh"
