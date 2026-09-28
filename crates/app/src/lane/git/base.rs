//! Where a lane branched from, and what it has committed since.
//!
//! The comparison is `merge-base(base, HEAD)..HEAD` — committed changes only,
//! never the index or the working tree, which the status list already shows.
//! The merge-base is resolved once and handed back, so the file list and every
//! per-file diff read the same pair of commits even if a ref moves between.

use std::path::{Path, PathBuf};

use super::{GitError, parse_numstat, run_git};

/// Remote a bare branch name falls back to when no local branch carries it.
const DEFAULT_REMOTE: &str = "origin";

/// A base resolved to the commit it names today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBase {
    /// What the user is told the lane is compared against (`origin/main`).
    pub label: String,
    pub sha: String,
}

/// Why a lane has no against-base view. Each is a state to show, not an
/// error to report: a lane without a base is ordinary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseProblem {
    /// Neither the lane nor its project names a base.
    NoBaseConfigured,
    /// The lane is checked out on its own base branch.
    OnBaseBranch,
    /// The named base resolves to no commit (deleted, never fetched).
    NotFound(String),
    /// Base and HEAD share no history (unrelated roots, a shallow clone).
    NoMergeBase,
    /// git itself failed; the message is for the log, not the row.
    Git(String),
}

impl From<GitError> for BaseProblem {
    fn from(e: GitError) -> Self {
        Self::Git(e.to_string())
    }
}

/// The two commits a comparison is taken between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseTips {
    pub base: ResolvedBase,
    pub head: String,
}

/// One file the lane changed since its base. Paths are repo-root-relative,
/// the form `git status` reports and `LanePaths::from_git_status` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeFile {
    pub path: PathBuf,
    /// The source of a rename or copy.
    pub old_path: Option<PathBuf>,
    /// First letter of git's status: `M`, `A`, `D`, `R`, `C`, `T`.
    pub status: char,
    pub added: u32,
    pub removed: u32,
}

/// Everything the lane committed since it left its base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgainstBase {
    pub tips: BaseTips,
    pub merge_base: String,
    pub files: Vec<RangeFile>,
    pub commits: u32,
}

/// Resolve `name` to a commit. A local branch reads as its upstream when it
/// has one — the lane is compared against what the base looks like upstream,
/// not a local copy that may be behind. A name no local branch carries falls
/// back to `origin/<name>`.
pub fn resolve_base_ref(repo: &Path, name: &str) -> Result<ResolvedBase, BaseProblem> {
    if verify(repo, &format!("refs/heads/{name}")).is_some() {
        let upstream = run_git(
            repo,
            [
                "for-each-ref",
                "--format=%(upstream:short)",
                &format!("refs/heads/{name}"),
            ],
        )?;
        let upstream = upstream.trim();
        if !upstream.is_empty()
            && let Some(sha) = verify(repo, upstream)
        {
            return Ok(ResolvedBase {
                label: upstream.to_owned(),
                sha,
            });
        }
    }
    if let Some(sha) = verify(repo, name) {
        return Ok(ResolvedBase {
            label: name.to_owned(),
            sha,
        });
    }
    let remote = format!("{DEFAULT_REMOTE}/{name}");
    match verify(repo, &remote) {
        Some(sha) => Ok(ResolvedBase { label: remote, sha }),
        None => Err(BaseProblem::NotFound(name.to_owned())),
    }
}

/// The two tips a comparison would be taken between, or why there is none.
/// Cheap — two `rev-parse`s — so a caller can compare it against what it
/// last computed and skip the diff when neither side moved.
pub fn base_tips(
    wt: &Path,
    base_name: Option<&str>,
    current_branch: Option<&str>,
) -> Result<BaseTips, BaseProblem> {
    let name = base_name.ok_or(BaseProblem::NoBaseConfigured)?;
    let base = resolve_base_ref(wt, name)?;
    if current_branch.is_some_and(|b| b == name || b == base.label) {
        return Err(BaseProblem::OnBaseBranch);
    }
    let head = verify(wt, "HEAD").ok_or_else(|| BaseProblem::NotFound("HEAD".to_owned()))?;
    Ok(BaseTips { base, head })
}

