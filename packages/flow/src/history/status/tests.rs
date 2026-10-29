use super::*;
use crate::run_log::{line::AttemptLine, machine::FlowRunEvent};

#[test]
fn projection_follows_durable_progress_and_unknown_nodes_stay_pending() {
    let root = tempfile::tempdir().unwrap();
    let node = NodeId::from("build");
    let store = crate::run_log::testing::attempt_in_flight(root.path(), &node);
    let running = RunHistory::read(root.path()).unwrap();
    assert_eq!(
        running.node_status(&node),
        NodeStatus::Running { attempt: 1 }
    );
    assert_eq!(running.node_status(&"unknown".into()), NodeStatus::Pending);
    let record = crate::record::AttemptRecord {
        attempt: 1,
        evidence_seq: 1,
        outcome: AttemptOutcome::Passed,
        invalidated: Default::default(),
        git_status: None,
        at: std::time::UNIX_EPOCH,
        took: Default::default(),
        waited: Default::default(),
        tools: Vec::new(),
        turns: 1,
        usage: None,
    };
    smol::block_on(store.commit(FlowRunEvent::AttemptPassed {
        node: node.clone(),
        record: Box::new(AttemptLine::new(&record, &reader::Spent::default())),
    }))
    .unwrap();
    let finished = RunHistory::read(root.path()).unwrap();
    assert_eq!(finished.node_status(&node), NodeStatus::Passed);
    assert!(finished.node_status(&node).is_complete());
    assert!(!running.node_status(&node).is_complete());
}

#[test]
fn invalidated_results_do_not_hide_the_replacement_attempt() {
    use crate::run_log::line::{FailureLine, SpentLine};
    use crate::run_log::machine::{NodeStart, Produces};
    use crate::run_log::store::{FlowRunStore, open_file_log};
    let root = tempfile::tempdir().unwrap();
    let node = NodeId::from("build");
    let gate = NodeId::from("check");
    let store =
        FlowRunStore::spawn(open_file_log(root.path()).unwrap(), root.path(), None).unwrap();
    let commit = |event| smol::block_on(store.commit(event)).unwrap();
    let line = |attempt, evidence| {
        Box::new(AttemptLine {
            attempt,
            evidence,
            at_ms: 0,
            took_ms: 0,
            waited_ms: 0,
            answers: Vec::new(),
            turns: 1,
            git_status: None,
            invalidated: Vec::new(),
            archived: Vec::new(),
            spent: SpentLine::default(),
        })
    };
    let status = || RunHistory::read(root.path()).unwrap().node_status(&node);
    commit(FlowRunEvent::RunStarted {
        started_by: None,
        nodes: vec![
            NodeStart {
                id: node.clone(),
                produces: Produces::File,
            },
            NodeStart {
                id: gate.clone(),
                produces: Produces::Nothing,
            },
        ],
        until: None,
        pinned: Vec::new(),
        profile: None,
    });
    commit(FlowRunEvent::attempt_started(node.clone(), 0));
    commit(FlowRunEvent::AttemptPassed {
        node: node.clone(),
        record: line(1, 1),
    });
    assert_eq!(status(), NodeStatus::Passed);
    commit(FlowRunEvent::attempt_started(gate.clone(), 0));
    commit(FlowRunEvent::Invalidated {
        gate: gate.clone(),
        members: vec![node.clone()],
    });
    assert_eq!(status(), NodeStatus::Pending);
    commit(FlowRunEvent::AttemptFailed {
        node: gate.clone(),
        failure: FailureLine::Exit { code: Some(1) },
        record: line(1, 2),
    });
    commit(FlowRunEvent::RerunScheduled {
        gate,
        members: vec![node.clone()],
    });
    commit(FlowRunEvent::attempt_started(node.clone(), 1));
    assert_eq!(status(), NodeStatus::Running { attempt: 1 });
    commit(FlowRunEvent::AttemptFailed {
        node: node.clone(),
        failure: FailureLine::Exit { code: Some(1) },
        record: line(1, 3),
    });
    assert_eq!(status(), NodeStatus::Failed);
    commit(FlowRunEvent::attempt_started(node.clone(), 1));
    assert_eq!(status(), NodeStatus::Running { attempt: 2 });
    commit(FlowRunEvent::AttemptCanceled {
        node: node.clone(),
        record: line(2, 4),
    });
    assert_eq!(status(), NodeStatus::Canceled);
}
