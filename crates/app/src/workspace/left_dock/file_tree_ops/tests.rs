use std::collections::HashMap;

use super::*;
use crate::files::tree::LoadedEntry;
use crate::lane::git::{GitFileEntry, GitStatusData};

fn loaded(name: &str, kind: EntryKind) -> LoadedEntry {
    LoadedEntry {
        name: name.to_string(),
        kind,
        is_symlink: false,
    }
}

#[test]
fn build_status_index_empty_when_status_none() {
    let idx = build_status_index(None);
    assert!(idx.is_empty());
}

#[test]
fn build_status_index_staged_overrides_unstaged() {
    let status = GitStatusData {
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
    let idx = build_status_index(Some(&status));
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

#[test]
fn event_has_non_git_path_skips_pure_git_changes() {
    let ev = DebouncedEvent::Changed {
        paths: vec![
            PathBuf::from("/repo/.git/index"),
            PathBuf::from("/repo/.git/refs/heads/main"),
        ],
    };
    assert!(!event_has_non_git_path(&ev));
}

#[test]
fn event_has_non_git_path_triggers_on_mixed_paths() {
    let ev = DebouncedEvent::Changed {
        paths: vec![
            PathBuf::from("/repo/.git/index"),
            PathBuf::from("/repo/src/main.rs"),
        ],
    };
    assert!(event_has_non_git_path(&ev));
}

#[test]
fn event_has_non_git_path_triggers_on_working_tree_changes() {
    let ev = DebouncedEvent::Removed {
        paths: vec![PathBuf::from("/repo/Cargo.toml")],
    };
    assert!(event_has_non_git_path(&ev));
}

#[test]
fn event_has_non_git_path_bulk_defaults_true() {
    assert!(event_has_non_git_path(&DebouncedEvent::Bulk));
}

#[test]
fn event_has_non_git_path_error_returns_false() {
    assert!(!event_has_non_git_path(&DebouncedEvent::Error(
        "kernel queue overflow".into()
    )));
}

#[test]
fn path_is_inside_git_dir_matches_nested_and_root_level() {
    assert!(path_is_inside_git_dir(Path::new("/repo/.git/HEAD")));
    assert!(path_is_inside_git_dir(Path::new(
        "/repo/.git/objects/aa/bbcc"
    )));
    assert!(!path_is_inside_git_dir(Path::new("/repo/src/main.rs")));
    // `.gitignore` must not be confused with `.git/`.
    assert!(!path_is_inside_git_dir(Path::new("/repo/.gitignore")));
}
