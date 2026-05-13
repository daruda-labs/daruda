//! `git status` / `git diff` / staging / commit / push / fetch / pull
//! wrappers, plus the parsing logic for the porcelain branch line.
//!
//! Every function shells out via [`super::run_git`] on
//! `background_executor` and returns owned data — no GPUI types.
//!
//! Submodule wiring: `git/mod.rs` declares `mod status;` +
//! `pub use status::*;` so external callers write
//! `crate::worktree::git::git_status(...)`. The `run_git` /
//! `GitError` symbols used here are `pub(super)` in `mod.rs`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{GitError, run_git};

/// One file entry from `git status --porcelain=v1`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitFileEntry {
    /// Staged-column status character (X in `XY PATH`).
    pub x: char,
    /// Working-tree status character (Y in `XY PATH`).
    pub y: char,
    /// File path (renamed files: destination / new name).
    pub path: PathBuf,
    /// Original path for renames / copies. `None` for non-rename entries.
    /// Lets the sidebar render `old → new` for `R`/`C` status without a
    /// second git invocation.
    pub original_path: Option<PathBuf>,
}

/// Staged + unstaged file sets from `git status --porcelain=v1 --branch`.
#[derive(Debug, Clone, Default)]
pub struct GitStatusData {
    /// Files with a staged change (X != ' ' && X != '?' && X != '!').
    pub staged: Vec<GitFileEntry>,
    /// Files with an unstaged change or untracked status.
    pub unstaged: Vec<GitFileEntry>,
    /// Current branch name; `None` for detached HEAD.
    pub branch: Option<String>,
    /// Configured upstream ref (e.g. `origin/main`); `None` when no upstream
    /// is set or HEAD is detached.
    pub upstream: Option<String>,
    /// Number of commits the local branch is ahead of `upstream`.
    pub ahead: u32,
    /// Number of commits the local branch is behind `upstream`.
    pub behind: u32,
    /// `(added, removed)` per tracked file path, sourced from
    /// `git diff HEAD --numstat`. Untracked files are absent (no HEAD
    /// baseline to compare against). Empty when the repo has no
    /// commits yet (`git diff HEAD` errors gracefully).
    pub diffstat: std::collections::HashMap<PathBuf, (u32, u32)>,
}

/// Parse the body of a `## ...` branch header line emitted by
/// `git status --branch`. Examples handled:
/// - `main` → branch="main"
/// - `main...origin/main` → upstream="origin/main"
/// - `main...origin/main [ahead 3]` → ahead=3
/// - `main...origin/main [ahead 1, behind 2]` → ahead=1, behind=2
/// - `main...origin/main [gone]` → upstream still recorded; ahead/behind 0
/// - `HEAD (no branch)` → detached HEAD, branch=None
/// - `No commits yet on main` → branch="main"
fn parse_branch_line_body(s: &str) -> (Option<String>, Option<String>, u32, u32) {
    if s == "HEAD (no branch)" {
        return (None, None, 0, 0);
    }
    let head_with_track = s.strip_prefix("No commits yet on ").unwrap_or(s);

    // `[ahead N, behind M]` / `[gone]` lives at the very end if present.
    let (head, tracking) = match head_with_track.rfind(" [") {
        Some(i) if head_with_track.ends_with(']') => (
            &head_with_track[..i],
            Some(&head_with_track[i + 2..head_with_track.len() - 1]),
        ),
        _ => (head_with_track, None),
    };

    let (branch, upstream) = match head.find("...") {
        Some(i) => (Some(head[..i].to_string()), Some(head[i + 3..].to_string())),
        None => (Some(head.to_string()), None),
    };

    let (mut ahead, mut behind) = (0u32, 0u32);
    if let Some(t) = tracking {
        for part in t.split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                behind = n.parse().unwrap_or(0);
            }
            // `gone` / unknown — leave both at 0; upstream string is still kept.
        }
    }

    (branch, upstream, ahead, behind)
}

