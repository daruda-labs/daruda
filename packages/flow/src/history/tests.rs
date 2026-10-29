use super::*;
use crate::record::{AttemptRecord, Invalidation};
use crate::run_log::{
    line::{AttemptLine, EndLine},
    machine::FlowRunEvent as E,
};

fn record(attempt: u32, evidence_seq: u32) -> AttemptRecord {
    AttemptRecord {
        attempt,
        evidence_seq,
        outcome: AttemptOutcome::Passed,
        invalidated: Invalidation::default(),
        git_status: None,
        at: std::time::UNIX_EPOCH,
        took: Default::default(),
        waited: Default::default(),
        tools: Vec::new(),
        turns: 1,
        usage: None,
    }
}

#[test]
fn strict_inspection_preserves_corruption_and_a_torn_tail() {
    let dir = tempfile::tempdir().unwrap();
    let node = NodeId::from("hello");
    let store = crate::run_log::testing::attempt_in_flight(dir.path(), &node);
    let spent = reader::Spent {
        cost: Some(crate::request::CostLimit {
            amount: 0.125,
            currency: "USD".into(),
        }),
        ..Default::default()
    };
    smol::block_on(store.commit(E::AttemptPassed {
        node: node.clone(),
        record: Box::new(AttemptLine::new(&record(1, 1), &spent)),
    }))
    .unwrap();
    smol::block_on(store.commit(E::RunEnded { end: EndLine::Done })).unwrap();
    let completed = RunHistory::read(dir.path()).unwrap();
    assert!(completed.can_continue());
    assert!(completed.elapsed.is_some());
    let started = completed.started_at.expect("native run start");
    assert!(started <= std::time::SystemTime::now());
    assert!(started > std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    assert_eq!(
        RunHistory::read(dir.path()).unwrap().started_at,
        Some(started)
    );
    assert_eq!(completed.spent.cost, spent.cost);
    drop(store);
    let file = crate::run_log::log_file(dir.path());
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"{partial")
        .unwrap();
    let torn = RunHistory::read(dir.path()).unwrap();
    assert!(torn.torn);
    assert!(torn.elapsed.is_none());
    assert!(!torn.can_continue());
    std::fs::write(file, "{broken}\n{also broken}\n").unwrap();
    assert!(RunHistory::read(dir.path()).is_err());
}

#[test]
fn source_survives_resume_and_rejects_escaping_paths() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = crate::load(
        "version: 1\nnodes:\n  - id: gate\n    kind: command\n    run: echo ok\n",
        None,
    )
    .unwrap();
    let store = crate::run_log::testing::started_store(dir.path(), &loaded, None, Vec::new());
    assert!(
        smol::block_on(store.commit(E::SourceRecorded {
            source: "../outside.yaml".into()
        }))
        .is_err()
    );
    smol::block_on(store.commit(E::SourceRecorded {
        source: ".daruda/flows/check.yaml".into(),
    }))
    .unwrap();
    smol::block_on(store.commit(E::Resumed { carried: 0 })).unwrap();
    assert_eq!(
        RunHistory::read(dir.path()).unwrap().source,
        Some(PathBuf::from(".daruda/flows/check.yaml"))
    );
}

#[test]
fn repeated_attempt_numbers_resolve_by_evidence_and_archives() {
    let flow = crate::load("version: 1\ndefaults:\n  agent:\n    id: claude\nnodes:\n  - id: hello\n    kind: agent\n    output: hello.md\n    prompt: hi\n", None).unwrap().flow().clone();
    let first = record(1, 1);
    let mut repair = record(1, 2);
    repair.invalidated.nodes.push("hello".into());
    repair
        .invalidated
        .archived
        .push(PathBuf::from("/run/logs/hello.attempt-1.evidence-2.md"));
    let next = record(1, 3);
    let history = RunHistory {
        started_at: None,
        elapsed: None,
        spent: Default::default(),
        flow: Some(flow),
        records: vec![
            NodeRecord {
                id: "hello".into(),
                attempts: vec![first, next],
            },
            NodeRecord {
                id: "gate".into(),
                attempts: vec![repair],
            },
        ],
        sessions: Vec::new(),
        source: None,
        phase: Some(RunPhase::Ended(EndKind::Done)),
        torn: false,
        state: None,
        evidence_paths: Default::default(),
        attempt_order: vec![("hello".into(), 1), ("gate".into(), 2), ("hello".into(), 3)],
        in_flight: Vec::new(),
        current_outputs: [NodeId::from("hello")].into_iter().collect(),
    };
    assert_eq!(history.attempt(&"hello".into(), 1).unwrap().attempt, 1);
    assert_eq!(history.attempt(&"hello".into(), 3).unwrap().attempt, 1);
    assert_eq!(
        history.output(Path::new("/run"), &"hello".into(), 1),
        Some(PathBuf::from("/run/logs/hello.attempt-1.evidence-2.md"))
    );
    assert_eq!(
        history.output(Path::new("/run"), &"hello".into(), 3),
        Some(PathBuf::from("/run/hello.md"))
    );
}

