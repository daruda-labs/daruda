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
pub(crate) fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    /// The reason this module exists: a status read must leave `.git/index`
    /// byte-identical. The plain run at the end is what proves the fixture
    /// was actually stale — without it the first assert could pass on a repo
    /// git had nothing to refresh in.
    #[test]
    fn a_status_read_does_not_rewrite_the_index() {
        if !crate::lane::git::has_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        run(root, &["init", "-q"]);
        run(root, &["config", "user.email", "daruda@test"]);
        run(root, &["config", "user.name", "daruda"]);
        std::fs::write(root.join("f.txt"), b"one\n").unwrap();
        run(root, &["add", "f.txt"]);
        run(root, &["commit", "-qm", "initial"]);

        // Re-write the same bytes under a later mtime so the index's stat
        // cache goes stale. The sleep buys a distinct timestamp: within one
        // filesystem tick git sees nothing to refresh and the test would pass
        // for the wrong reason.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(root.join("f.txt"), b"one\n").unwrap();

        let index = root.join(".git").join("index");
        let before = std::fs::read(&index).unwrap();

        git_command(root)
            .args(["status", "--porcelain=v1"])
            .output()
            .unwrap();
        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "a status read through the builder must not touch the index"
        );

        Command::new("git")
            .current_dir(root)
            .args(["status", "--porcelain=v1"])
            .output()
            .unwrap();
        assert_ne!(
            std::fs::read(&index).unwrap(),
            before,
            "the fixture was not stale — the assert above proved nothing"
        );
    }
}