/// Parse the output of `git status --porcelain=v1` into a `GitStatusData`.
///
/// Extracted so the logic can be unit-tested without a real git repo.
/// Conflict entries (where x or y is `'U'`, or both are `'D'` / `'A'`)
/// are placed only in `unstaged` — they are unresolvable as staged changes
/// and must be resolved before committing.
pub(crate) fn parse_git_status_output(output: &str) -> GitStatusData {
    let mut data = GitStatusData::default();
    for line in output.lines() {
        let line = line.trim_end();
        // `## ...` branch header — emitted by `git status --branch`.
        if let Some(rest) = line.strip_prefix("## ") {
            let (branch, upstream, ahead, behind) = parse_branch_line_body(rest);
            data.branch = branch;
            data.upstream = upstream;
            data.ahead = ahead;
            data.behind = behind;
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        let path_str = &line[3..];
        // Renamed / copied files appear as `old -> new`; capture both
        // sides so the sidebar can render `old → new`. The destination
        // is the canonical path for stage / unstage / diff ops.
        let (display, original) = match path_str.rfind(" -> ") {
            Some(i) => (
                &path_str[i + 4..],
                Some(PathBuf::from(path_str[..i].trim())),
            ),
            None => (path_str, None),
        };
        let file_path = PathBuf::from(display.trim());
        let entry = GitFileEntry {
            x,
            y,
            path: file_path,
            original_path: original,
        };
        // Conflict pairs must not appear in staged — they block commits.
        let is_conflict = matches!((x, y), ('U', _) | (_, 'U') | ('D', 'D') | ('A', 'A'));
        if !is_conflict && x != ' ' && x != '?' && x != '!' {
            data.staged.push(entry.clone());
        }
        if y != ' ' || x == '?' {
            data.unstaged.push(entry);
        }
    }
    data
}

/// `git status --porcelain=v1 --branch` for the worktree rooted at `path`,
/// plus a follow-up `git diff HEAD --numstat` to populate per-file
/// diffstat. The `--branch` flag prepends a `## branch...upstream [ahead
/// N, behind M]` header line that the parser turns into branch /
/// upstream / ahead / behind fields. Renamed entries in the `XY PATH ->
/// PATH` form show the destination.
///
/// The diffstat call is non-fatal — a fresh `git init` repo has no HEAD,
/// so the call errors and `diffstat` stays empty (sidebar simply omits
/// the `+N −M` column for that worktree until the first commit lands).
pub fn git_status(path: &Path) -> Result<GitStatusData, GitError> {
    let raw = run_git(path, ["status", "--porcelain=v1", "--branch"])?;
    let mut data = parse_git_status_output(&raw);
    if let Ok(stats) = git_diff_numstat(path) {
        for (added, removed, p) in stats {
            data.diffstat.insert(p, (added, removed));
        }
    }
    Ok(data)
}

/// `git diff HEAD --numstat` — `(added, removed, path)` per tracked file
/// with any working-tree or staged divergence from HEAD. Binary files
/// emit `-\t-\t<path>`; the parser maps those to `(0, 0)` so the sidebar
/// row can decide whether to show a stat at all (typically not, since
/// "+0 −0" implies "no change" which is misleading for binary diffs).
///
/// Failure is non-fatal for the sidebar: a brand-new repo with no
/// commits has no HEAD, and `git diff HEAD` exits non-zero. Callers
/// surface this as "no diffstat available" rather than an error.
pub fn git_diff_numstat(wt_path: &Path) -> Result<Vec<(u32, u32, PathBuf)>, GitError> {
    let raw = run_git(wt_path, ["diff", "HEAD", "--numstat"])?;
    Ok(parse_numstat(&raw))
}

pub(crate) fn parse_numstat(text: &str) -> Vec<(u32, u32, PathBuf)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let added = parts[0].parse().unwrap_or(0);
        let removed = parts[1].parse().unwrap_or(0);
        let path = PathBuf::from(parts[2].trim());
        out.push((added, removed, path));
    }
    out
}

/// `git diff [--cached] -- <path>` as a UTF-8 string.
/// Runs from `wt_path` (the worktree directory) so that paths relative to the
/// worktree are resolved correctly for both main and linked worktrees.
pub fn git_diff(wt_path: &Path, path: &Path, staged: bool) -> Result<String, GitError> {
    let mut args: Vec<String> = vec!["diff".into()];
    if staged {
        args.push("--cached".into());
    }
    args.push("--".into());
    args.push(path.to_string_lossy().into_owned());
    run_git(wt_path, args)
}

