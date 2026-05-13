#!/usr/bin/env bash
# Lint: file-size budget warnings (CLAUDE.md G1).
#
# Background: `CLAUDE.md` §"File-structure rules" + `app/src/CLAUDE.md`
# G1 codify these budgets. They are advisory, not hard limits — a single
# huge file is sometimes the right answer (e.g. const tables, generated
# code). This lint surfaces files that have grown past the budget so a
# reviewer notices, but never fails the build by default. Pass
# `--strict` to flip to exit-1 (used by CI gating PRs that explicitly
# opt in).
#
# Budgets:
#   - mod.rs / regular .rs       : 800 lines  (CLAUDE.md G1, app G1)
#   - tests.rs / test files      : 2000 lines (tests dominate; G1 carve-out)
#   - ux/theme.rs                : 2200 lines (pure const tables)
#   - surface/strings.rs         : 1100 lines (pure const tables)
#   - vendored / generated       : skipped
#
# Files explicitly waived (long const tables that will be re-evaluated
# only when their domain becomes user-tunable):
#   - crates/daruda_terminal/src/ux/theme.rs
#   - crates/daruda_terminal/src/ux/strings.rs (data + constants)
#   - crates/app/src/surface/strings.rs
#
# Usage:
#   scripts/lint-file-size.sh           # warn-only (exit 0 even on hits)
#   scripts/lint-file-size.sh --strict  # exit 1 on any over-budget file
#
# Exit codes:
#   0 — no over-budget files (or warn-only mode)
#   1 — at least one over-budget file (only with --strict)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STRICT=0
if [ "${1-}" = "--strict" ]; then
    STRICT=1
fi

DEFAULT_BUDGET=800
TEST_BUDGET=2000

# Per-path overrides — pure-data files where the budget doesn't apply.
# Match by suffix (file path ends with the listed string).
WAIVED_PATHS=(
    "crates/daruda_terminal/src/ux/theme.rs"
    "crates/daruda_terminal/src/ux/strings.rs"
    "crates/app/src/surface/strings.rs"
)

# Skip directories — vendored crates and target builds aren't ours.
SKIP_DIRS=(
    "crates/gpui_component"
    "crates/gpui_component_assets"
    "crates/gpui_component_macros"
    "crates/ghostty_vt"
    "crates/ghostty_vt_sys"
    "vendor"
    "target"
)

is_waived() {
    local path="$1"
    for w in "${WAIVED_PATHS[@]}"; do
        if [ "$path" = "$w" ]; then
            return 0
        fi
    done
    return 1
}

is_in_skip_dir() {
    local path="$1"
    for d in "${SKIP_DIRS[@]}"; do
        case "$path" in
            "$d"/*) return 0 ;;
        esac
    done
    return 1
}

budget_for() {
    local path="$1"
    case "$path" in
        */tests.rs|*/tests/*.rs) echo $TEST_BUDGET ;;
        *) echo $DEFAULT_BUDGET ;;
    esac
}

over_budget=0
hits=()

while IFS= read -r f; do
    if is_in_skip_dir "$f" || is_waived "$f"; then
        continue
    fi
    lines=$(wc -l <"$f" | tr -d ' ')
    budget=$(budget_for "$f")
    if [ "$lines" -gt "$budget" ]; then
        hits+=("$f: $lines lines (budget $budget)")
        over_budget=1
    fi
done < <(find crates -name '*.rs' -type f | sort)

if [ ${#hits[@]} -eq 0 ]; then
    echo "[lint-file-size] OK — all .rs files within budget."
    exit 0
fi

echo "[lint-file-size] over-budget files (CLAUDE.md G1):"
printf '  %s\n' "${hits[@]}"
echo
echo "Action: extract by responsibility, or convert tests.rs to tests/ dir."
echo "Add a path to WAIVED_PATHS in this script if the file is pure-data"
echo "(const tables) and the domain is not user-tunable."

if [ "$STRICT" -eq 1 ]; then
    exit 1
fi
echo
echo "(warn-only mode — pass --strict to fail.)"
exit 0
