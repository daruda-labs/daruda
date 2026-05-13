//! Blocking wrappers around the `git` CLI for worktree operations.
//!
//! Synchronous on purpose — the UI layer invokes these from
//! `cx.background_spawn` so the main thread stays free. Keeping this
//! module GPUI-free means it's unit-testable with a temp-directory
//! fixture and also reusable from CLI tools later.
//!
//! Every function shells out to `git`. If `git` isn't installed the
//! wrappers return `GitError::Spawn`; callers decide whether to fall
//! back (e.g. treat as "not a repo") or surface the error in the
//! status bar.
//!
//! The status / diff / staging / commit / push family lives in
//! [`status`]; this module retains the run-git plumbing, the repo
//! probe, worktree lifecycle, and the merge sub-module.

mod status;

pub use status::*;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Worktree entry parsed from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWorktreeInfo {
    pub path: PathBuf,
    /// `None` = detached HEAD (no attached branch).
    pub branch: Option<String>,
    pub head: Option<String>,
    /// True for the "bare" marker — daruda usually filters these out.
    pub bare: bool,
}

#[derive(Debug)]
pub enum GitError {
    /// Couldn't launch `git` at all (not installed, permission, etc.).
    Spawn(std::io::Error),
    /// `git` ran but exited non-zero.
    Exit { code: Option<i32>, stderr: String },
    /// Output wasn't valid UTF-8.
    Utf8,
    /// `git worktree list --porcelain` returned an unparseable chunk.
    Parse(String),
    /// `git` was killed because it ran past the configured deadline. The
    /// usual cause is a network operation (`push`, `fetch`, `pull`) on a
    /// stalled connection. Surfaces in the status bar so the user knows
    /// the worktree isn't actually frozen.
    Timeout(Duration),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Spawn(e) => write!(f, "spawn git: {e}"),
            GitError::Exit {
                code: Some(c),
                stderr,
            } => write!(f, "git exited {c}: {}", stderr.trim()),
            GitError::Exit { code: None, stderr } => {
                write!(f, "git terminated by signal: {}", stderr.trim())
            }
            GitError::Utf8 => write!(f, "git output not valid UTF-8"),
            GitError::Parse(s) => write!(f, "unexpected git porcelain output: {s}"),
            GitError::Timeout(d) => write!(f, "git timed out after {}s", d.as_secs()),
        }
    }
}

/// Default deadline for every `run_git` invocation. Network operations
/// (`push`, `fetch`, `pull`) on a slow connection can legitimately take
/// 30s+; 60s gives margin while still detecting a stalled connection
/// before the user assumes the UI is frozen. Local operations
/// (`status`, `log`, `rev-parse`) complete in well under a second so the
/// poll overhead is invisible.
const RUN_GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Granularity of the `try_wait` loop. 50 ms is short enough to make
/// short git commands feel synchronous, long enough that polling cost
/// stays well under 1% of CPU.
const RUN_GIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

impl std::error::Error for GitError {}

/// Run `git <args>` in `cwd` with a `RUN_GIT_TIMEOUT` deadline. Returns
/// stdout as a String on success. If the deadline elapses, the child is
/// killed (`Child::kill`) and the call returns `GitError::Timeout` —
/// callers must treat this distinctly from a clean failure (e.g. the
/// commit/push state is unknown when this fires).
pub(super) fn run_git<I, S>(cwd: &Path, args: I) -> Result<String, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_git_with_timeout(cwd, args, RUN_GIT_TIMEOUT)
}

