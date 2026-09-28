use std::collections::HashMap;

use super::*;
use crate::files::tree::LoadedEntry;
use crate::lane::git::{GitFileEntry, GitWorktreeStatus};

fn loaded(name: &str, kind: EntryKind) -> LoadedEntry {
    LoadedEntry {
        name: name.to_string(),
        kind,
        is_symlink: false,
    }
}

fn lane_at(wt: &'static str, repo: &'static str) -> crate::lane::paths::LanePaths<'static> {
    crate::lane::paths::LanePaths {
        wt_path: std::path::Path::new(wt),
        repo_root: Some(std::path::Path::new(repo)),
    }
}

#[test]
fn build_status_index_empty_when_status_none() {
    let idx = build_status_index(None, &lane_at("/r", "/r"));
    assert!(idx.is_empty());
}

/// git names paths from the repository root; the tree names them from the
/// lane. For a lane opened at `repo/sub` every key used to miss.
#[test]
fn a_subdirectory_lane_keys_its_status_lane_relative() {
    let entry = |path: &str| GitFileEntry {
        x: 'M',
        y: ' ',
        path: PathBuf::from(path),
        ..Default::default()
    };
    let status = GitWorktreeStatus {
        staged: vec![entry("sub/a.rs"), entry("other/b.rs")],
        ..Default::default()
    };
    let idx = build_status_index(Some(&status), &lane_at("/repo/sub", "/repo"));
    assert_eq!(idx.len(), 1, "a change outside the lane is not the lane's");
    assert_eq!(idx.get(&PathBuf::from("a.rs")).copied(), Some('M'));
}

#[test]
fn build_status_index_staged_overrides_unstaged() {
    let status = GitWorktreeStatus {
        staged: vec![GitFileEntry {
            x: 'M',
            y: ' ',
            path: PathBuf::from("a.txt"),
            ..Default::default()
        }],
        unstaged: vec![GitFileEntry {
            x: ' ',
            y: 'M',
            path: PathBuf::from("a.txt"),
            ..Default::default()
        }],
        ..Default::default()
    };
    let idx = build_status_index(Some(&status), &lane_at("/r", "/r"));
    // Staged char wins for paths in both lists.
    assert_eq!(idx.get(&PathBuf::from("a.txt")).copied(), Some('M'));
}

#[test]
fn flatten_walks_only_expanded_children() {
    // Build a small tree by hand to avoid filesystem dependency.
    let mut tree = FileTree::new(PathBuf::from("/tmp/wt"));
    let root = tree.root_id;
    let a = tree.insert_child(root, loaded("a", EntryKind::Dir));
    let _b = tree.insert_child(root, loaded("b", EntryKind::File));
    let _aa = tree.insert_child(a, loaded("aa", EntryKind::File));
    tree.sort_children(root);
    tree.sort_children(a);

    // Without expanding `a`, only direct children are visible.
    let mut out = Vec::new();
    walk_into(
        &tree,
        tree.root_id,
        0,
        &HashMap::new(),
        None,
        None,
        true,
        &mut out,
    );
    let names: Vec<&str> = out.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);

    // Expand `a` → `aa` becomes visible.
    tree.toggle_expand(a);
    let mut out = Vec::new();
    walk_into(
        &tree,
        tree.root_id,
        0,
        &HashMap::new(),
        None,
        None,
        true,
        &mut out,
    );
    let names: Vec<&str> = out.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["a", "aa", "b"]);
}

#[test]
fn flatten_assigns_status_from_index() {
    let mut tree = FileTree::new(PathBuf::from("/tmp/wt"));
    let root = tree.root_id;
    let _a = tree.insert_child(root, loaded("a.txt", EntryKind::File));
    tree.sort_children(root);
    let mut idx = HashMap::new();
    idx.insert(PathBuf::from("a.txt"), 'M');
    let mut out = Vec::new();
    walk_into(&tree, tree.root_id, 0, &idx, None, None, true, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].git_status, Some('M'));
}

#[test]
fn flatten_marks_keyboard_focused_row() {
    let mut tree = FileTree::new(PathBuf::from("/tmp/wt"));
    let root = tree.root_id;
    let a = tree.insert_child(root, loaded("a.txt", EntryKind::File));
    let _b = tree.insert_child(root, loaded("b.txt", EntryKind::File));
    tree.sort_children(root);
    let mut out = Vec::new();
    walk_into(
        &tree,
        tree.root_id,
        0,
        &HashMap::new(),
        Some(a),
        None,
        true,
        &mut out,
    );
    assert_eq!(out.len(), 2);
    let by_name: HashMap<&str, &VisibleEntry> = out.iter().map(|v| (v.name.as_str(), v)).collect();
    assert!(by_name["a.txt"].is_keyboard_focused);
    assert!(!by_name["b.txt"].is_keyboard_focused);
}
