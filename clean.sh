#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "=== Cleaning build artifacts ==="

# Rust target/
for target_dir in "$ROOT/target" "$ROOT"/*/target; do
    if [ -d "$target_dir" ]; then
        size=$(du -sh "$target_dir" | cut -f1)
        echo "  Removing $target_dir ($size)"
        rm -rf "$target_dir"
    fi
done

# Node node_modules/
for nm_dir in "$ROOT/node_modules" "$ROOT"/*/node_modules; do
    if [ -d "$nm_dir" ]; then
        size=$(du -sh "$nm_dir" | cut -f1)
        echo "  Removing $nm_dir ($size)"
        rm -rf "$nm_dir"
    fi
done

# .DS_Store
count=$(find "$ROOT" -name .DS_Store | wc -l | tr -d ' ')
if [ "$count" -gt 0 ]; then
    echo "  Removing $count .DS_Store files"
    find "$ROOT" -name .DS_Store -delete
fi

echo ""
echo "Total after clean: $(du -sh "$ROOT" | cut -f1)"
echo "=== Done ==="