/// Like [`run_git`] but with an explicit timeout. Exposed for tests so
/// they can pin a short deadline without waiting `RUN_GIT_TIMEOUT`.
fn run_git_with_timeout<I, S>(cwd: &Path, args: I, timeout: Duration) -> Result<String, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::io::Read as _;
    use std::thread;

    let mut child = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(GitError::Spawn)?;

    // Drain stdout/stderr concurrently so the child never blocks on a
    // full pipe buffer while we're polling `try_wait`. Each handle
    // returns the bytes it captured up to EOF (which the child closes
    // on exit, so the threads always terminate cleanly after `wait`).
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stdout_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    // SIGKILL the parent. Grandchildren (ssh, askpass,
                    // network helpers) may hold the stdout/stderr pipes
                    // open after the parent dies, which would block a
                    // `join()` here for as long as the OS takes to
                    // notice. Detach the drain threads instead — the OS
                    // reaps them when the parent process exits. The
                    // caller gets `Timeout` immediately.
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(stdout_thread);
                    drop(stderr_thread);
                    return Err(GitError::Timeout(timeout));
                }
                thread::sleep(RUN_GIT_POLL_INTERVAL);
            }
            Err(e) => return Err(GitError::Spawn(e)),
        }
    };

    let stdout_buf = stdout_thread.join().unwrap_or_default();
    let stderr_buf = stderr_thread.join().unwrap_or_default();

    if !status.success() {
        return Err(GitError::Exit {
            code: status.code(),
            stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        });
    }
    String::from_utf8(stdout_buf).map_err(|_| GitError::Utf8)
}

/// `git init` in `path`. Used by the Git Changes view's "Initialize
/// Git Repo" button — turns a non-git worktree into a fresh repo so
/// the user can start staging immediately.
pub fn git_init(path: &Path) -> Result<(), GitError> {
    run_git(path, ["init"]).map(|_| ())
}

