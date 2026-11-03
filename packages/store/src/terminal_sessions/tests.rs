use super::*;
use daruda_runlog::{
    Machine,
    testing::{assert_machine_laws, enumerate_crashes},
};

fn observed(session: &str, second: i64, start: bool, end: bool) -> SessionEvent {
    SessionEvent::Observed {
        observation: Observation {
            conversation: Conversation {
                provider: AccountRecipeId::Claude,
                config_home: Some(PathBuf::from("/accounts/a")),
                session_id: session.into(),
            },
            at: DateTime::from_timestamp(second, 0).unwrap(),
            cwd: PathBuf::from("/work"),
            transcript_path: Some(PathBuf::from("/transcript")),
            agent_pid: Some(42),
            kind: if end {
                ObservationKind::SessionEnd
            } else if start {
                ObservationKind::SessionStart
            } else {
                ObservationKind::Update
            },
        },
    }
}

fn closed(reason: TerminalEndReason, second: i64) -> SessionEvent {
    SessionEvent::TerminalClosed {
        reason,
        at: DateTime::from_timestamp(second, 0).unwrap(),
    }
}

fn apply(events: &[SessionEvent]) -> SessionHistory {
    let mut state = SessionHistory::default();
    for event in events {
        SessionMachine::apply(&mut state, event);
    }
    state
}

#[test]
fn successive_sessions_and_resumes_keep_separate_executions() {
    let state = apply(&[
        observed("a", 1, true, false),
        observed("a", 2, false, true),
        observed("b", 3, true, false),
        observed("a", 4, true, false),
    ]);
    assert_eq!(state.executions.len(), 3);
    assert_eq!(
        state
            .executions
            .iter()
            .map(|run| run.attempt)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(matches!(
        state.executions[1].state,
        ExecutionState::Ended {
            reason: EndReason::SessionReplaced,
            ..
        }
    ));
    assert_eq!(
        state.executions[0].conversation,
        state.executions[2].conversation
    );
    assert!(matches!(
        state.executions[2].state,
        ExecutionState::Observed
    ));
}

#[test]
fn same_session_id_in_another_account_or_provider_is_a_different_conversation() {
    let a = observed("same", 1, true, false);
    let mut b = observed("same", 2, true, false);
    let SessionEvent::Observed { observation } = &mut b else {
        panic!()
    };
    observation.conversation.config_home = Some(PathBuf::from("/accounts/b"));
    let mut c = observed("same", 3, true, false);
    let SessionEvent::Observed { observation } = &mut c else {
        panic!()
    };
    observation.conversation.provider = AccountRecipeId::Codex;
    assert_eq!(apply(&[a, b, c]).executions.len(), 3);
}

#[test]
fn a_delayed_goodbye_refines_process_exit_but_never_pane_closure() {
    let exit = SessionEvent::ProcessExited {
        session_id: "a".into(),
        pid: 42,
        at: DateTime::from_timestamp(3, 0).unwrap(),
    };
    let state = apply(&[
        observed("a", 1, true, false),
        exit.clone(),
        observed("a", 4, false, true),
    ]);
    assert!(matches!(
        state.executions[0].state,
        ExecutionState::Ended {
            reason: EndReason::SessionEnded,
            ..
        }
    ));
    let state = apply(&[
        observed("a", 1, true, false),
        exit,
        closed(TerminalEndReason::PaneClosed, 4),
        observed("a", 5, false, true),
    ]);
    assert!(matches!(
        state.executions[0].state,
        ExecutionState::Ended {
            reason: EndReason::PaneClosed,
            ..
        }
    ));
}

#[test]
fn late_hooks_are_retained_without_reopening_a_closed_terminal() {
    let state = apply(&[
        closed(TerminalEndReason::PaneClosed, 3),
        observed("a", 4, true, false),
        observed("a", 5, false, false),
    ]);
    assert_eq!(state.executions.len(), 1);
    assert!(state.terminal_is_closed());
    assert!(matches!(
        state.executions[0].state,
        ExecutionState::Ended {
            reason: EndReason::PaneClosed,
            ..
        }
    ));
}

#[test]
fn stale_events_and_the_wrong_pid_cannot_end_the_current_execution() {
    let state = apply(&[
        observed("a", 2, true, false),
        observed("b", 1, true, false),
        SessionEvent::ProcessExited {
            session_id: "a".into(),
            pid: 99,
            at: DateTime::from_timestamp(3, 0).unwrap(),
        },
    ]);
    assert_eq!(state.executions.len(), 1);
    assert!(matches!(
        state.executions[0].state,
        ExecutionState::Observed
    ));
}

#[test]
fn journal_machine_laws_and_crash_recovery() {
    let events = vec![
        observed("a", 1, true, false),
        observed("b", 2, true, false),
        observed("b", 3, false, true),
        observed("b", 4, true, false),
        closed(TerminalEndReason::ObservationLost, 5),
    ];
    let states = vec![
        SessionHistory::default(),
        apply(&events[..1]),
        apply(&events[..3]),
        apply(&events),
    ];
    assert_machine_laws::<SessionMachine>(&states, &events);
    assert!(enumerate_crashes::<SessionMachine>(events).cases > 0);
}
