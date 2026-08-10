#!/usr/bin/env bash
# Lint: forbid hand-rolled `daruda` / `.daruda` data-directory literals.
#
# Background: every on-disk file daruda itself writes and reads back
# across restarts (config.toml, hook status, activity cache, the
# Telegram bridge's Keychain service name, ...) must be profile-scoped
# via `daruda_store::persistence::default_data_dir()` — release keeps
# the unsuffixed path, debug/named profiles get `daruda-<profile>/`.
#
# Four separate places independently hardcoded `.join("daruda")` /
# `.join(".daruda")` instead of calling that function, so a debug or
# test run silently shared (and could corrupt) a real release install's
# state: `daruda_config::config_path`, `daruda_config::project::
# project_config_dir`, `daruda_agent::hooks::status_file::default_dir`,
# and `workspace::sync::limits::activity_paths`'s cache path. The last
# one (Telegram's Keychain-stored bot token) additionally caused two
# profiles to 409-conflict polling Telegram with the same token — see
# `crates/app/src/telegram/keychain.rs`'s `service_name` doc comment.
#
# This script does not (and cannot) catch the Keychain case — a service
# *name*, not a directory path — that risk is mitigated by routing
# through `daruda_store::persistence::profile_suffix()` instead; review
# any new Keychain/OS-credential-store integration against that pattern
# by hand.
#
# Anything from the first `#[cfg(test)]` line onward in a file is
# excluded (mirrors `lint-inline-literals.sh`) — tests legitimately
# build sample `~/git/daruda`-style paths (e.g. to exercise
# `redact_home`) that have nothing to do with app-state resolution.
#
# Usage:
#   scripts/lint-daruda-path-literals.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Files allowed to spell out the data-directory name literally — either
# they *are* the canonical implementation (persistence.rs / profile.rs /
# log_writer.rs), or the path is deliberately NOT app-instance state:
#   - task_edit_pane/{mod,task_edit_ops}.rs: `<worktree>/.daruda/task-*.md`
#     is a per-repo artifact tied to the branch, not app installation
#     state — every profile touching the same repo should share it.
#   - hooks/installer.rs: `~/.daruda/hooks/notify.sh` is a single global
#     hook script by design (Claude Code's settings.json only supports
#     one registration; the script self-resolves whichever daruda binary
#     is on PATH at hook-fire time regardless of profile).
#   - tasks/prompt_file.rs, workspace/flow_paths.rs: the per-repo
#     `.daruda/` directory that holds task prompts and flow definitions.
#     Committed with the repo and shared by every profile that opens it,
#     for the same reason as the `task-*.md` files above.
WHITELIST=(
    "crates/daruda_store/src/persistence.rs"
    "crates/daruda_store/src/profile.rs"
    "crates/daruda_store/src/observability/log_writer.rs"
    "crates/app/src/workspace/main_area/task_edit_pane/mod.rs"
    "crates/app/src/workspace/main_area/task_edit_pane/task_edit_ops.rs"
    "crates/app/src/hooks/installer.rs"
    "crates/daruda_store/src/tasks/prompt_file.rs"
    "crates/app/src/workspace/flow_paths.rs"
)

is_whitelisted() {
    local file="$1"
    for w in "${WHITELIST[@]}"; do
        [[ "$file" == "$w" ]] && return 0
    done
    return 1
}

SCAN_DIRS=(
    "crates/app/src"
    "crates/daruda_store/src"
    "crates/daruda_terminal/src"
    "crates/daruda_config/src"
    "crates/daruda_agent/src"
)

# Portable array population — no `mapfile` (macOS bash 3.2 doesn't ship it).
FILES=()
while IFS= read -r line; do
    FILES+=("$line")
done < <(find "${SCAN_DIRS[@]}" -name '*.rs' -type f)

violations=""
for file in "${FILES[@]}"; do
    if is_whitelisted "$file"; then
        continue
    fi
    # Dedicated test files (named `tests.rs`, or under a `tests/` dir)
    # are pure fixtures — synthetic paths like `tmp.join("daruda")` are
    # load-bearing test input, not app-state resolution (mirrors
    # lint-no-eprintln.sh's filename-based exclusion).
    case "$file" in
        */tests.rs|*/tests/*) continue ;;
    esac
    # Stop scanning at the first `#[cfg(test)]` line (mirrors
    # lint-inline-literals.sh) so in-file test modules never trip this.
    hit=$(perl -ne '
        if (/^\s*#\[cfg\(test\)\]/) { last; }
        # Both shapes: the literal inside a `.join(...)`, and the same
        # literal hoisted into a `const` — which used to slip past, and
        # is exactly what an author tidying up a path does.
        if (/\.join\("\.?daruda"\)/ || /"\.daruda"/) { print "$ARGV:$.:$_"; }
    ' "$file" || true)
    if [ -n "$hit" ]; then
        violations="${violations}${hit}"
    fi
done

if [ -n "$violations" ]; then
    echo "Hand-rolled daruda data-directory literal found — use"
    echo "daruda_store::persistence::default_data_dir() instead:"
    echo "$violations"
    echo
    echo "See the root CLAUDE.md 'Cross-profile data isolation' section."
    exit 1
fi

echo "✓ No hand-rolled daruda data-directory literals found in tracked source."
