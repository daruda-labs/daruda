use super::*;
use daruda_agent::hooks::status_file as sf;

#[gpui::test]
fn an_exited_pty_retires_its_route_and_in_memory_status(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let terminal = crate::hooks::agent_hooks::mint_terminal_id();
    let dir = tempfile::tempdir().unwrap();
    let mut status = sf::StatusFile::new_hook(
        "exiting-pty",
        dir.path(),
        daruda_agent::SessionStatus::Working,
        "UserPromptSubmit",
    );
    status.terminal_id = Some(terminal.clone());
    let path = sf::path_for(dir.path(), status.storage_key());
    sf::write_atomic(&path, &status).unwrap();
    workspace.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        crate::runs::RunsHost::register_terminal(ws.run_pane(pane), terminal.clone(), cx);
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        ws.note_terminal_exited(pane, cx);
        assert!(ws.claude_sessions.status.get("exiting-pty").is_none());
        assert!(!ws.claude_sessions.pty_bindings.contains_key(&pane));
        assert!(crate::runs::RunsHost::terminal_pane(&terminal, cx).is_none());
        assert!(path.exists());
    });
}

#[gpui::test]
fn routed_process_exit_clears_status_without_deleting_the_terminal_file(
    cx: &mut gpui::TestAppContext,
) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let dir = tempfile::tempdir().unwrap();
    let mut status = sf::StatusFile::new_hook(
        "exiting-agent",
        dir.path(),
        daruda_agent::SessionStatus::Working,
        "UserPromptSubmit",
    );
    status.terminal_id = Some(crate::hooks::agent_hooks::mint_terminal_id());
    status.agent_pid = Some(42);
    let path = sf::path_for(dir.path(), status.storage_key());
    sf::write_atomic(&path, &status).unwrap();
    workspace.update(cx, |ws, cx| {
        ws.claude_sessions.status.update(status);
        ws.apply_pty_tracker_event(
            ProcessExited {
                session_id: "exiting-agent".into(),
                pid: 41,
            },
            cx,
        );
        assert!(
            ws.claude_sessions.status.get("exiting-agent").is_some(),
            "an older process cannot remove its successor"
        );
        ws.apply_pty_tracker_event(
            ProcessExited {
                session_id: "exiting-agent".into(),
                pid: 42,
            },
            cx,
        );
        assert!(ws.claude_sessions.status.get("exiting-agent").is_none());
        assert!(path.exists());
    });
}

#[gpui::test]
fn terminal_session_switch_interrupt_and_removal_share_the_routed_identity(
    cx: &mut gpui::TestAppContext,
) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let dir = tempfile::tempdir().unwrap();
    let terminal = crate::hooks::agent_hooks::mint_terminal_id();
    let mut status = sf::StatusFile::new_hook(
        "first",
        dir.path(),
        daruda_agent::SessionStatus::Working,
        "UserPromptSubmit",
    );
    status.terminal_id = Some(terminal.clone());
    let path = sf::path_for(dir.path(), status.storage_key());
    workspace.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        crate::runs::RunsHost::register_terminal(ws.run_pane(pane), terminal.clone(), cx);
        sf::write_atomic(&path, &status).unwrap();
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        ws.interrupt_terminal_agent(pane, cx);
        assert_eq!(
            ws.claude_sessions.status.get("first").unwrap().status,
            daruda_agent::SessionStatus::Idle
        );
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        assert_eq!(
            ws.claude_sessions.status.get("first").unwrap().status,
            daruda_agent::SessionStatus::Idle,
            "an old watcher replay cannot undo the interrupt"
        );
        status.session_id = "second".into();
        status.last_event = "SessionStart".into();
        status.timestamp = chrono::Utc::now();
        sf::write_atomic(&path, &status).unwrap();
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        assert!(ws.claude_sessions.status.get("first").is_none());
        assert_eq!(ws.claude_sessions.pty_bindings[&pane].session_id, "second");
        sf::delete(&path).unwrap();
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Removed {
                key: terminal.as_str().into(),
            },
            cx,
        );
        assert!(ws.claude_sessions.status.get("second").is_none());
        assert!(ws.session_statuses(cx).is_empty());
        status.timestamp = chrono::Utc::now();
        sf::write_atomic(&path, &status).unwrap();
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        ws.release_pane_tracking(&[pane], cx);
        assert!(ws.claude_sessions.status.get("second").is_none());
        assert!(crate::runs::RunsHost::terminal_pane(&terminal, cx).is_none());
        assert!(
            path.exists(),
            "pane teardown leaves disk cleanup to the handler and startup"
        );
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        assert!(ws.claude_sessions.status.get("second").is_none());
    });
}

