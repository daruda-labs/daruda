//! The single place a `git` child process is configured.
//!
//! Every repo-targeting invocation in the app goes through
//! [`git_command`]. The flags it pins are not preferences — each one
//! closes a way the CLI would otherwise corrupt what daruda reads back:
//!
//! - `--no-optional-locks` keeps a read-only `git status` from rewriting
//!   `.git/index` to refresh its stat cache. That write is what a `.git`
//!   watcher would see, re-triggering the status it came from.
//! - `core.fsmonitor=false` stops a repo-supplied hook from running as a
//!   side effect of a status read.
//! - `log.showSignature=false` keeps a signature block out of
//!   `--pretty`/`--format` output the parsers here assume is bare.
//! - `--no-pager` and the `C` locale keep output machine-shaped: several
//!   call sites match on git's own English stderr.

use std::path::Path;
use std::process::Command;

/// A `git` command rooted at `cwd`, configured for machine reading.
pub fn git_command(cwd: &Path) -> Command {
    let mut command = daruda_core::process::command("git");
    command
        .current_dir(cwd)
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "log.showSignature=false"])
        .arg("--no-optional-locks")
        .arg("--no-pager")
        // `LANGUAGE` outranks `LC_ALL`/`LANG` in gettext's resolution
        // order, so it must be cleared rather than overridden.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env_remove("LANGUAGE");
    command
}
