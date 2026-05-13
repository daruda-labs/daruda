#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_DIR="target/release/daruda.app"
if [ ! -d "$APP_DIR" ]; then
    echo "Error: $APP_DIR not found. Run ./scripts/build-app.sh first."
    exit 1
fi

if ! command -v create-dmg &>/dev/null; then
    echo "Error: create-dmg not found. Run: brew install create-dmg"
    exit 1
fi

DARUDA_VERSION=$(grep -A20 '^\[workspace\.package\]' Cargo.toml \
  | grep '^version' \
  | head -1 \
  | sed 's/version = "\(.*\)"/\1/')

DMG_NAME="daruda-${DARUDA_VERSION}.dmg"
DMG_OUT="target/release/${DMG_NAME}"

rm -f "$DMG_OUT"

create-dmg \
    --volname "daruda" \
    --window-size 540 380 \
    --icon-size 128 \
    --icon "daruda.app" 150 180 \
    --app-drop-link 390 180 \
    "$DMG_OUT" \
    "$APP_DIR"

echo ""
echo "DMG:     $DMG_OUT"
echo "Version: $DARUDA_VERSION"
echo "Size:    $(du -sh "$DMG_OUT" | cut -f1)"
echo ""
echo "Release: gh release create v${DARUDA_VERSION} \"${DMG_OUT}\" --title \"daruda v${DARUDA_VERSION}\""