#[test]
fn parallel_archive_resolution_uses_commit_order_not_start_order() {
    let flow = crate::load("version: 1\ndefaults:\n  agent:\n    id: claude\nnodes:\n  - id: hello\n    kind: agent\n    output: hello.md\n    prompt: hi\n", None).unwrap().flow().clone();
    let child = record(1, 3);
    let mut gate = record(1, 2);
    gate.invalidated.nodes.push("hello".into());
    gate.invalidated
        .archived
        .push(PathBuf::from("/run/logs/hello.attempt-1.evidence-2.md"));
    let history = RunHistory {
        started_at: None,
        elapsed: None,
        spent: Default::default(),
        flow: Some(flow),
        records: vec![
            NodeRecord {
                id: "hello".into(),
                attempts: vec![child],
            },
            NodeRecord {
                id: "gate".into(),
                attempts: vec![gate],
            },
        ],
        sessions: Vec::new(),
        source: None,
        phase: Some(RunPhase::Ended(EndKind::Done)),
        torn: false,
        state: None,
        evidence_paths: Default::default(),
        attempt_order: vec![("hello".into(), 3), ("gate".into(), 2)],
        in_flight: Vec::new(),
        current_outputs: Default::default(),
    };
    assert_eq!(
        history.output(Path::new("/run"), &"hello".into(), 3),
        Some(PathBuf::from("/run/logs/hello.attempt-1.evidence-2.md"))
    );
}

#[test]
fn existing_transcripts_are_read_without_renaming_them() {
    let dir = tempfile::tempdir().unwrap();
    let node = NodeId::from("hello");
    let store = crate::run_log::testing::attempt_in_flight(dir.path(), &node);
    smol::block_on(store.commit(E::AttemptPassed {
        node: node.clone(),
        record: Box::new(AttemptLine::new(&record(1, 1), &Default::default())),
    }))
    .unwrap();
    let [modern, legacy] =
        crate::runner::acp::transcript_paths(&dir.path().join("logs"), &node, 1, 1);
    std::fs::write(&legacy, "old conversation").unwrap();
    assert_eq!(
        RunHistory::read(dir.path())
            .unwrap()
            .transcript(dir.path(), &node, 1),
        Some(legacy.clone())
    );
    assert!(legacy.exists());
    assert!(!modern.exists());
}

#[test]
fn command_evidence_remains_available_when_the_snapshot_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = crate::load(
        "version: 1\nnodes:\n  - id: check\n    kind: command\n    run: echo ok\n",
        None,
    )
    .unwrap();
    let store = crate::run_log::testing::started_store(dir.path(), &loaded, None, Vec::new());
    let waiting = RunHistory::read(dir.path()).unwrap();
    assert!(waiting.flow().is_none());
    assert_eq!(waiting.nodes(), vec![NodeId::from("check")]);
    assert!(waiting.latest_selection().is_none());
    smol::block_on(store.commit(E::attempt_started("check".into(), 0))).unwrap();
    let running = RunHistory::read(dir.path()).unwrap();
    assert_eq!(running.nodes(), vec![NodeId::from("check")]);
    assert_eq!(running.latest_selection(), Some((NodeId::from("check"), 1)));
    assert_eq!(
        running.node_status(&"check".into()),
        super::NodeStatus::Running { attempt: 1 }
    );
    smol::block_on(store.commit(E::AttemptPassed {
        node: "check".into(),
        record: Box::new(AttemptLine::new(&record(1, 1), &Default::default())),
    }))
    .unwrap();
    let log = dir.path().join("logs/check.attempt-1.evidence-1.log");
    std::fs::write(&log, "ok").unwrap();
    let history = RunHistory::read(dir.path()).unwrap();
    assert!(history.flow.is_none());
    assert!(history.is_command(&"check".into()));
    assert_eq!(
        history.transcript(dir.path(), &"check".into(), 1),
        Some(log)
    );
}

#[test]
fn legacy_output_collisions_do_not_become_conversations() {
    for extension in [Some("md"), Some("txt"), None] {
        let dir = tempfile::tempdir().unwrap();
        let node = NodeId::from("hello");
        let store = crate::run_log::testing::attempt_in_flight(dir.path(), &node);
        if let Some(extension) = extension {
            std::fs::write(dir.path().join(crate::resume::RUN_SPEC_FILE), format!(
                "version: 1\ndefaults:\n  agent:\n    id: claude\nnodes:\n  - id: hello\n    kind: agent\n    output: hello.{extension}\n    prompt: hi\n"
            )).unwrap();
        }
        let [modern, legacy] =
            crate::runner::acp::transcript_paths(&dir.path().join("logs"), &node, 1, 1);
        std::fs::write(&legacy, "ambiguous legacy bytes").unwrap();
        let mut failed = record(1, 1);
        failed.invalidated.nodes.push(node.clone());
        failed.invalidated.archived.push(legacy.clone());
        smol::block_on(store.commit(E::AttemptFailed {
            node: node.clone(),
            failure: crate::run_log::line::FailureLine::Refused,
            record: Box::new(AttemptLine::new(&failed, &Default::default())),
        }))
        .unwrap();
        let history = RunHistory::read(dir.path()).unwrap();
        let expected = if extension == Some("txt") {
            &legacy
        } else {
            &modern
        };
        assert_eq!(
            history.transcript(dir.path(), &node, 1).as_ref(),
            Some(expected)
        );
        std::fs::write(&modern, "unambiguous conversation").unwrap();
        assert_eq!(
            RunHistory::read(dir.path())
                .unwrap()
                .transcript(dir.path(), &node, 1),
            Some(modern)
        );
    }
}
