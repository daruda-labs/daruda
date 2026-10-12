#!/usr/bin/env bash
# Install the packages listed in the given files, plus any named directly.
# Lists keep one package per line; `#` starts a comment.
set -euo pipefail
packages=()
for arg in "$@"; do
    if [ -f "$arg" ]; then
        while read -r package; do
            [ -n "$package" ] && packages+=("$package")
        done < <(sed 's/#.*//; s/[[:space:]]//g' "$arg")
    else
        packages+=("$arg")
    fi
done
sudo apt-get update
sudo apt-get install -y --no-install-recommends "${packages[@]}"
