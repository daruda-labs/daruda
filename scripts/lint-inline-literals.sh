#!/usr/bin/env bash
# Lint for inline color / pixel literals in production source files.
#
# Background: every UI color and pixel size must live in
# `daruda_terminal::ux::theme` (colors + pixel metrics) or
# `crate::surface::*` (app-shell text/keybindings). Inline values like
# `gpui::white()` and `px(2.0)` drift away from theming and break the
# moment a future theme/dpi change is wired up.
#
# This script grep-walks `crates/` and fails when any tracked file
# introduces a banned literal *outside* the few definition sites that
# are allowed to use them. See CLAUDE.md G4 for the rule + exceptions.
#
# Anything inside an `#[cfg(test)]` block is excluded — tests
# legitimately use synthetic pixel sizes and concrete colors.
#
# Usage:
#   scripts/lint-inline-literals.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Files allowed to contain raw color / pixel / string literals — they
# *are* the constant definitions everyone else references.
WHITELIST=(
    "crates/daruda_terminal/src/ux/theme.rs"
    "crates/daruda_terminal/src/ux/strings.rs"
    "crates/app/src/surface/constants.rs"
    "crates/app/src/surface/keybindings.rs"
    "crates/app/src/surface/strings.rs"
    # ui/theme/ is the daruda → gpui_component palette bridge plus the
    # app-side UI palette (workspace chrome, sidebar, status bar, etc.).
    # `mod.rs` carries variant-derived hsla values (danger_hover at
    # l=0.60 etc.) next to the slot they map to. `palette.rs` carries
    # the workspace-chrome constants that used to live in
    # daruda_terminal/ux/theme.rs.
    "crates/app/src/ui/theme/mod.rs"
    "crates/app/src/ui/theme/palette.rs"
)

is_whitelisted() {
    local file="$1"
    for w in "${WHITELIST[@]}"; do
        [[ "$file" == "$w" ]] && return 0
    done
    # Tests are exempt — they often need synthetic concrete values.
    [[ "$file" == *"/tests/"* ]] && return 0
    [[ "$file" == *"tests.rs" ]] && return 0
    # Dedicated test crates — every file is a fixture, pixel coordinates
    # in `simulate_click(point(px(4.0), px(6.0)))` are load-bearing test
    # input rather than styling literals.
    [[ "$file" == crates/visual_tests/* ]] && return 0
    # Vendored upstream crates (longbridge/gpui-component, Apache-2.0).
    # Their inline px / color literals are part of the upstream design
    # and intentionally not routed through daruda's theme module.
    [[ "$file" == crates/gpui_component/* ]] && return 0
    [[ "$file" == crates/gpui_component_macros/* ]] && return 0
    return 1
}

violations=0

# Collect tracked source files into a portable array (no `mapfile` —
# macOS bash 3.2 doesn't ship it).
FILES=()
while IFS= read -r line; do
    FILES+=("$line")
done < <(git ls-files 'crates/**/*.rs')

for file in "${FILES[@]}"; do
    if is_whitelisted "$file"; then
        continue
    fi

    # Perl one-liner: stop scanning at the first `#[cfg(test)]` so
    # in-file test modules don't trip the lint. Matches:
    #   gpui::white() / black() / red() / green() / blue() / yellow()
    #     / transparent_black()  — base colors that should be theme consts
    #   hsla(<numeric>...)        — inline color definition outside theme
    #   px(<numeric>)             — pixel literal outside theme
    #
    # Allowlist (structural / arithmetic, not styling):
    #   px(0) / px(0.0) / px(0.) — zero clamps and defaults
    #   gpui::black().opacity(<UPPER_IDENT>) /
    #   gpui::white().opacity(<UPPER_IDENT>)  — already-named alpha is
    #     the documented exception in CLAUDE.md G4
    output=$(perl -ne '
        if (/^\s*#\[cfg\(test\)\]/) { last; }
        my $color = /gpui::(?:white|black|red|green|blue|yellow|transparent_black)\(\)/;
        my $color_named_alpha = /gpui::(?:white|black|red|green|blue|yellow|transparent_black)\(\)\.opacity\(\s*[a-zA-Z_][A-Za-z0-9_:]*\s*\)/;
        my $hsla = /\bhsla\(\s*-?[0-9]/;
        my $px = /\bpx\(\s*-?[0-9][0-9_.]*\s*\)/;
        my $px_zero = /\bpx\(\s*-?0(?:\.[0-9_]*)?\s*\)/;
        if (($color && !$color_named_alpha)
            || $hsla
            || ($px && !$px_zero)) {
            chomp;
            print "  $ARGV:$.: $_\n";
        }
    ' "$file" 2>/dev/null) || true

    if [[ -n "$output" ]]; then
        echo "$output"
        violations=$((violations + 1))
    fi
done

if (( violations > 0 )); then
    echo
    echo "✗ Inline-literal lint failed: $violations file(s) with hits."
    echo "  See CLAUDE.md G4. Hoist values to daruda_terminal::ux::theme"
    echo "  (colors + pixel metrics) or crate::surface::* (app-shell"
    echo "  strings/keybindings) and reference the named constant."
    exit 1
fi

echo "✓ No inline color/pixel literals found in tracked source."
