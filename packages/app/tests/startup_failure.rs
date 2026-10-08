//! Fatal startup failures must be observable even when the process exits at once.

use daruda_core::process_env;

#[test]
fn corrupt_workspace_input_is_reported_and_persisted_before_exit() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("recent-workspaces.json");
    std::fs::write(&source, "{ broken").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_daruda"))
        .arg("--smoke")
        .env(process_env::DATA_DIR.name(), root.path())
        .env(process_env::PROFILE.name(), "startup-failure-test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recent-workspaces.json"), "{stdout}");
    let logs: String = std::fs::read_dir(root.path().join("logs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("daruda-"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect();
    assert!(
        logs.contains("Workspace storage preparation failed"),
        "{logs}"
    );
    assert!(logs.contains("recent-workspaces.json"), "{logs}");
    assert_eq!(std::fs::read_to_string(source).unwrap(), "{ broken");
    assert!(!root.path().join("state/workspace").exists());
}

#[test]
fn unavailable_log_directory_does_not_hide_the_original_failure() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("recent-workspaces.json"), "{ broken").unwrap();
    std::fs::write(root.path().join("logs"), "preserve").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_daruda"))
        .arg("--smoke")
        .env(process_env::DATA_DIR.name(), root.path())
        .env(process_env::PROFILE.name(), "startup-failure-test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recent-workspaces.json"), "{stdout}");
    assert!(stdout.contains("logs"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("logs")).unwrap(),
        "preserve"
    );
}
