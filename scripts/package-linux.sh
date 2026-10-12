#!/usr/bin/env bash
# Package the Linux build twice from one payload: a portable tar.gz that runs
# from wherever it is extracted, and a .deb that puts the binary on PATH and
# registers the desktop entry and icons the dock resolves.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE=release
SKIP_BUILD=0
while [ $# -gt 0 ]; do
    case "$1" in
        --profile) PROFILE="$2"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        *) echo "usage: $0 [--profile release|debug] [--skip-build]" >&2; exit 2 ;;
    esac
done
case "$PROFILE" in release | debug) ;; *) echo "unknown profile: $PROFILE" >&2; exit 2 ;; esac

for tool in jq convert dpkg-deb dpkg-shlibdeps; do
    command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done

VERSION=$(./scripts/version.sh)
TARGET_DIR=$(cargo metadata --locked --no-deps --format-version 1 | jq -er '.target_directory')

case "$(uname -m)" in
    x86_64) ARCH=x86_64; DEB_ARCH=amd64 ;;
    aarch64) ARCH=aarch64; DEB_ARCH=arm64 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [ "$SKIP_BUILD" -eq 0 ]; then
    if [ "$PROFILE" = release ]; then
        cargo build --locked -p daruda --release
    else
        cargo build --locked -p daruda
    fi
fi
BINARY="$TARGET_DIR/$PROFILE/daruda"
[ -x "$BINARY" ] || { echo "Build output is missing: $BINARY" >&2; exit 1; }

OUT="$TARGET_DIR/packages"
mkdir -p "$OUT"
SUFFIX=""
[ "$PROFILE" = debug ] && SUFFIX="-debug"
NAME="daruda-$VERSION-linux-$ARCH$SUFFIX"
STAGE=$(mktemp -d "$OUT/.staging-XXXXXX")
trap 'rm -rf "$STAGE"' EXIT

ICON_SIZES=(16 32 48 64 128 256 512)
icon_path() { echo "icons/hicolor/$1x$1/apps/daruda.png"; }

# One payload, laid out the way the tar.gz ships it; the deb re-roots it.
PAYLOAD="$STAGE/payload"
install -Dm755 "$BINARY" "$PAYLOAD/daruda"
install -Dm644 scripts/linux/daruda.desktop "$PAYLOAD/daruda.desktop"
# A single 1024px PNG is ignored by some docks; hicolor wants each size.
for size in "${ICON_SIZES[@]}"; do
    install -d "$PAYLOAD/$(dirname "$(icon_path "$size")")"
    convert assets/icon.png -resize "${size}x${size}" "$PAYLOAD/$(icon_path "$size")"
done
install -Dm644 LICENSE "$PAYLOAD/LICENSE"
cp -R LICENSES "$PAYLOAD/LICENSES"

# --- portable archive ------------------------------------------------------
BUNDLE="$STAGE/$NAME"
cp -R "$PAYLOAD" "$BUNDLE"
cat >"$BUNDLE/README.txt" <<EOF
daruda $VERSION ($PROFILE, Linux $ARCH)

Extract the archive anywhere and run ./daruda. Drawing needs a Vulkan driver
(mesa-vulkan-drivers on Debian and Ubuntu) or an EGL-capable GL driver.
To add a launcher entry, copy daruda.desktop to ~/.local/share/applications/
with Exec= pointing at this binary, and icons/ into ~/.local/share/icons/.
The .deb from the same release does all of that for you.
Install git to use repositories, shell flows, and ACP agents.
Source and build instructions: https://github.com/daruda-labs/daruda
EOF
ARCHIVE="$OUT/$NAME.tar.gz"
tar -C "$STAGE" --owner=0 --group=0 --numeric-owner -czf "$ARCHIVE" "$NAME"
echo "Packaged: $ARCHIVE"

# --- deb -------------------------------------------------------------------
DEB="$STAGE/deb"
install -Dm755 "$BINARY" "$DEB/usr/bin/daruda"
install -Dm644 scripts/linux/daruda.desktop "$DEB/usr/share/applications/daruda.desktop"
for size in "${ICON_SIZES[@]}"; do
    install -Dm644 "$PAYLOAD/$(icon_path "$size")" "$DEB/usr/share/$(icon_path "$size")"
done
install -Dm644 LICENSE "$DEB/usr/share/doc/daruda/copyright"
cp -R LICENSES "$DEB/usr/share/doc/daruda/LICENSES"

# dpkg-shlibdeps reads the binary's NEEDED entries and names the packages
# that provide them, so Depends follows the link line rather than a list
# kept by hand. It insists on a debian/control beside it; an empty one does.
DEPENDS=$(
    cd "$DEB"
    mkdir -p debian && : >debian/control
    dpkg-shlibdeps -O usr/bin/daruda | sed -n 's/^shlibs:Depends=//p'
)
rm -rf "$DEB/debian"
[ -n "$DEPENDS" ] || { echo "dpkg-shlibdeps produced no dependency list" >&2; exit 1; }
# Opened at runtime, so no NEEDED entry names them: wgpu's Vulkan loader,
# and the Wayland client libraries gpui loads through wayland-backend's dlopen.
DEPENDS="$DEPENDS, libvulkan1, libwayland-client0, libwayland-cursor0"

# A Debian version has no `-`; a prerelease tag's becomes `~`, which also
# sorts before the release it precedes.
DEB_VERSION="${VERSION//-/\~}"
install -d "$DEB/DEBIAN"
cat >"$DEB/DEBIAN/control" <<EOF
Package: daruda
Version: $DEB_VERSION
Section: devel
Priority: optional
Architecture: $DEB_ARCH
Maintainer: daruda-labs <admin@daruda.ai>
Installed-Size: $(du -sk "$DEB/usr" | cut -f1)
Depends: $DEPENDS
Recommends: mesa-vulkan-drivers, git
Homepage: https://github.com/daruda-labs/daruda
Description: Run AI coding agents in parallel, each in its own git worktree
 daruda is a desktop workspace that runs several AI coding agents side by
 side, each attached to its own git worktree, with a terminal and an agent
 chat pane per worktree.
EOF
PACKAGE="$OUT/$NAME.deb"
dpkg-deb --build --root-owner-group "$DEB" "$PACKAGE" >/dev/null
echo "Packaged: $PACKAGE"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "archive=$ARCHIVE"
        echo "deb=$PACKAGE"
    } >>"$GITHUB_OUTPUT"
fi
