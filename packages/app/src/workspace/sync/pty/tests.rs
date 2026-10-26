use super::*;
use daruda_agent::hooks::status_file as sf;

#[gpui::test]
fn exited_cli_removes_status_without_unlinking_a_writers_lock(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let session = format!("lock-retention-{}", uuid::Uuid::new_v4());
    let dir = sf::default_dir().unwrap();
    let path = sf::path_for(&dir, &session);
    let lock_path = sf::lock_path_for(&dir, &session);
    let status = sf::StatusFile::new_hook(
        &session,
        "/tmp/cli",
        daruda_agent::SessionStatus::Working,
        "PreToolUse",
    );
    sf::write_atomic(&path, &status).unwrap();
    let held = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    held.lock().unwrap();
    workspace.update(cx, |ws, cx| {
        ws.claude_sessions.status.update(status);
        ws.apply_pty_tracker_event(
            PtyTrackerEvent::SessionProcessExited {
                session_id: session.clone(),
                claude_pid: 42,
            },
            cx,
        );
        assert!(ws.claude_sessions.status.get(&session).is_none());
    });
    assert!(!path.exists());
    assert!(
        lock_path.exists(),
        "an exited CLI can leave queued hook writers"
    );
    let contender = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(matches!(
        contender.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(contender);
    drop(held);
    std::fs::remove_file(lock_path).unwrap();
}
