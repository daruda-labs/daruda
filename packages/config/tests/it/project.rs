//! Public API tests for project.

use daruda_config::project::*;
use std::path::{Path, PathBuf};

#[test]
fn project_id_is_deterministic() {
    let p = PathBuf::from("/tmp/foo");
    assert_eq!(project_id(&p), project_id(&p));
}

#[test]
fn project_id_differs_for_different_paths() {
    let a = PathBuf::from("/tmp/foo");
    let b = PathBuf::from("/tmp/bar");
    assert_ne!(project_id(&a), project_id(&b));
}

#[test]
fn project_id_canonicalizes_dot_segments() {
    // `/tmp/./foo` canonicalises to `/tmp/foo` when both exist;
    // the test creates the dir so canonicalize succeeds and the
    // two paths collapse to one id.
    let temp = tempfile::tempdir().unwrap();
    let direct = temp.path().to_path_buf();
    let with_dot = temp.path().join(".").to_path_buf();
    assert_eq!(project_id(&direct), project_id(&with_dot));
}

#[test]
fn load_for_returns_default_when_path_missing() {
    let temp = tempfile::tempdir().unwrap();
    // No config.toml written — load_for must yield default.
    let cfg = ProjectConfig::load_for(temp.path());
    assert!(cfg.shell.is_none());
}

#[test]
fn load_from_returns_default_when_file_unreadable() {
    let cfg = ProjectConfig::load_from(Path::new("/nonexistent/config.toml"));
    assert!(cfg.shell.is_none());
}

#[test]
fn load_from_parses_shell_section() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project.toml");
    std::fs::write(
        &path,
        "[shell]\nprogram = \"/usr/local/bin/zsh\"\nclose_pane_on_exit = false\n",
    )
    .unwrap();
    let cfg = ProjectConfig::load_from(&path);
    let shell = cfg.shell.expect("shell section parses");
    assert_eq!(shell.program.as_deref(), Some("/usr/local/bin/zsh"));
    assert!(!shell.close_pane_on_exit);
}

#[test]
fn load_from_corrupt_toml_falls_back_to_default() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project.toml");
    std::fs::write(&path, "this is = not [valid toml").unwrap();
    let cfg = ProjectConfig::load_from(&path);
    assert!(cfg.shell.is_none(), "corrupt files must not panic");
}

#[test]
fn the_template_alone_overrides_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, PROJECT_CONFIG_TEMPLATE).unwrap();
    assert!(!ProjectConfig::load_from(&path).overrides_anything());
}

#[test]
fn a_shell_section_is_an_override() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[shell]\nprogram = \"/bin/zsh\"\n").unwrap();
    assert!(ProjectConfig::load_from(&path).overrides_anything());
}