/// Cheap availability probe — runs `git --version`. Use this to
/// decide whether to show the "Initialize Git Repo" button vs. hide
/// the worktree UI entirely.
pub fn has_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True when `path` is inside a git repository (any worktree counts).
pub fn is_git_repo(path: &Path) -> bool {
    run_git(path, ["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Top-level directory of the repo that owns `path` (the common
/// git dir). Returns `None` when `path` isn't inside a repo.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    run_git(path, ["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| PathBuf::from(s.trim()))
}

/// Currently-checked-out branch name at `path`, or `None` for detached
/// HEAD / non-repo. Uses `git symbolic-ref --short HEAD` which fails
/// cleanly when detached — we map that to `Ok(None)` rather than an
/// error so the caller can render "(detached)" without a special case.
pub fn current_branch(path: &Path) -> Result<Option<String>, GitError> {
    match run_git(path, ["symbolic-ref", "--short", "HEAD"]) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(GitError::Exit { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `git init` at `path`. Creates the directory if missing.
pub fn init(path: &Path) -> Result<(), GitError> {
    std::fs::create_dir_all(path).map_err(GitError::Spawn)?;
    run_git(path, ["init", "--quiet"]).map(|_| ())
}

/// `git worktree add <new_path> [-b <new_branch>] [<base>]`.
///
/// * `new_branch = Some(_)` → create a new branch (`-b`).
/// * `new_branch = None` → check out the existing branch whose ref
///   matches `new_path`'s basename (git's default).
/// * `base = Some(_)` → branch from / check out at this ref
///   (e.g. `"main"`, `"origin/main"`, a SHA). `None` falls through
///   to git's default (current HEAD when creating a new branch).
///
/// `repo_root` must be the main repository (any existing worktree
/// path works; git resolves the shared gitdir).
pub fn add_worktree(
    repo_root: &Path,
    new_path: &Path,
    new_branch: Option<&str>,
    base: Option<&str>,
) -> Result<(), GitError> {
    let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
    if let Some(b) = new_branch {
        args.push("-b".into());
        args.push(b.to_string());
    }
    args.push(new_path.to_string_lossy().into_owned());
    if let Some(b) = base {
        // Force commit resolution so `origin/<base>`-style refs do
        // not become the new branch's upstream — without `^{commit}`
        // git auto-tracks the remote ref, which then triggers
        // unintended `push` collisions when daruda later finalizes
        // the worktree. Mirrors superset-desktop git.ts:457-519.
        args.push(format!("{b}^{{commit}}"));
    }
    run_git(repo_root, args)?;

    // `push.autoSetupRemote=true` makes the *first* `git push` from
    // the new worktree auto-create the remote branch and set its
    // upstream — the user never has to type `-u origin <branch>`.
    // Failure here is non-fatal: the worktree is already on disk.
    let _ = run_git(
        new_path,
        ["config", "--local", "push.autoSetupRemote", "true"],
    );

    Ok(())
}

/// `git worktree remove [--force] <path>`. The main worktree of a
/// repo cannot be removed — git rejects that with a clear message
/// and we pass the error through.
///
/// Exit 128 with "is not a working tree" means the directory is already
/// gone (manual deletion or a prior interrupted removal). Treat that as
/// success so the UI can clean up the stale sidebar entry.
pub fn remove_worktree(repo_root: &Path, path: &Path, force: bool) -> Result<(), GitError> {
    let mut args: Vec<String> = vec!["worktree".into(), "remove".into()];
    if force {
        args.push("--force".into());
    }
    args.push(path.to_string_lossy().into_owned());
    match run_git(repo_root, args) {
        Ok(_) => Ok(()),
        Err(GitError::Exit {
            code: Some(128),
            ref stderr,
        }) if stderr.contains("is not a working tree") => Ok(()),
        Err(e) => Err(e),
    }
}

/// `git branch -D <branch>` — force-delete a branch. Used by the
/// "remove worktree + branch" flow: after `worktree remove` succeeds,
/// the branch is no longer checked out anywhere, so deletion is safe
/// (git still rejects deleting the currently-checked-out branch of
/// the main worktree, surfacing as `GitError::Exit`).
pub fn delete_branch(repo_root: &Path, branch: &str) -> Result<(), GitError> {
    run_git(repo_root, ["branch", "-D", branch]).map(|_| ())
}

/// `git worktree list --porcelain`. Returns one entry per linked
/// worktree (including the main one). The porcelain format is
/// stable: blank-line-separated records, each `key value` lines.
/// Unknown keys are ignored so future git versions don't break us.
pub fn list_worktrees(repo_root: &Path) -> Result<Vec<GitWorktreeInfo>, GitError> {
    let stdout = run_git(repo_root, ["worktree", "list", "--porcelain"])?;
    parse_worktree_list(&stdout)
}

/// Aggregated repo probe: combines `has_git` + `is_git_repo` +
/// `repo_root` + `list_worktrees` into one call. Returns `None` when
/// git is unusable or the path isn't a repo, so callers can branch on
/// a single Option instead of threading three fallible probes.
pub struct RepoProbe {
    pub repo_root: PathBuf,
    pub worktrees: Vec<GitWorktreeInfo>,
}

pub fn probe_repo(project_root: &Path) -> Option<RepoProbe> {
    if !has_git() || !is_git_repo(project_root) {
        return None;
    }
    let repo_root = repo_root(project_root)?;
    let worktrees = list_worktrees(&repo_root).ok()?;
    Some(RepoProbe {
        repo_root,
        worktrees,
    })
}

fn parse_worktree_list(text: &str) -> Result<Vec<GitWorktreeInfo>, GitError> {
    let mut out = Vec::new();
    let mut cur: Option<GitWorktreeInfo> = None;

    let flush = |cur: &mut Option<GitWorktreeInfo>, out: &mut Vec<GitWorktreeInfo>| {
        if let Some(wt) = cur.take() {
            out.push(wt);
        }
    };

    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut cur, &mut out);
            cur = Some(GitWorktreeInfo {
                path: PathBuf::from(rest),
                branch: None,
                head: None,
                bare: false,
            });
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            let Some(wt) = cur.as_mut() else {
                return Err(GitError::Parse(line.to_string()));
            };
            wt.head = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            let Some(wt) = cur.as_mut() else {
                return Err(GitError::Parse(line.to_string()));
            };
            // `branch refs/heads/main` → `main`.
            wt.branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
        } else if line == "bare" {
            let Some(wt) = cur.as_mut() else {
                return Err(GitError::Parse(line.to_string()));
            };
            wt.bare = true;
        } else if line == "detached" {
            // Explicit detached marker — leave `branch: None`.
            if cur.is_none() {
                return Err(GitError::Parse(line.to_string()));
            }
        }
        // Silently skip unknown keys (locked, prunable, …) — we only
        // need path/branch/HEAD for the sidebar.
    }
    flush(&mut cur, &mut out);
    Ok(out)
}

// ----------------------------------------------------------------
// Git status + diff + commit
// ----------------------------------------------------------------
/// Outcome of a `git merge` operation.
#[derive(Debug)]
pub enum MergeOutcome {
    /// `git merge` reported "Already up to date."
    AlreadyUpToDate,
    /// Merge completed (fast-forward or new merge commit).
    Success,
    /// One or more files have conflicts.  The target worktree is left in
    /// mid-merge state so the user can resolve conflicts manually and commit.
    /// The caller may call `git_merge_abort` to cancel instead.
    Conflicts(Vec<String>),
}

/// `git merge --no-ff --no-edit <source_branch>` run from `target_path`.
///
/// Executed in the *target* worktree directory so the merge lands on
/// the branch that is already checked out there — no `checkout` needed.
/// `--no-ff` always creates a merge commit, preserving branch history even
/// when a fast-forward would be possible.
///
/// Exit 0 + "Already up to date." stdout → `AlreadyUpToDate`.
/// Exit 0 otherwise → `Success`.
/// Exit 1 → stdout is scanned for "CONFLICT" lines → `Conflicts(files)`.
/// Any other exit code → `Err(GitError::Exit)`.
pub fn git_merge(target_path: &Path, source_branch: &str) -> Result<MergeOutcome, GitError> {
    let output = Command::new("git")
        .current_dir(target_path)
        .args(["merge", "--no-ff", "--no-edit", source_branch])
        .output()
        .map_err(GitError::Spawn)?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    match output.status.code() {
        Some(0) => {
            if stdout.trim() == "Already up to date." {
                Ok(MergeOutcome::AlreadyUpToDate)
            } else {
                Ok(MergeOutcome::Success)
            }
        }
        Some(1) => {
            // Parse conflicting file paths from git's stdout.
            //
            // Common formats (git 2.x):
            //   CONFLICT (content): Merge conflict in path/to/file
            //   CONFLICT (add/add): Merge conflict in path/to/file
            //   CONFLICT (modify/delete): path/to/file deleted in HEAD. ...
            //   CONFLICT (rename/rename): old renamed to new1 in HEAD and new2 in ...
            //   CONFLICT (rename/delete): path/to/old renamed to new in HEAD, ...
            //
            // Strategy: after "): ", strip the "Merge conflict in " prefix
            // (content/add), then take the first whitespace-delimited token
            // as the file path. For rename variants the "old" path (first
            // token after "): ") is always a real file involved in the
            // conflict, which is sufficient for display purposes.
            let files: Vec<String> = stdout
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if !line.starts_with("CONFLICT") {
                        return None;
                    }
                    let after_colon = line.find("): ")?.checked_add(3).map(|i| &line[i..])?;
                    let path_start = after_colon
                        .strip_prefix("Merge conflict in ")
                        .unwrap_or(after_colon);
                    let file = path_start.split_whitespace().next()?.trim_end_matches('.');
                    if file.is_empty() {
                        None
                    } else {
                        Some(file.to_string())
                    }
                })
                .collect();
            Ok(MergeOutcome::Conflicts(files))
        }
        _ => Err(GitError::Exit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
    }
}

/// `git merge --abort` — cancel an in-progress conflicted merge and
/// restore the target worktree to its pre-merge state.
pub fn git_merge_abort(target_path: &Path) -> Result<(), GitError> {
    run_git(target_path, ["merge", "--abort"]).map(|_| ())
}

/// `git restore -- <path>` — discard working-tree changes for a tracked file.
pub fn git_discard_working(wt_path: &Path, path: &Path) -> Result<(), GitError> {
    let args: Vec<&OsStr> = vec![OsStr::new("restore"), OsStr::new("--"), path.as_os_str()];
    run_git(wt_path, args).map(|_| ())
}

/// `git clean -f -- <path>` — delete an untracked file.
pub fn git_clean_untracked(wt_path: &Path, path: &Path) -> Result<(), GitError> {
    let args: Vec<&OsStr> = vec![
        OsStr::new("clean"),
        OsStr::new("-f"),
        OsStr::new("--"),
        path.as_os_str(),
    ];
    run_git(wt_path, args).map(|_| ())
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------


#[cfg(test)]
mod tests;
