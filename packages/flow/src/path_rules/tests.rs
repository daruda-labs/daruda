//! Native path syntax; Windows prefixes must never replace a join's root.

use super::*;

#[test]
fn relative_paths_keep_nested_names_and_reject_traversal() {
    for path in [".", "./reports/out.md", "reports/한글 ' $ `/out.md"] {
        assert!(stays_relative(Path::new(path)), "{path}");
    }
    for path in ["../out.md", "reports/../../out.md", "/out.md"] {
        assert!(!stays_relative(Path::new(path)), "{path}");
    }
}

#[cfg(windows)]
#[test]
fn windows_prefixes_and_roots_cannot_be_relative_children() {
    for path in [
        r"C:out.md",
        r"C:\out.md",
        r"\out.md",
        r"\\server\share\out.md",
        r"\\?\C:\out.md",
    ] {
        assert!(!stays_relative(Path::new(path)), "{path}");
    }
}

#[test]
fn cwd_must_be_an_existing_directory_inside_the_root() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    let nested = root.path().join("nested 한글");
    std::fs::create_dir(&nested).expect("directory");
    assert!(check_cwd(root.path(), &nested).is_ok());
    assert!(matches!(
        check_cwd(root.path(), outside.path()),
        Err(CwdError::Outside { .. })
    ));
    assert!(check_cwd(root.path(), &root.path().join("missing")).is_err());
    let file = root.path().join("file");
    std::fs::write(&file, "contents").expect("file");
    assert!(check_cwd(root.path(), &file).is_err());
}
