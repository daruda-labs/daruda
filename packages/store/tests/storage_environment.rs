//! Exercise public storage APIs in a fresh process without mutating env in tests.

use daruda_core::process_env;
use daruda_store::{observability::log_writer, persistence};

#[test]
fn public_storage_apis_capture_one_override_namespace() {
    let temporary = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "storage_snapshot_child",
            "--nocapture",
        ])
        .env(process_env::DATA_DIR.name(), "isolated")
        .env(process_env::PROFILE.name(), "preview")
        .env("STORAGE_TEST_CHILD", "1")
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temporary.path().join("isolated/logs").is_dir());
    assert!(temporary.path().join("isolated/diagnostics.zip").is_file());
    assert!(
        temporary
            .path()
            .join("isolated/state/workspace/recent-workspaces.json")
            .is_file()
    );
}

#[test]
#[ignore = "runs in a subprocess with an isolated environment"]
fn storage_snapshot_child() {
    if std::env::var_os("STORAGE_TEST_CHILD").is_none() {
        return;
    }
    let original_cwd = std::env::current_dir().unwrap();
    let expected = original_cwd.join("isolated");
    assert_eq!(persistence::default_data_dir(), expected);
    let other = tempfile::tempdir().unwrap();
    std::env::set_current_dir(other.path()).unwrap();
    assert_eq!(persistence::default_data_dir(), expected);
    assert_eq!(persistence::node_install_dir(), expected.join("node"));
    assert_eq!(persistence::flow_lock_root(), expected.join("flow-locks"));
    assert_eq!(
        persistence::remote_lock_root(),
        expected.join("remote-locks")
    );
    assert_eq!(log_writer::log_dir(), Some(expected.join("logs")));
    assert_eq!(log_writer::log_profile(), "preview");
    let report = daruda_store::observability::error_report::ErrorReport::new("test panic").build();
    let written = log_writer::write_panic_log(&report).unwrap();
    assert!(written.starts_with(expected.join("logs")));
    daruda_store::observability::diagnostics::export_current(
        &expected.join("diagnostics.zip"),
        chrono::Utc::now(),
    )
    .unwrap();
    let uuid = daruda_store::project::WorkspaceUuid::new();
    daruda_store::project::touch_recent_in(&expected, uuid, "legacy".into()).unwrap();
    let workspace_store = daruda_store::project::WorkspaceStore::open_current().unwrap();
    assert_eq!(workspace_store.load_recent()[0].display_name, "legacy");
    workspace_store
        .touch_recent(uuid, "new storage".into())
        .unwrap();
    assert_eq!(
        daruda_store::project::load_recent_in(&expected)[0].display_name,
        "legacy"
    );
    let reopened = daruda_store::project::WorkspaceStore::open_current().unwrap();
    assert_eq!(reopened.load_recent()[0].display_name, "new storage");
    std::env::set_current_dir(original_cwd).unwrap();
}
