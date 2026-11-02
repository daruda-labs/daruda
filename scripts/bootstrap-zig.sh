#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/.context/zig"
VERSION="0.17.0"

mkdir -p "${OUT_DIR}"

os="$(uname -s)"
arch="$(uname -m)"

# Normalize to the names ziglang.org uses in its filenames.
case "${os}" in
  Darwin) zig_os="macos" ;;
  Linux) zig_os="linux" ;;
  MINGW*|MSYS*|CYGWIN*)
    echo "Windows is not covered by this script." >&2
    echo "Its Zig build ships as a .zip, and Git Bash does not reliably" >&2
    echo "carry an extractor for one. Install Zig ${VERSION} yourself and" >&2
    echo "put it on PATH, or point ZIG at the binary." >&2
    exit 1
    ;;
  *)
    echo "Unsupported OS: ${os}" >&2
    exit 1
    ;;
esac

case "${arch}" in
  x86_64|amd64) zig_arch="x86_64" ;;
  arm64|aarch64) zig_arch="aarch64" ;;
  *)
    echo "Unsupported arch: ${arch}" >&2
    exit 1
    ;;
esac

# From https://ziglang.org/download/index.json for ${VERSION}.
case "${zig_arch}-${zig_os}" in
  x86_64-macos)
    expected_sha="4f9a1c5269aa17ebda5e6d3c2b89d6cbf36f7d2b22a0306e9ab98f25f95529c6"
    ;;
  aarch64-macos)
    expected_sha="b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a"
    ;;
  x86_64-linux)
    expected_sha="1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026"
    ;;
  aarch64-linux)
    expected_sha="9e8d11661d4ae3bd57702a3832781e23ad151dde5798e16a5ccd503f65234ff8"
    ;;
  *)
    # Unreachable while the two cases above cover both normalized values —
    # but `set -u` would otherwise report this as an unbound variable three
    # lines later, naming the wrong problem.
    echo "No published Zig ${VERSION} build for ${zig_arch}-${zig_os}" >&2
    exit 1
    ;;
esac

tarball="zig-${zig_arch}-${zig_os}-${VERSION}.tar.xz"
url="https://ziglang.org/download/${VERSION}/${tarball}"

dest="${OUT_DIR}/${VERSION}"
if [[ -x "${dest}/zig" ]]; then
  echo "Zig ${VERSION} already installed, skipping download"
  ln -sfn "${dest}/zig" "${OUT_DIR}/zig"
  exit 0
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

archive="${tmp_dir}/${tarball}"

echo "Downloading ${url}"
curl -fsSL -o "${archive}" "${url}"

# macOS ships `shasum`, most Linux images ship `sha256sum`, and neither
# is guaranteed to be the other.
if command -v shasum >/dev/null 2>&1; then
  echo "${expected_sha}  ${archive}" | shasum -a 256 -c -
else
  echo "${expected_sha}  ${archive}" | sha256sum -c -
fi

rm -rf "${dest}"
mkdir -p "${dest}"

tar -xf "${archive}" -C "${dest}" --strip-components=1

ln -sfn "${dest}/zig" "${OUT_DIR}/zig"

echo "Installed Zig ${VERSION} to ${OUT_DIR}/zig"