/// What the lane committed between the merge-base of `tips` and its HEAD.
pub fn changes_since(wt: &Path, tips: BaseTips) -> Result<AgainstBase, BaseProblem> {
    let merge_base = merge_base(wt, &tips.base.sha, &tips.head)?;
    let names = run_git(
        wt,
        ["diff", "--name-status", "-z", "-M", &merge_base, &tips.head],
    )?;
    let stats = run_git(
        wt,
        ["diff", "--numstat", "-z", "-M", &merge_base, &tips.head],
    )?;
    let commits = run_git(
        wt,
        [
            "rev-list",
            "--count",
            &format!("{merge_base}..{}", tips.head),
        ],
    )?;
    Ok(AgainstBase {
        files: join_stats(parse_name_status(&names), &parse_numstat(&stats)),
        commits: commits.trim().parse().unwrap_or(0),
        merge_base,
        tips,
    })
}

/// `git diff -M <from> <to> -- <paths>` for one file. Both sides of a rename
/// go in the pathspec, or git cannot pair them and shows a delete and an add.
pub fn git_diff_range(
    wt: &Path,
    from: &str,
    to: &str,
    paths: &[&Path],
) -> Result<String, GitError> {
    let mut args: Vec<String> = vec![
        "diff".into(),
        "-M".into(),
        from.into(),
        to.into(),
        "--".into(),
    ];
    args.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
    run_git(wt, args)
}

/// The file as it was at `rev` — the Raw view of a range pane.
pub fn git_show_at(wt: &Path, rev: &str, repo_rel: &Path) -> Result<Vec<u8>, GitError> {
    let spec = format!("{rev}:{}", super::status::tree_path(repo_rel));
    let output = super::git_command(wt)
        .args(["show", &spec])
        .output()
        .map_err(GitError::Spawn)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(GitError::Exit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn merge_base(wt: &Path, a: &str, b: &str) -> Result<String, BaseProblem> {
    match run_git(wt, ["merge-base", a, b]) {
        Ok(sha) => Ok(sha.trim().to_owned()),
        // Exit 1 is git's "no common ancestor"; anything else is a failure.
        Err(GitError::Exit { code: Some(1), .. }) => Err(BaseProblem::NoMergeBase),
        Err(e) => Err(e.into()),
    }
}

/// The commit `rev` names, or `None` when it names none.
fn verify(repo: &Path, rev: &str) -> Option<String> {
    run_git(
        repo,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .ok()
    .map(|s| s.trim().to_owned())
    .filter(|s| !s.is_empty())
}

/// `--name-status -z` records: `STATUS\0PATH\0`, or for a rename or copy
/// `R<score>\0OLD\0NEW\0`.
pub(crate) fn parse_name_status(text: &str) -> Vec<RangeFile> {
    let mut out = Vec::new();
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    while let Some(code) = records.next() {
        let Some(status) = code.chars().next() else {
            continue;
        };
        let Some(first) = records.next() else {
            break;
        };
        let (old_path, path) = if matches!(status, 'R' | 'C') {
            match records.next() {
                Some(new) => (Some(PathBuf::from(first)), PathBuf::from(new)),
                None => break,
            }
        } else {
            (None, PathBuf::from(first))
        };
        out.push(RangeFile {
            path,
            old_path,
            status,
            added: 0,
            removed: 0,
        });
    }
    out
}

/// Line counts keyed by destination path, which `parse_numstat` also keys on.
fn join_stats(mut files: Vec<RangeFile>, stats: &[(u32, u32, PathBuf)]) -> Vec<RangeFile> {
    for file in &mut files {
        if let Some((added, removed, _)) = stats.iter().find(|(_, _, p)| *p == file.path) {
            file.added = *added;
            file.removed = *removed;
        }
    }
    files
}

#[cfg(test)]
mod tests;
