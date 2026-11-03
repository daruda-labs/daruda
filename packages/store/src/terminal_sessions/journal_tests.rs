use super::*;

#[test]
fn an_empty_journal_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let journal = SessionJournal::open(dir.path()).unwrap();
    assert!(journal.state().executions.is_empty());
    drop(journal);
    assert!(
        SessionJournal::read(dir.path())
            .unwrap()
            .executions
            .is_empty()
    );
}

#[test]
fn a_corrupt_journal_is_refused_without_overwriting_it() {
    let dir = tempfile::tempdir().unwrap();
    drop(SessionJournal::open(dir.path()).unwrap());
    let path = FileBackend::new(dir.path()).log_path(0);
    std::fs::write(&path, b"corrupt\ncorrupt\n").unwrap();
    assert!(SessionJournal::read(dir.path()).is_err());
    assert!(SessionJournal::open(dir.path()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"corrupt\ncorrupt\n");
}

#[test]
fn newer_schemas_are_refused_without_overwriting_them() {
    struct FutureMachine;
    impl daruda_runlog::Machine for FutureMachine {
        type State = super::super::SessionHistory;
        type Event = super::super::SessionEvent;
        type Rejected = std::convert::Infallible;
        const SCHEMA: u32 = 2;
        fn check(_: &Self::State, _: &Self::Event) -> Result<(), Self::Rejected> {
            Ok(())
        }
        fn apply(_: &mut Self::State, _: &Self::Event) {}
    }
    let dir = tempfile::tempdir().unwrap();
    drop(RunLog::<FutureMachine, _>::open(FileBackend::new(dir.path()), "future").unwrap());
    let path = FileBackend::new(dir.path()).log_path(0);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        SessionJournal::open(dir.path()),
        Err(HistoryError::Open(daruda_runlog::OpenError::TooNew { .. }))
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn frequent_compaction_keeps_executions_and_the_latest_observation_after_reopen() {
    use crate::accounts::AccountRecipeId;
    use crate::terminal_sessions::{
        Conversation, EndReason, ExecutionState, Observation, ObservationKind,
    };
    use std::path::PathBuf;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let first_at: chrono::DateTime<chrono::Utc> = "2026-11-03T00:00:00Z".parse().unwrap();
    for index in 0..100 {
        let session = if index < 50 { "first" } else { "second" };
        let observation = Observation {
            conversation: Conversation {
                provider: AccountRecipeId::Claude,
                config_home: Some(PathBuf::from("/account")),
                session_id: session.to_owned(),
            },
            at: first_at + Duration::from_secs(index),
            cwd: PathBuf::from(format!("/work/{index}")),
            transcript_path: Some(PathBuf::from(format!("/transcript/{session}"))),
            agent_pid: Some(42),
            kind: match index % 50 {
                0 => ObservationKind::SessionStart,
                49 => ObservationKind::SessionEnd,
                _ => ObservationKind::Update,
            },
        };
        let mut journal = SessionJournal::open(dir.path()).unwrap();
        journal
            .commit(SessionEvent::Observed { observation })
            .unwrap();
    }
    let durable = SessionJournal::read(dir.path()).unwrap();
    assert_eq!(durable.executions.len(), 2);
    assert_eq!(
        durable.executions[0].last_observed_at,
        first_at + Duration::from_secs(49)
    );
    let last = &durable.executions[1];
    assert_eq!(last.last_observed_at, first_at + Duration::from_secs(99));
    assert_eq!(last.cwd, PathBuf::from("/work/99"));
    assert_eq!(
        last.transcript_path,
        Some(PathBuf::from("/transcript/second"))
    );
    assert_eq!(last.agent_pid, Some(42));
    assert!(matches!(
        last.state,
        ExecutionState::Ended {
            reason: EndReason::SessionEnded,
            ..
        }
    ));
    let history = RunLog::<SessionMachine, _>::read_history(&FileBackend::new(dir.path())).unwrap();
    assert!(
        history.base_seq > 0,
        "the fixture must cross the snapshot budget"
    );
    assert!(history.records.len() <= COMPACT_AFTER_RECORDS as usize);
    assert_eq!(history.state, durable);
    let reopened = SessionJournal::open(dir.path()).unwrap();
    assert_eq!(reopened.state(), &durable);
}