#[gpui::test]
fn routed_hooks_bind_without_a_pid(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let dir = tempfile::tempdir().unwrap();
    let terminal = crate::hooks::agent_hooks::mint_terminal_id();
    let mut status = sf::StatusFile::new_hook(
        "routed-session",
        "/tmp/cli",
        daruda_agent::SessionStatus::Working,
        "UserPromptSubmit",
    );
    status.terminal_id = Some(terminal.clone());
    status.harness = Some(daruda_agent::hooks::route::Harness::Codex);
    let path = sf::path_for(dir.path(), &status.session_id);
    sf::write_atomic(&path, &status).unwrap();
    workspace.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        crate::runs::RunsHost::register_terminal(ws.run_pane(pane), terminal, cx);
        ws.apply_claude_status_event(crate::hooks::watcher::StatusEvent::Changed(path), cx);
        let binding = &ws.claude_sessions.pty_bindings[&pane];
        assert_eq!(binding.session_id, status.session_id);
        assert_eq!(binding.claude_pid, None);
        assert!(
            ws.session_statuses(cx)
                .contains(&(status.storage_key().to_owned(), status.status))
        );
    });
}

#[gpui::test]
fn routed_hooks_ignore_other_windows_and_retired_terminals(cx: &mut gpui::TestAppContext) {
    let (_, first) = crate::workspace::tests::build_workspace(cx);
    let (_, second) = crate::workspace::tests::build_workspace(cx);
    let dir = tempfile::tempdir().unwrap();
    let terminal = crate::hooks::agent_hooks::mint_terminal_id();
    let mut status = sf::StatusFile::new_hook(
        "owned-session",
        "/tmp/cli",
        daruda_agent::SessionStatus::Working,
        "UserPromptSubmit",
    );
    status.terminal_id = Some(terminal.clone());
    let path = sf::path_for(dir.path(), &status.session_id);
    sf::write_atomic(&path, &status).unwrap();
    first.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        crate::runs::RunsHost::register_terminal(ws.run_pane(pane), terminal, cx);
    });
    second.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        ws.claude_sessions.status.load_initial(vec![status.clone()]);
        assert!(!ws.claude_sessions.pty_bindings.contains_key(&pane));
        assert!(
            !ws.session_statuses(cx)
                .iter()
                .any(|(id, _)| id == &status.session_id)
        );
        ws.claude_sessions.status.remove(&status.session_id);
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        assert!(ws.claude_sessions.status.get(&status.session_id).is_none());
    });
    first.update(cx, |ws, cx| {
        let pane = ws.active_runtime().focused_pane_id;
        crate::runs::RunsHost::unregister_terminal(ws.run_pane(pane), cx);
        ws.apply_claude_status_event(crate::hooks::watcher::StatusEvent::Changed(path), cx);
        assert!(ws.claude_sessions.status.get(&status.session_id).is_none());
    });
}

#[gpui::test]
fn exited_cli_clears_memory_without_unlinking_a_writers_files(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let session = format!("lock-retention-{}", uuid::Uuid::new_v4());
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path().to_path_buf();
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
            ProcessExited {
                session_id: session.clone(),
                pid: 42,
            },
            cx,
        );
        assert!(ws.claude_sessions.status.get(&session).is_none());
    });
    assert!(path.exists());
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

#[gpui::test]
fn ending_one_terminal_preserves_another_with_the_same_session(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let dir = tempfile::tempdir().unwrap();
    workspace.update(cx, |ws, cx| {
        let mut files = Vec::new();
        for (pane, pid) in [(100, 41), (101, 42)] {
            let terminal = crate::hooks::agent_hooks::mint_terminal_id();
            crate::runs::RunsHost::register_terminal(ws.run_pane(pane), terminal.clone(), cx);
            let mut file = sf::StatusFile::new_hook(
                "resumed",
                dir.path(),
                daruda_agent::SessionStatus::Working,
                "UserPromptSubmit",
            );
            file.terminal_id = Some(terminal);
            file.agent_pid = Some(pid);
            let path = sf::path_for(dir.path(), file.storage_key());
            sf::write_atomic(&path, &file).unwrap();
            ws.apply_claude_status_event(
                crate::hooks::watcher::StatusEvent::Changed(path.clone()),
                cx,
            );
            files.push((file, path));
        }
        assert_eq!(ws.claude_sessions.status.len(), 2);
        let (file, path) = &mut files[0];
        file.last_event = "SessionEnd".into();
        file.timestamp = chrono::Utc::now();
        sf::write_atomic(path, file).unwrap();
        ws.apply_claude_status_event(
            crate::hooks::watcher::StatusEvent::Changed(path.clone()),
            cx,
        );
        assert!(!ws.claude_sessions.pty_bindings.contains_key(&100));
        assert!(ws.claude_sessions.pty_bindings.contains_key(&101));
        assert_eq!(ws.claude_sessions.status.len(), 1);
        ws.apply_pty_tracker_event(
            ProcessExited {
                session_id: "resumed".into(),
                pid: 41,
            },
            cx,
        );
        assert!(ws.claude_sessions.pty_bindings.contains_key(&101));
        assert_eq!(
            ws.claude_sessions
                .status
                .get_key(files[1].0.storage_key())
                .unwrap()
                .status,
            daruda_agent::SessionStatus::Working
        );
    });
}
