//! Unified changed-file list model for the Git Changes dock (GPUI-free).
//!
//! Builds the single sorted staged+unstaged list, groups it by directory,
//! and derives the per-directory staging state and per-row affordance gates.
//! The render layer in the parent module consumes these; `ordered_visible_paths`
//! is the single source of truth for the keyboard cursor's navigation order.

use std::path::PathBuf;

use crate::lane::git::GitFileEntry;
use crate::lane::paths::LanePaths;
use crate::path_ext::PathExt;

pub(in crate::workspace) struct UnifiedEntry {
    pub(in crate::workspace) path: PathBuf,
    pub(in crate::workspace) staged: Option<GitFileEntry>,
    pub(in crate::workspace) unstaged: Option<GitFileEntry>,
}

pub(super) fn build_unified_list(
    staged: &[GitFileEntry],
    unstaged: &[GitFileEntry],
) -> Vec<UnifiedEntry> {
    use std::collections::BTreeMap;

    let mut map: BTreeMap<PathBuf, UnifiedEntry> = BTreeMap::new();

    for e in staged {
        map.entry(e.path.clone())
            .or_insert_with(|| UnifiedEntry {
                path: e.path.clone(),
                staged: None,
                unstaged: None,
            })
            .staged = Some(e.clone());
    }
    for e in unstaged {
        map.entry(e.path.clone())
            .or_insert_with(|| UnifiedEntry {
                path: e.path.clone(),
                staged: None,
                unstaged: None,
            })
            .unstaged = Some(e.clone());
    }

    map.into_values().collect()
}

/// Group a unified list by lane-relative parent directory.
///
/// Output order: directory groups first (alphabetical by dir path),
/// then a single root-level group (no parent dir) at the end. Files
/// within each group are sorted alphabetically by their full path.
/// Putting root files last separates "files in a folder" from
/// "loose files at the repo root" visually instead of interleaving
/// them with directory groups by raw string order.
pub(super) fn group_by_dir(
    entries: Vec<UnifiedEntry>,
    wt_paths: &LanePaths<'_>,
) -> Vec<(Option<String>, Vec<UnifiedEntry>)> {
    use std::collections::BTreeMap;

    let mut dirs: BTreeMap<String, Vec<UnifiedEntry>> = BTreeMap::new();
    let mut root: Vec<UnifiedEntry> = Vec::new();

    for entry in entries {
        let abs = wt_paths.from_git_status(&entry.path);
        let dir = abs
            .strip_prefix_or_self(wt_paths.wt_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());
        match dir {
            Some(d) => dirs.entry(d).or_default().push(entry),
            None => root.push(entry),
        }
    }

    for v in dirs.values_mut() {
        v.sort_by(|a, b| a.path.cmp(&b.path));
    }
    root.sort_by(|a, b| a.path.cmp(&b.path));

    let mut groups: Vec<(Option<String>, Vec<UnifiedEntry>)> =
        dirs.into_iter().map(|(k, v)| (Some(k), v)).collect();
    if !root.is_empty() {
        groups.push((None, root));
    }
    groups
}

/// One row of the Git Changes list, flattened so the view can address every
/// drawn thing by a single index and virtualize over it.
///
/// Mirrors zed's `git_ui::GitListEntry`: a directory header is a **row**, not
/// chrome wrapped around a group. Keeping the tree as nested groups forced the
/// view to build an element per changed file on every render, which is linear
/// in the change set (measured 6.8 ms at 200 files, 32 ms at 1000).
pub(in crate::workspace) enum GitChangesRow {
    DirHeader(GitDirHeaderRow),
    File(UnifiedEntry),
}

/// A directory group's header row. Carries everything the header needs to
/// draw and act, because the group's entries are no longer adjacent to it in
/// a nested structure.
pub(in crate::workspace) struct GitDirHeaderRow {
    /// Lane-relative directory path, as displayed and as keyed in the
    /// collapsed-dirs set.
    pub(in crate::workspace) dir: String,
    pub(in crate::workspace) collapsed: bool,
    pub(in crate::workspace) state: DirStageState,
    /// Repo-root-relative paths the header's checkbox acts on — already
    /// narrowed by `state`, since which side of the index a click moves
    /// depends on it. Always the git-status form so they round-trip into
    /// `git_add` / `git_restore_staged`.
    pub(in crate::workspace) stage_paths: Vec<PathBuf>,
}