/// `git diff --no-index /dev/null <path>` — show an untracked file as entirely new.
///
/// Unlike `git diff`, this exits with code 1 when differences are found (which is
/// always the case for new files), so it uses a custom runner that accepts code 1.
pub fn git_diff_untracked(wt_path: &Path, path: &Path) -> Result<String, GitError> {
    use std::process::Command;
    let output = Command::new("git")
        .current_dir(wt_path)
        .args(["diff", "--no-index", "/dev/null"])
        .arg(path)
        .output()
        .map_err(GitError::Spawn)?;
    // Exit 0 = identical (empty file), 1 = differences found (normal for new files).
    match output.status.code() {
        Some(0) | Some(1) => String::from_utf8(output.stdout).map_err(|_| GitError::Utf8),
        _ => Err(GitError::Exit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
    }
}

/// `git add -- <path>` — stage a specific file.
pub fn git_add(repo_root: &Path, path: &Path) -> Result<(), GitError> {
    let args: Vec<&OsStr> = vec![OsStr::new("add"), OsStr::new("--"), path.as_os_str()];
    run_git(repo_root, args).map(|_| ())
}

/// `git add -- <path>...` — stage a batch of paths in a single git
/// invocation. Used by per-directory "stage all in this dir" so we
/// don't fork-and-exec git once per file.
pub fn git_add_paths(repo_root: &Path, paths: &[PathBuf]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&OsStr> = vec![OsStr::new("add"), OsStr::new("--")];
    args.extend(paths.iter().map(|p| p.as_os_str()));
    run_git(repo_root, args).map(|_| ())
}

/// `git add --all` — stage all changes including untracked files and deletions.
///
/// Must run from the worktree path (`wt_path`) so linked worktrees only stage
/// their own changes, not those of sibling worktrees sharing the same repo.
pub fn git_add_all(wt_path: &Path) -> Result<(), GitError> {
    run_git(wt_path, ["add", "--all"]).map(|_| ())
}

/// `git restore --staged -- <path>` — remove a file from the index (unstage).
///
/// Requires git ≥ 2.23 (macOS 12 Monterey ships git 2.30+, which satisfies
/// our minimum deployment target).
pub fn git_restore_staged(repo_root: &Path, path: &Path) -> Result<(), GitError> {
    let args: Vec<&OsStr> = vec![
        OsStr::new("restore"),
        OsStr::new("--staged"),
        OsStr::new("--"),
        path.as_os_str(),
    ];
    run_git(repo_root, args).map(|_| ())
}

/// `git restore --staged -- <path>...` — unstage a batch of paths in a
/// single git invocation. Companion to [`git_add_paths`] for per-dir
/// "unstage all" actions.
pub fn git_restore_staged_paths(repo_root: &Path, paths: &[PathBuf]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&OsStr> = vec![
        OsStr::new("restore"),
        OsStr::new("--staged"),
        OsStr::new("--"),
    ];
    args.extend(paths.iter().map(|p| p.as_os_str()));
    run_git(repo_root, args).map(|_| ())
}

/// `git commit -m <message>`.
pub fn git_commit(repo_root: &Path, message: &str) -> Result<(), GitError> {
    run_git(repo_root, ["commit", "-m", message]).map(|_| ())
}

/// `git push`.
pub fn git_push(repo_root: &Path) -> Result<(), GitError> {
    run_git(repo_root, ["push"]).map(|_| ())
}

/// `git restore --staged .` — remove all files from the index, keeping working-tree state.
pub fn git_restore_all_staged(wt_path: &Path) -> Result<(), GitError> {
    run_git(wt_path, ["restore", "--staged", "."]).map(|_| ())
}

/// `git commit --amend -m <message>` — replace the tip commit with staged changes
/// and the given message.
pub fn git_commit_amend(repo_root: &Path, message: &str) -> Result<(), GitError> {
    run_git(repo_root, ["commit", "--amend", "-m", message]).map(|_| ())
}

/// `git fetch` — download objects and refs from all remotes.
pub fn git_fetch(repo_root: &Path) -> Result<(), GitError> {
    run_git(repo_root, ["fetch"]).map(|_| ())
}

/// `git pull` — fetch and integrate with the current branch.
pub fn git_pull(repo_root: &Path) -> Result<(), GitError> {
    run_git(repo_root, ["pull"]).map(|_| ())
}

/// `git show :<path>` — retrieve the staged (index) content of a file as raw bytes.
pub fn git_show_staged(repo_root: &Path, path: &Path) -> Result<Vec<u8>, GitError> {
    let arg = format!(":{}", path.to_string_lossy());
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["show", &arg])
        .output()
        .map_err(GitError::Spawn)?;
    if !output.status.success() {
        return Err(GitError::Exit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(output.stdout)
}
