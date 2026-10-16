//! Fixtures for tests that drive a run below `execute`: a started log, and a
//! way to make a finished run look killed.

use std::path::Path;

use super::machine::{FlowRunEvent, NodeStart, Produces};
use super::store::{FlowRunStore, open_file_log};
use crate::NodeId;
use crate::load::LoadedFlow;

/// A fresh log in `run_dir` holding `RunStarted`, as `execute` leaves one.
pub(crate) fn started_store(
    run_dir: &Path,
    loaded: &LoadedFlow,
    until: Option<NodeId>,
    pinned: Vec<NodeId>,
) -> FlowRunStore {
    let store = spawn(run_dir);
    let started = FlowRunEvent::RunStarted {
        nodes: super::node_starts(loaded),
        until,
        pinned,
        profile: loaded.flow().profile.clone(),
    };
    smol::block_on(store.commit(started)).expect("RunStarted commits");
    store
}

/// A log with one agent node `node` whose first attempt is in flight, for a
/// runner test that asks questions on its behalf.
pub(crate) fn attempt_in_flight(run_dir: &Path, node: &NodeId) -> FlowRunStore {
    let store = spawn(run_dir);
    let started = FlowRunEvent::RunStarted {
        nodes: vec![NodeStart {
            id: node.clone(),
            produces: Produces::File,
        }],
        until: None,
        pinned: Vec::new(),
        profile: None,
    };
    smol::block_on(store.commit(started)).expect("RunStarted commits");
    smol::block_on(store.commit(FlowRunEvent::attempt_started(node.clone(), 0)))
        .expect("AttemptStarted commits");
    store
}

fn spawn(run_dir: &Path) -> FlowRunStore {
    let log = open_file_log(run_dir).unwrap_or_else(|e| panic!("the log opens: {e:?}"));
    FlowRunStore::spawn(log, run_dir, None).unwrap_or_else(|e| panic!("the log spawns: {e:?}"))
}

/// Make a finished run look killed: drop its `RunEnded` and every record
/// after it, the way a `kill -9` before that commit would have left the log.
pub(crate) fn forget_end(run_dir: &Path) {
    let path = super::log_dir(run_dir).join(super::FIRST_LOG_FILE);
    let text = std::fs::read_to_string(&path).expect("the run's log");
    let kept: Vec<&str> = text
        .lines()
        .take_while(|line| !line.contains("\"type\":\"run_ended\""))
        .collect();
    let mut body = kept.join("\n");
    body.push('\n');
    std::fs::write(&path, body).expect("rewrite the log");
}
