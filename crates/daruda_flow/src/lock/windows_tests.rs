use super::*;

#[test]
fn network_prefix_encoding_is_stable_across_rust_versions() {
    let tree = CanonicalTree::unchecked(r"\\s\h\repo".into());
    assert_eq!(
        lock_dir_for(Path::new("locks"), &tree),
        Path::new("locks").join("unc-0073-0068").join("repo")
    );
}

#[test]
fn resolved_tree_can_acquire_and_release_a_lock() {
    let tree = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let resolved = CanonicalTree::resolve(tree.path()).unwrap();
    let directory = lock_dir_for(root.path(), &resolved);
    assert!(directory.starts_with(root.path()));
    let lock = RunLock::acquire(&directory, "windows", &|_| true).unwrap();
    assert!(directory.join(LOCK_FILE).is_file());
    assert!(RunLock::acquire(&directory, "other", &|_| true).is_err());
    lock.release().unwrap();
    RunLock::acquire(&directory, "other", &|_| true)
        .unwrap()
        .release()
        .unwrap();
}

#[test]
fn equivalent_prefixes_share_a_lock_but_distinct_volumes_do_not() {
    let root = tempfile::tempdir().unwrap();
    let directory = |path: &str| lock_dir_for(root.path(), &CanonicalTree::unchecked(path.into()));
    assert_eq!(directory(r"C:\repo"), directory(r"\\?\C:\repo"));
    assert_eq!(
        directory(r"\\server\share\repo"),
        directory(r"\\?\UNC\server\share\repo")
    );
    let paths = [
        r"C:\repo",
        r"D:\repo",
        r"\\server-a\share\repo",
        r"\\server\a-share\repo",
        r"\\server\other\repo",
    ];
    let directories: std::collections::BTreeSet<_> =
        paths.iter().map(|path| directory(path)).collect();
    assert_eq!(directories.len(), paths.len());
    for path in directories {
        assert!(path.starts_with(root.path()));
        std::fs::create_dir_all(path).unwrap();
    }
}
