//! Public API tests for paths.

use daruda_project::lane::paths::LanePaths;
use std::path::{Path, PathBuf};

#[test]
fn from_git_status_uses_repo_root() {
    let wt = Path::new("/term/daruda");
    let repo = Path::new("/term");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: Some(repo),
    };
    assert_eq!(
        wp.from_git_status(Path::new("daruda/crates/foo.rs")),
        PathBuf::from("/term/daruda/crates/foo.rs")
    );
}

#[test]
fn from_git_status_falls_back_to_wt_when_no_repo() {
    let wt = Path::new("/projects/myapp");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: None,
    };
    assert_eq!(
        wp.from_git_status(Path::new("src/main.rs")),
        PathBuf::from("/projects/myapp/src/main.rs")
    );
}

#[test]
fn from_files_tree_always_uses_wt_path() {
    let wt = Path::new("/term/daruda");
    let repo = Path::new("/term");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: Some(repo),
    };
    assert_eq!(
        wp.from_files_tree(Path::new("crates/foo.rs")),
        PathBuf::from("/term/daruda/crates/foo.rs")
    );
}

#[test]
fn to_wt_relative_strips_prefix() {
    let wt = Path::new("/term/daruda");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: None,
    };
    assert_eq!(
        wp.to_wt_relative(Path::new("/term/daruda/crates/foo.rs")),
        Some(PathBuf::from("crates/foo.rs"))
    );
}

#[test]
fn to_wt_relative_returns_none_outside_wt() {
    let wt = Path::new("/term/daruda");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: None,
    };
    assert!(wp.to_wt_relative(Path::new("/other/file.rs")).is_none());
}

#[test]
fn to_repo_relative_strips_repo_prefix() {
    let wt = Path::new("/term/daruda");
    let repo = Path::new("/term");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: Some(repo),
    };
    assert_eq!(
        wp.to_repo_relative(Path::new("/term/daruda/crates/foo.rs")),
        Some(PathBuf::from("daruda/crates/foo.rs"))
    );
}

#[test]
fn to_repo_relative_none_for_non_git() {
    let wt = Path::new("/projects/myapp");
    let wp = LanePaths {
        wt_path: wt,
        repo_root: None,
    };
    assert!(
        wp.to_repo_relative(Path::new("/projects/myapp/src/main.rs"))
            .is_none()
    );
}

#[test]
fn round_trip_same_path_when_wt_equals_repo() {
    let root = Path::new("/term");
    let wp = LanePaths {
        wt_path: root,
        repo_root: Some(root),
    };
    let git_rel = Path::new("crates/foo.rs");
    let abs = wp.from_git_status(git_rel);
    assert_eq!(abs, PathBuf::from("/term/crates/foo.rs"));
    assert_eq!(wp.to_wt_relative(&abs), Some(git_rel.to_path_buf()));
    assert_eq!(wp.to_repo_relative(&abs), Some(git_rel.to_path_buf()));
}
