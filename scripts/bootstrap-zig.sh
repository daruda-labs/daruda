#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/.context/zig"
VERSION="0.14.1"

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
    expected_sha="b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43"
    ;;
  aarch64-macos)
    expected_sha="39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa"
    ;;
  x86_64-linux)
    expected_sha="24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c"
    ;;
  aarch64-linux)
    expected_sha="f7a654acc967864f7a050ddacfaa778c7504a0eca8d2b678839c21eea47c992b"
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
