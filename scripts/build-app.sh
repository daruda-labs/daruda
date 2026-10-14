#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PROFILE=release
SKIP_BUILD=0
while [ $# -gt 0 ]; do
    case "$1" in
        --profile) PROFILE="${2:?--profile requires release or debug}"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        *) echo "usage: $0 [--profile release|debug] [--skip-build]" >&2; exit 2 ;;
    esac
done
case "$PROFILE" in release | debug) ;; *) echo "unknown profile: $PROFILE" >&2; exit 2 ;; esac

DARUDA_VERSION=$(./scripts/version.sh)
TARGET_DIR=$(cargo metadata --locked --no-deps --format-version 1 | jq -er '.target_directory')

if [ "$SKIP_BUILD" -eq 0 ]; then
    echo "Building daruda ${DARUDA_VERSION} ($PROFILE)..."
    if [ "$PROFILE" = release ]; then
        cargo build --locked -p daruda --release
    else
        cargo build --locked -p daruda
    fi
fi
BINARY="$TARGET_DIR/$PROFILE/daruda"
[ -x "$BINARY" ] || { echo "Build output is missing: $BINARY" >&2; exit 1; }

APP_DIR="$TARGET_DIR/$PROFILE/daruda.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$BINARY" "$APP_DIR/Contents/MacOS/daruda"

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
if [ "$PROFILE" = release ]; then
    echo "DMG:     ./scripts/build-dmg.sh"
fi
