//! Throwing changes away, back to HEAD. The list is re-read when the discard
//! runs, not taken from the panel's cache: an agent may have committed,
//! renamed or created files since the panel last looked. Only the paths the
//! user was shown are touched, and each is undone by whether HEAD has it —
//! restored if so, removed if not — rather than by its status letters, which
//! a merge conflict (`AA` over a file HEAD has) would misread.
//!
//! Paths are repository-root relative, as `git status` reports them, and
//! every command runs from the lane's worktree root with literal pathspecs:
//! a file named `[id].tsx` is that file, not a pattern.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{GitError, GitFileEntry, git_worktree_status, run_git};

/// A batch of discards grouped by the git command each needs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DiscardPlan {
    /// HEAD has these: restore index and worktree from it.
    pub restore: Vec<PathBuf>,
    /// Tracked, but HEAD lacks these: drop them from index and worktree.
    pub remove: Vec<PathBuf>,
    /// Never tracked: delete them.
    pub untracked: Vec<PathBuf>,
}

impl DiscardPlan {
    /// Plan `entries` against the set of paths HEAD has. A path listed in
    /// both the staged and the unstaged set is planned once.
    pub fn classify<'a>(
        entries: impl IntoIterator<Item = &'a GitFileEntry>,
        in_head: &HashSet<PathBuf>,
    ) -> Self {
        let mut plan = Self::default();
        let mut seen = HashSet::new();
        for entry in entries {
            if entry.x == '?' {
                if seen.insert(entry.path.clone()) {
                    plan.untracked.push(entry.path.clone());
                }
                continue;
            }
            for path in std::iter::once(&entry.path).chain(&entry.original_path) {
                if !seen.insert(path.clone()) {
                    continue;
                }
                if in_head.contains(path) {
                    plan.restore.push(path.clone());
                } else {
                    plan.remove.push(path.clone());
                }
            }
        }
        plan
    }

    pub fn is_empty(&self) -> bool {
        self.restore.is_empty() && self.remove.is_empty() && self.untracked.is_empty()
    }

    /// Removals run before the restore, so a rename's new path is gone
    /// before its original returns.
    fn run(&self, worktree_root: &Path) -> Result<(), GitError> {
        run_with_paths(worktree_root, &["rm", "-q", "-f", "--"], &self.remove)?;
        run_with_paths(
            worktree_root,
            &["restore", "--source=HEAD", "--staged", "--worktree", "--"],
            &self.restore,
        )?;
        run_with_paths(worktree_root, &["clean", "-q", "-f", "--"], &self.untracked)
    }
}

/// Put `pinned` back to HEAD, reading the status afresh from
/// `worktree_root` and skipping any pinned path that no longer has a change.
pub fn discard(worktree_root: &Path, pinned: &[PathBuf]) -> Result<(), GitError> {
    if pinned.is_empty() {
        return Ok(());
    }
    let pinned: HashSet<&PathBuf> = pinned.iter().collect();
    let status = git_worktree_status(worktree_root)?;
    let entries: Vec<&GitFileEntry> = status
        .staged
        .iter()
        .chain(&status.unstaged)
        .filter(|e| pinned.contains(&e.path))
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    let candidates: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.x != '?')
        .flat_map(|e| std::iter::once(e.path.clone()).chain(e.original_path.clone()))
        .collect();
    let in_head = paths_in_head(worktree_root, &candidates)?;
    DiscardPlan::classify(entries, &in_head).run(worktree_root)
}

/// Which of `paths` HEAD has. An unborn HEAD has none.
fn paths_in_head(cwd: &Path, paths: &[PathBuf]) -> Result<HashSet<PathBuf>, GitError> {
    if paths.is_empty() || run_git(cwd, ["rev-parse", "-q", "--verify", "HEAD"]).is_err() {
        return Ok(HashSet::new());
    }
    let mut args: Vec<&OsStr> = [
        "--literal-pathspecs",
        "ls-tree",
        "-r",
        "-z",
        "--name-only",
        "HEAD",
        "--",
    ]
    .iter()
    .map(OsStr::new)
    .collect();
    args.extend(paths.iter().map(|p| p.as_os_str()));
    let out = run_git(cwd, args)?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn run_with_paths(cwd: &Path, head: &[&str], paths: &[PathBuf]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&OsStr> = std::iter::once("--literal-pathspecs")
        .chain(head.iter().copied())
        .map(OsStr::new)
        .collect();
    args.extend(paths.iter().map(|p| p.as_os_str()));
    run_git(cwd, args).map(|_| ())
}

#[cfg(test)]
mod tests;
