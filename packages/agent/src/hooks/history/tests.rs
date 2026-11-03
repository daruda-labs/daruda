use super::super::status_file::StatusFile;
use super::*;
use crate::SessionStatus;

fn file(terminal: &TerminalId, session: &str, event: &str) -> StatusFile {
    let mut file = StatusFile::new_hook(session, "/work", SessionStatus::Idle, event);
    file.terminal_id = Some(terminal.clone());
    file
}

#[test]
fn journals_survive_status_removal_and_keep_all_sessions_without_a_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = TerminalId::parse("terminal-a").unwrap();
    for session in ["first", "second"] {
        let _lock = SessionLock::acquire(dir.path(), terminal.as_str()).unwrap();
        observe_locked(dir.path(), &file(&terminal, session, "SessionStart")).unwrap();
        observe_locked(dir.path(), &file(&terminal, session, "SessionEnd")).unwrap();
    }
    assert_eq!(
        terminals(dir.path()).unwrap().as_slice(),
        std::slice::from_ref(&terminal)
    );
    assert_eq!(read(dir.path(), &terminal).unwrap().executions.len(), 2);
    reconcile_terminals(dir.path(), &terminals(dir.path()).unwrap()).unwrap();
    let history = read(dir.path(), &terminal).unwrap();
    assert!(history.terminal_is_closed());
    assert_eq!(history.executions.len(), 2);
}

#[test]
fn restart_records_observation_loss_without_claiming_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = TerminalId::parse("terminal-a").unwrap();
    observe_locked(dir.path(), &file(&terminal, "session", "SessionStart")).unwrap();
    reconcile_terminals(dir.path(), &terminals(dir.path()).unwrap()).unwrap();
    let history = read(dir.path(), &terminal).unwrap();
    assert!(matches!(
        history.executions[0].state,
        daruda_store::terminal_sessions::ExecutionState::Ended {
            reason: daruda_store::terminal_sessions::EndReason::ObservationLost,
            ..
        }
    ));
}

#[test]
fn routes_keep_history_isolated() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let terminal = TerminalId::parse("terminal-a").unwrap();
    observe_locked(a.path(), &file(&terminal, "session", "SessionStart")).unwrap();
    assert!(terminals(b.path()).unwrap().is_empty());
}

#[test]
fn startup_snapshot_reconciliation_keeps_new_terminal_observations_open() {
    let dir = tempfile::tempdir().unwrap();
    let old = TerminalId::parse("old-terminal").unwrap();
    observe_locked(dir.path(), &file(&old, "old-session", "SessionStart")).unwrap();
    let snapshot = terminals(dir.path()).unwrap();
    let new = TerminalId::parse("new-terminal").unwrap();
    observe_locked(dir.path(), &file(&new, "new-session", "SessionStart")).unwrap();
    reconcile_terminals(dir.path(), &snapshot).unwrap();
    assert!(read(dir.path(), &old).unwrap().terminal_is_closed());
    assert!(!read(dir.path(), &new).unwrap().terminal_is_closed());
}
