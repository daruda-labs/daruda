#!/usr/bin/env bash
# Lint: keep platform capabilities behind their one gate.
#
# Background: the same OS call used to be written out in every crate that
# needed it. Killing a child's process tree was spelled four times in two
# spellings of one POSIX call; `create_owner_only_dir` existed twice
# verbatim; three watchers each defined `canonicalize_or_self`. Adding a
# second platform would have meant writing each of those again, per
# platform — which is the seam being wrong, not the work being doubled.
#
# So each capability has exactly one home, and this script is what keeps
# it that way. It checks the *gate*, not the `cfg`: a domain crate asking
# "am I on Windows?" is fine, reaching for `killpg` itself is not.
#
# | Instead of                    | Call                                  |
# |-------------------------------|---------------------------------------|
# | `Command::new`                | `daruda_core::process::command`        |
# | `libc::kill*` / `process_group`| `daruda_core::process::{lead_own_group,kill_tree}` |
# | `fs::canonicalize`            | `daruda_core::path::canonicalize`      |
# | `env::var("SHELL")`           | `daruda_core::shell::interactive`      |
# | `std::os::unix::fs::symlink`  | `daruda_core::path::symlink`           |
#
# Tests are exempt: a fixture spawning `git init` is not the app's
# behaviour on a user's machine, and forcing it through the gate buys
# nothing. Both a whole test file and an in-file `#[cfg(test)]` module are
# skipped, mirroring lint-inline-literals.sh.
#
# Usage:
#   scripts/lint-platform-boundary.sh
#
# Exit codes:
#   0 — clean
#   1 — at least one violation found

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The two places a platform call may live.
#
#   - `daruda_core`'s capability modules: the gates themselves.
#   - `crates/app/src/platform/`: capabilities that need a window handle,
#     which a GPUI-free crate cannot hold.
#
# Plus the narrow cases where the call *is* the subject rather than a way
# to get something done:
#   - `daruda_store/src/persistence.rs` + `profile.rs`: resolve where
#     daruda's own state lives, which is what a data dir is.
#
# And two credential readers. Both reach an OS credential store, and the
# honest reason they are two rather than one is crate layering:
# `daruda_agent` cannot see `crates/app`. It shows — `keychain.rs` has a
# Linux `secret-tool` arm while `credentials.rs` falls back to a JSON
# file — so this pair is a deferral, not a design:
#   - `app/src/remote_channel/keychain.rs`: daruda's own secrets. Takes the
#     service name as a parameter, so it is already the single door for
#     everything in `app` (`telegram/keychain.rs` delegates to it).
#   - `daruda_agent/src/accounts/credentials.rs`: reads an entry *another
#     program* owns (Claude Code's), whose location is that CLI's choice.
#
# Moving both behind one gate means putting it where daruda_agent can
# reach — a job for the stage that adds the Windows arm, since that is
# when the divergence starts costing something.
WHITELIST_PREFIXES=(
    "crates/daruda_core/src/process.rs"
    "crates/daruda_core/src/path.rs"
    "crates/daruda_core/src/shell.rs"
    "crates/daruda_core/src/shell/"
    "crates/app/src/platform/"
    "crates/daruda_store/src/persistence.rs"
    "crates/daruda_store/src/profile.rs"
    "crates/app/src/remote_channel/keychain.rs"
    "crates/daruda_agent/src/accounts/credentials.rs"
)

# Known violations, listed so they are visible rather than silently exempt.
# These are not allowed regions — each is a capability that belongs behind
# the boundary and has not moved yet.
#
#   - `workspace/sync/ports.rs`: `#[cfg(target_os)] mod macos` (lsof/ps) and
#     `mod linux` (/proc) are 371 of the file's 776 lines — exactly the shape
#     this rule forbids. Its spawns do go through the gate, so what is left
#     is a file move into `app/src/platform/`, big enough to be its own
#     change and not worth burying in this one.
#
# A file here still fails the lint if it calls the OS *outside* what is
# already noted — the point is to not grow the debt, not to pardon it.
DEFERRED=(
    "crates/app/src/workspace/sync/ports.rs — platform modules not yet moved to app/src/platform/"
)

