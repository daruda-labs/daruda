//! Fatal startup failures must be observable even when the process exits at once.

use daruda_core::process_env;

#[test]
fn a_forwarded_launch_never_opens_or_reconciles_the_task_log() {
    use std::io::{Read as _, Write as _};
    use std::net::{Ipv4Addr, TcpListener};

    let root = tempfile::tempdir().unwrap();
    let lock = std::fs::File::create(root.path().join("desktop.lock")).unwrap();
    lock.lock().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = serde_json::json!({
        "port": listener.local_addr().unwrap().port(),
        "token": "test-forward-token",
        "pid": std::process::id(),
    });
    std::fs::write(
        root.path().join("desktop.json"),
        serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    let receiver = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < std::time::Duration::from_secs(10) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("forwarded request did not arrive: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request["token"], "test-forward-token");
        stream.write_all(b"ok").unwrap();
    });
    std::fs::write(root.path().join("tasks.json"), b"preserve legacy bytes").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_daruda"))
        .env(process_env::DATA_DIR.name(), root.path())
        .env(process_env::PROFILE.name(), "forwarded-task-test")
        .output()
        .unwrap();
    assert!(receiver.join().is_ok(), "{output:?}");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(root.path().join("tasks.json")).unwrap(),
        b"preserve legacy bytes"
    );
    assert!(!root.path().join("state").exists());
    assert!(!root.path().join("task-log").exists());
}

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
