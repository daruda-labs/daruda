//! Tree-flatten walker — produces the linear `VisibleEntry` list that
//! the sidebar `uniform_list` consumes.
//!
//! Also owns `VisibleEntry` (flattened sidebar row) and
//! `build_status_index` (git-status collapse). Free functions over
//! `&FileTree`; no `Workspace` access.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::files::gitignore::GitignoreSet;
use crate::files::tree::{Entry, EntryId, EntryKind, FileTree};
use crate::worktree::git::GitStatusData;

// ----------------------------------------------------------------
// VisibleEntry — flattened row for `uniform_list`
// ----------------------------------------------------------------

/// One row of the linear visible list. All fields are owned so the
/// `Arc<Vec<VisibleEntry>>` can move into a `'static` `uniform_list`
/// closure.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct VisibleEntry {
    pub entry_id: EntryId,
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
    pub is_symlink: bool,
    pub is_ignored: bool,
    /// Depth in the tree. Root's children are depth 0; their children
    /// depth 1; etc.
    pub depth: usize,
    pub is_expanded: bool,
    /// Keyboard cursor is on this row — drives the selected background.
    /// Mouse click also moves the cursor here, so the visible
    /// "selected" row always tracks the user's last interaction
    /// regardless of which file viewer pane currently has focus.
    pub is_keyboard_focused: bool,
    /// Status character from `git status --porcelain`. `None` when the
    /// path is absent from the cache. Used by the row badge.
    pub git_status: Option<char>,
}

// ----------------------------------------------------------------
// status_index — flatten GitStatusData into a path → char HashMap
// ----------------------------------------------------------------

/// Build a worktree-relative `path → status char` index. `None`
/// returns an empty map.
///
/// Staged status wins over unstaged when a path appears in both.
pub(in crate::workspace) fn build_status_index(
    status: Option<&GitStatusData>,
) -> HashMap<PathBuf, char> {
    let mut idx: HashMap<PathBuf, char> = HashMap::new();
    let Some(s) = status else { return idx };
    for e in &s.staged {
        idx.insert(e.path.clone(), e.x);
    }
    for e in &s.unstaged {
        idx.entry(e.path.clone()).or_insert(e.y);
    }
    idx
}

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_into(
    tree: &FileTree,
    parent_id: EntryId,
    parent_depth: usize,
    status_index: &HashMap<PathBuf, char>,
    keyboard_focus: Option<EntryId>,
    gitignore: Option<&GitignoreSet>,
    show_hidden: bool,
    out: &mut Vec<VisibleEntry>,
) {
    let emit_self = parent_id != tree.root_id;
    if emit_self && let Some(entry) = tree.entry(parent_id) {
        out.push(visible_from(
            entry,
            tree,
            parent_depth,
            status_index,
            keyboard_focus,
            gitignore,
        ));
    }
    let child_depth = if emit_self {
        parent_depth + 1
    } else {
        parent_depth
    };
    let parent_expanded = parent_id == tree.root_id || tree.is_expanded(parent_id);
    if !parent_expanded {
        return;
    }
    for child in tree.child_entries(parent_id) {
        if !show_hidden && child.is_hidden {
            continue;
        }
        if child.kind.is_dir() {
            walk_into(
                tree,
                child.id,
                child_depth,
                status_index,
                keyboard_focus,
                gitignore,
                show_hidden,
                out,
            );
        } else {
            out.push(visible_from(
                child,
                tree,
                child_depth,
                status_index,
                keyboard_focus,
                gitignore,
            ));
        }
    }
}

pub(super) fn visible_from(
    entry: &Entry,
    tree: &FileTree,
    depth: usize,
    status_index: &HashMap<PathBuf, char>,
    keyboard_focus: Option<EntryId>,
    gitignore: Option<&GitignoreSet>,
) -> VisibleEntry {
    let is_ignored = gitignore
        .map(|g| g.is_ignored(&tree.root.join(&entry.path), entry.kind.is_dir()))
        .unwrap_or(false);
    VisibleEntry {
        entry_id: entry.id,
        path: entry.path.clone(),
        name: entry.name.clone(),
        kind: entry.kind,
        is_symlink: entry.is_symlink,
        is_ignored,
        depth,
        is_expanded: entry.kind.is_dir() && tree.is_expanded(entry.id),
        is_keyboard_focused: keyboard_focus == Some(entry.id),
        git_status: status_index.get(&entry.path).copied(),
    }
}