/// The rows the dock draws, in draw order, with collapsed groups' files left
/// out — one pass over the change set, shared by the renderer and the keyboard
/// cursor so the two can never disagree about order.
pub(in crate::workspace) fn build_rows(
    status: &crate::lane::git::GitStatusData,
    collapsed: &std::collections::HashSet<String>,
    wt_paths: &LanePaths<'_>,
) -> Vec<GitChangesRow> {
    let unified = build_unified_list(&status.staged, &status.unstaged);
    let groups = group_by_dir(unified, wt_paths);
    let mut rows = Vec::new();
    for (dir_opt, entries) in groups {
        let is_collapsed = dir_opt
            .as_deref()
            .map(|d| collapsed.contains(d))
            .unwrap_or(false);
        if let Some(dir) = dir_opt {
            let state = compute_dir_state(&entries);
            let stage_paths = entries
                .iter()
                .filter(|e| match state {
                    DirStageState::AllStaged => e.staged.is_some(),
                    DirStageState::NoneStaged | DirStageState::Mixed => e.unstaged.is_some(),
                })
                .map(|e| e.path.clone())
                .collect();
            rows.push(GitChangesRow::DirHeader(GitDirHeaderRow {
                state,
                stage_paths,
                dir,
                collapsed: is_collapsed,
            }));
        }
        if is_collapsed {
            continue;
        }
        rows.extend(entries.into_iter().map(GitChangesRow::File));
    }
    rows
}

/// Repo-root-relative paths in the same order the left dock renders them,
/// minus any rows hidden inside a collapsed dir group. Single source of
/// truth for the keyboard cursor's navigation order — it reads the same
/// [`build_rows`] the renderer does, so the two cannot disagree about order.
pub(in crate::workspace) fn ordered_visible_paths(
    status: &crate::lane::git::GitStatusData,
    collapsed: &std::collections::HashSet<String>,
    wt_paths: &LanePaths<'_>,
) -> Vec<PathBuf> {
    build_rows(status, collapsed, wt_paths)
        .into_iter()
        .filter_map(|row| match row {
            GitChangesRow::File(e) => Some(e.path),
            GitChangesRow::DirHeader(_) => None,
        })
        .collect()
}

/// Tracking indicator for the header — `↑N ↓M` when the local branch
/// diverges from its upstream. Returns `None` when both sides are 0
/// (in sync, no upstream, or detached HEAD) so the caller can omit the
/// element entirely.
pub(super) fn tracking_indicator_text(ahead: u32, behind: u32) -> Option<String> {
    match (ahead, behind) {
        (0, 0) => None,
        (a, 0) => Some(format!("↑{a}")),
        (0, b) => Some(format!("↓{b}")),
        (a, b) => Some(format!("↑{a} ↓{b}")),
    }
}

/// Count merge-conflict entries in the unstaged list. `parse_git_status_output`
/// routes anything with a `U` column or both-add / both-delete into
/// `unstaged`, so checking that side is sufficient.
pub(super) fn count_conflicts(unstaged: &[GitFileEntry]) -> usize {
    unstaged
        .iter()
        .filter(|e| matches!((e.x, e.y), ('U', _) | (_, 'U') | ('D', 'D') | ('A', 'A')))
        .count()
}

/// Aggregate staging state of a directory group.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum DirStageState {
    /// Every file in the dir is fully staged (no working-tree leftover).
    AllStaged,
    /// No file in the dir has any staged change.
    NoneStaged,
    /// Mixed — some staged, some unstaged, or files with both staged
    /// and unstaged changes (e.g. `MM`).
    Mixed,
}

pub(super) fn compute_dir_state(entries: &[UnifiedEntry]) -> DirStageState {
    let mut any_staged = false;
    let mut any_unstaged = false;
    for e in entries {
        if e.staged.is_some() {
            any_staged = true;
        }
        if e.unstaged.is_some() {
            any_unstaged = true;
        }
    }
    match (any_staged, any_unstaged) {
        (true, false) => DirStageState::AllStaged,
        (false, true) => DirStageState::NoneStaged,
        (true, true) => DirStageState::Mixed,
        // No changes in this dir — render as if NoneStaged (the dir
        // wouldn't be in the list at all if every entry were empty,
        // but be defensive).
        (false, false) => DirStageState::NoneStaged,
    }
}

