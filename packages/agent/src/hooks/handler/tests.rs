use super::*;
use serde_json::json;

fn event(name: &str, session: &str) -> HookEvent {
    serde_json::from_value(json!({
        "hook_event_name": name, "session_id": session, "cwd": "/work",
        "source": "startup"
    }))
    .unwrap()
}

#[test]
fn one_terminal_owns_one_file_and_old_session_end_cannot_delete_its_successor() {
    let dir = tempfile::tempdir().unwrap();
    let route = HookRoute {
        terminal: super::super::route::TerminalId::parse("terminal-a").unwrap(),
        status_dir: dir.path().to_path_buf(),
    };
    for session in ["first", "second"] {
        handle_event(
            &route,
            Harness::Claude,
            &event("SessionStart", session),
            None,
        )
        .unwrap();
    }
    let path = path_for(dir.path(), route.terminal.as_str());
    assert_eq!(read(&path).unwrap().unwrap().session_id, "second");
    assert_eq!(status_file::list_dir(dir.path()).unwrap().len(), 1);
    handle_event(&route, Harness::Claude, &event("SessionEnd", "first"), None).unwrap();
    assert!(path.exists());
    handle_event(
        &route,
        Harness::Claude,
        &event("SessionEnd", "second"),
        None,
    )
    .unwrap();
    assert_eq!(read(&path).unwrap().unwrap().last_event, "SessionEnd");
    handle_event(
        &route,
        Harness::Claude,
        &event("UserPromptSubmit", "second"),
        None,
    )
    .unwrap();
    assert_eq!(
        read(&path).unwrap().unwrap().last_event,
        "SessionEnd",
        "a late event cannot revive an ended session"
    );
    assert!(status_file::lock_path_for(dir.path(), route.terminal.as_str()).exists());
}

#[test]
fn subagent_hooks_do_not_change_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let route = HookRoute {
        terminal: super::super::route::TerminalId::parse("terminal-a").unwrap(),
        status_dir: dir.path().to_path_buf(),
    };
    let mut event = event("UserPromptSubmit", "session");
    if let HookEvent::UserPromptSubmit { common, .. } = &mut event {
        common.agent_id = Some("child".into());
    }
    handle_event(&route, Harness::Claude, &event, None).unwrap();
    assert!(status_file::list_dir(dir.path()).unwrap().is_empty());
}