is_whitelisted() {
    local file="$1"
    for w in "${WHITELIST_PREFIXES[@]}"; do
        case "$file" in
            "$w"*) return 0 ;;
        esac
    done
    return 1
}

SCAN_DIRS=(
    "crates/app/src"
    "crates/daruda_core/src"
    "crates/daruda_acp/src"
    "crates/daruda_agent/src"
    "crates/daruda_config/src"
    "crates/daruda_flow/src"
    "crates/daruda_store/src"
    "crates/daruda_terminal/src"
    "crates/daruda_update/src"
    # Examples ship as usage documentation, so a reader copies what they do.
    "crates/daruda_flow/examples"
)

# Portable array population — no `mapfile` (macOS bash 3.2 doesn't ship it).
FILES=()
while IFS= read -r line; do
    FILES+=("$line")
done < <(find "${SCAN_DIRS[@]}" -name '*.rs' -type f | sort)

violations=""
for file in "${FILES[@]}"; do
    if is_whitelisted "$file"; then
        continue
    fi
    case "$file" in
        */tests.rs|*/tests/*|*_tests.rs) continue ;;
    esac
    # Skip `#[cfg(test)]` items by tracking their braces, not by stopping at
    # the first one. Stopping was wrong twice over: `#[cfg(test)] mod name;`
    # is a declaration with no body, and an attribute on a test-only method
    # sits mid-file — either one blanked the rest of the file. `node.rs` and
    # `sync/ports.rs` each hid production calls behind one.
    hit=$(perl -ne '
        if ($skip_depth > 0) {
            $skip_depth += tr/{//;
            $skip_depth -= tr/}//;
            next;
        }
        if ($pending_test) {
            # A bare `mod name;` declares a file-backed module — nothing to
            # skip here, and that file is scanned on its own.
            if (/;\s*$/ && !/\{/) { $pending_test = 0; next; }
            if (/\{/) {
                $pending_test = 0;
                $skip_depth = tr/{// - tr/}//;
                next;
            }
            # Attribute, then a line that opens nothing yet — keep waiting.
            next;
        }
        if (/^\s*#\[cfg\(test\)\]/) { $pending_test = 1; next; }
        next if m{^\s*//};
        if (/\bCommand::new\b/ && !/CommandBuilder::new/) {
            print "$ARGV:$.: Command::new -> daruda_core::process::command\n";
        }
        if (/\blibc::(kill|killpg)\b/) {
            print "$ARGV:$.: libc kill -> daruda_core::process::kill_tree\n";
        }
        if (/\bprocess_group\s*\(/) {
            print "$ARGV:$.: process_group -> daruda_core::process::lead_own_group\n";
        }
        if (/\bfs::canonicalize\b/ || /\.canonicalize\(\)/) {
            print "$ARGV:$.: canonicalize -> daruda_core::path::canonicalize\n";
        }
        if (/env::var(_os)?\(\s*"SHELL"/) {
            print "$ARGV:$.: \$SHELL -> daruda_core::shell::interactive\n";
        }
        if (/\bstd::os::unix::fs::symlink\b/) {
            print "$ARGV:$.: unix symlink -> daruda_core::path::symlink\n";
        }
    ' "$file" || true)
    if [ -n "$hit" ]; then
        violations="${violations}${hit}"$'\n'
    fi
done

if [ -n "$violations" ]; then
    echo "Platform capability called outside its gate:"
    echo
    echo "$violations"
    echo "Each of these has one home in daruda_core (or app/src/platform for"
    echo "the ones needing a window). Calling the OS directly here is how the"
    echo "same code ends up in four crates, each needing its own second arm."
    exit 1
fi

echo "✓ Platform capabilities stay behind their gates."
if [ ${#DEFERRED[@]} -gt 0 ]; then
    echo
    echo "  Known and not yet moved (this lint does not catch these):"
    for d in "${DEFERRED[@]}"; do
        echo "    - $d"
    done
fi