/// Whether the "Discard Changes" context-menu item should be disabled
/// for a row in the given (is_staged, has_unstaged) state.
///
/// `git restore` only touches the working tree, so a purely staged row
/// (`M `, `A `, `D `) has nothing to discard without first unstaging —
/// surface that by greying out the item. Untracked (`??`) routes to
/// `git clean -f`, which always has something to discard, so the row is
/// `is_staged = false, has_unstaged = true` and stays enabled. Combined
/// states (`MM`, `MD`, `AM`, etc.) have a working-tree change to discard
/// and stay enabled.
pub(super) fn discard_disabled(is_staged: bool, has_unstaged: bool) -> bool {
    is_staged && !has_unstaged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane::git::{GitFileEntry, GitStatusData};
    use std::collections::HashSet;
    use std::path::Path;

    fn entry(x: char, y: char, path: &str) -> GitFileEntry {
        GitFileEntry {
            x,
            y,
            path: PathBuf::from(path),
            ..Default::default()
        }
    }

    /// `LanePaths` borrows from the caller's `Path` storage. The
    /// helper uses `from_git_status(p)` (repo-root-relative → absolute)
    /// and `strip_prefix_or_self(wt_path).parent()` to derive the
    /// dir-group key. For tests we set `wt_path == repo_root` so the
    /// repo-root-relative input round-trips back to the same string the
    /// render pipeline groups by.
    fn paths_for(root: &Path) -> LanePaths<'_> {
        LanePaths {
            wt_path: root,
            repo_root: Some(root),
        }
    }

    #[test]
    fn ordered_visible_paths_alphabetical_across_groups() {
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![entry('M', ' ', "src/lib.rs")],
            unstaged: vec![
                entry(' ', 'M', "Cargo.toml"),
                entry(' ', 'M', "src/foo/a.rs"),
            ],
            ..Default::default()
        };
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(
            paths,
            vec![
                PathBuf::from("src/lib.rs"),
                PathBuf::from("src/foo/a.rs"),
                PathBuf::from("Cargo.toml"),
            ],
            "directory groups first (alphabetical by dir path), then root \
             entries last; alphabetical within each group"
        );
    }

    #[test]
    fn ordered_visible_paths_skips_collapsed_dirs() {
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![],
            unstaged: vec![
                entry(' ', 'M', "Cargo.toml"),
                entry(' ', 'M', "src/foo/a.rs"),
                entry(' ', 'M', "src/foo/b.rs"),
                entry(' ', 'M', "src/lib.rs"),
            ],
            ..Default::default()
        };
        let mut collapsed: HashSet<String> = HashSet::new();
        collapsed.insert("src/foo".to_string());
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(
            paths,
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("Cargo.toml"),],
            "src/foo entries hidden; src/lib.rs remains as dir group, \
             Cargo.toml stays last as a root entry"
        );
    }

    #[test]
    fn ordered_visible_paths_unifies_staged_and_unstaged() {
        // A file with both staged + unstaged changes (`MM`) appears
        // once in the visible list.
        let root = Path::new("/repo");
        let status = GitStatusData {
            staged: vec![entry('M', 'M', "file.rs")],
            unstaged: vec![entry('M', 'M', "file.rs")],
            ..Default::default()
        };
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert_eq!(paths, vec![PathBuf::from("file.rs")]);
    }

    #[test]
    fn discard_disabled_truth_table() {
        // (is_staged, has_unstaged) → disabled
        let cases = [
            // `M ` / `A ` / `D ` — staged only, nothing in working tree.
            ((true, false), true, "staged-only must be disabled"),
            // `MM` / `MD` / `AM` — staged + working-tree change.
            ((true, true), false, "staged + unstaged must be enabled"),
            // ` M` — unstaged modification.
            ((false, true), false, "unstaged-only must be enabled"),
            // `??` — untracked, `has_unstaged = true` via porcelain Y='?'.
            ((false, true), false, "untracked must be enabled"),
            // Defensive: nothing on either side shouldn't happen, but
            // it logically has nothing to discard either.
            (
                (false, false),
                false,
                "no changes — enabled (no-op fallback)",
            ),
        ];
        for ((is_staged, has_unstaged), expected, msg) in cases {
            assert_eq!(
                discard_disabled(is_staged, has_unstaged),
                expected,
                "{msg}: ({is_staged}, {has_unstaged})"
            );
        }
    }

    #[test]
    fn ordered_visible_paths_empty_when_no_changes() {
        let root = Path::new("/repo");
        let status = GitStatusData::default();
        let collapsed: HashSet<String> = HashSet::new();
        let paths = ordered_visible_paths(&status, &collapsed, &paths_for(root));
        assert!(paths.is_empty());
    }
}
