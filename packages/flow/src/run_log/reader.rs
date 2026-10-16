//! Reading a run directory back without taking it over: the run log when the
//! run has one, the `progress.jsonl` journal an older build wrote when not.
//! Lenient by design — a history list must not fail over one damaged run.

use std::path::Path;
use std::time::Duration;

use daruda_runlog::{Body, Machine as _, Record};

use super::machine::{
    EndKind, FixOutcome, FlowRunEvent as E, FlowRunMachine, FlowRunState, ImportedOutcome, RunPhase,
};
use crate::NodeId;
use crate::event::{FlowEvent, Sequenced};
use crate::record::{AttemptOutcome, NodeRecord};
use crate::request::CostLimit;
use crate::schedule::FIX_SESSION_ID;

/// What the run had spent, in the scheduler's own terms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spent {
    /// Budget units consumed. Keeps the established name of the journal key.
    pub node_runs: u32,
    /// Time spent waiting for a person, across every process that ran it.
    pub parked: Duration,
    pub cost: Option<CostLimit>,
    pub cost_mixed: bool,
    pub warnings: Vec<String>,
}

/// Everything a run directory says about what was finished.
#[derive(Debug, Clone, Default)]
pub struct Replay {
    /// The nodes that passed (pins included), in the order they did.
    pub passed: Vec<NodeId>,
    /// The next evidence number; continued, never restarted.
    pub next_seq: u32,
    /// Every attempt so far, so a resumed `run.md` covers both halves.
    pub records: Vec<NodeRecord>,
    pub spent: Spent,
    pub profile: Option<String>,
    /// The selection the run was started under.
    pub until: Option<NodeId>,
    /// Nodes reused rather than run. Also in `passed`.
    pub pinned: Vec<NodeId>,
    /// The file ended in a line that could not be read.
    pub torn: bool,
}

/// Whether the run in `run_dir` keeps a run log.
pub fn has_log(run_dir: &Path) -> bool {
    super::log_dir(run_dir)
        .join(super::FIRST_LOG_FILE)
        .is_file()
}

/// Whether the run got as far as recording that it started, in either format.
pub fn started(run_dir: &Path) -> bool {
    has_log(run_dir) || crate::journal::exists(run_dir)
}

/// The node the run was started up to, read from its first event alone: a
/// fact fixed when the run started, so reading it before the lock cannot
/// disagree with what the run finds under it. An older build's journal, or
/// a log still waiting to import one, says it in the journal.
pub fn selection(run_dir: &Path) -> Option<NodeId> {
    if !has_log(run_dir) || awaits_import(run_dir) {
        return crate::journal::read(run_dir).until;
    }
    let path = super::log_dir(run_dir).join(super::FIRST_LOG_FILE);
    let file = std::fs::File::open(path).ok()?;
    std::io::BufRead::split(std::io::BufReader::new(file), b'\n')
        .map_while(Result::ok)
        .map_while(|line| serde_json::from_slice::<Record<E>>(&line).ok())
        .find_map(|record| match record.body {
            Body::Event {
                event: E::RunStarted { until, .. },
            } => Some(until),
            Body::Event {
                event: E::Imported(imported),
            } => Some(imported.until),
            _ => None,
        })
        .flatten()
}

/// What a run directory says it finished. A missing record is an empty
/// replay: a crash before the first line is a run with nothing done.
pub fn read(run_dir: &Path) -> Replay {
    if !has_log(run_dir) || awaits_import(run_dir) {
        return crate::journal::read(run_dir);
    }
    let lines = read_records(run_dir);
    let mut replay = replay_events(lines.records.iter().filter_map(event_of));
    replay.torn = lines.torn;
    replay
}

/// Whether an older build's journal still holds this run's progress: the
/// journal is there and the log, if any, has recorded nothing of the run. A
/// resume that crashed between opening the log and importing the journal
/// leaves exactly that, and the journal is still the record.
pub(crate) fn awaits_import(run_dir: &Path) -> bool {
    crate::journal::exists(run_dir)
        && state(run_dir).is_none_or(|state| state.phase() == RunPhase::Fresh)
}

/// The state the log replays to, or `None` when the run keeps no log.
pub fn state(run_dir: &Path) -> Option<FlowRunState> {
    if !has_log(run_dir) {
        return None;
    }
    let mut state = FlowRunState::default();
    for event in read_records(run_dir).records.iter().filter_map(event_of) {
        FlowRunMachine::apply(&mut state, event);
    }
    Some(state)
}

/// When the log was last written, for a run it says is still going: the
/// moment a dead holder stopped. `None` for a run that ended or keeps no log.
pub(crate) fn last_written_while_running(run_dir: &Path) -> Option<u64> {
    let records = read_records(run_dir).records;
    let mut state = FlowRunState::default();
    for event in records.iter().filter_map(event_of) {
        FlowRunMachine::apply(&mut state, event);
    }
    if state.phase() != RunPhase::Running {
        return None;
    }
    records.last().map(|record| record.at_ms)
}

/// How a run ended, as its log says. The log wins over a marker.
pub(crate) enum Ending {
    NoLog,
    Open,
    Ended(EndKind),
}

pub(crate) fn ending(run_dir: &Path) -> Ending {
    match state(run_dir).map(|state| state.phase()) {
        None => Ending::NoLog,
        Some(RunPhase::Ended(kind)) => Ending::Ended(kind),
        Some(RunPhase::Fresh | RunPhase::Running) => Ending::Open,
    }
}

/// The narration the run's records produce, numbered by their seq: what a
/// watcher of the live run was sent.
pub fn narration(run_dir: &Path) -> Vec<Sequenced<FlowEvent>> {
    let mut state = FlowRunState::default();
    let mut said = Vec::new();
    for record in read_records(run_dir).records {
        let Body::Event { event } = &record.body else {
            continue;
        };
        FlowRunMachine::apply(&mut state, event);
        if let Some(narrated) = super::narrate::narrate(run_dir, event, &state) {
            said.push(Sequenced {
                seq: record.seq,
                event: narrated,
            });
        }
    }
    said
}

/// Fold a run's events into what a resumed run starts from.
pub(crate) fn replay_events<'e>(events: impl IntoIterator<Item = &'e E>) -> Replay {
    let mut replay = Replay::default();
    let fix = NodeId::from(FIX_SESSION_ID);
    for event in events {
        match event {
            E::RunStarted {
                until,
                pinned,
                profile,
                ..
            } => {
                replay.profile = profile.clone();
                replay.until = until.clone();
                replay.passed.extend(pinned.iter().cloned());
                replay.pinned = pinned.clone();
            }
            E::Imported(imported) => {
                replay.profile = imported.profile.clone();
                replay.until = imported.until.clone();
                replay.pinned = imported.pinned.clone();
                replay.passed.extend(imported.pinned.iter().cloned());
                replay.passed.extend(imported.passed.iter().cloned());
                replay.next_seq = imported.next_evidence;
                replay.spent = (&imported.spent).into();
                for attempt in &imported.records {
                    let outcome = match &attempt.outcome {
                        ImportedOutcome::Passed => AttemptOutcome::Passed,
                        ImportedOutcome::Canceled => AttemptOutcome::Canceled,
                        ImportedOutcome::Reported { reason } => {
                            AttemptOutcome::Reported(reason.clone())
                        }
                    };
                    crate::record::push_attempt(
                        &mut replay.records,
                        &attempt.node,
                        attempt.record.to_record(outcome),
                    );
                }
            }
            E::AttemptStarted { evidence, .. } | E::FixStarted { evidence, .. } => {
                replay.next_seq = replay.next_seq.max(evidence + 1);
            }
            E::AttemptPassed { node, record } => {
                if !replay.passed.contains(node) {
                    replay.passed.push(node.clone());
                }
                settled(&mut replay, node, record, AttemptOutcome::Passed);
            }
            E::AttemptFailed {
                node,
                failure,
                record,
            } => {
                let reason = format!("failed: {}", crate::runner::NodeFailure::from(failure));
                settled(&mut replay, node, record, AttemptOutcome::Reported(reason));
            }
            E::AttemptCanceled { node, record } => {
                settled(&mut replay, node, record, AttemptOutcome::Canceled);
            }
            E::FixEnded {
                outcome, record, ..
            } => {
                let outcome = match outcome {
                    FixOutcome::Passed => AttemptOutcome::Passed,
                    FixOutcome::Canceled => AttemptOutcome::Canceled,
                    FixOutcome::Failed { failure } => AttemptOutcome::Reported(format!(
                        "failed: {}",
                        crate::runner::NodeFailure::from(failure)
                    )),
                };
                settled(&mut replay, &fix, record, outcome);
            }
            E::Invalidated { members, .. } => {
                replay.passed.retain(|passed| !members.contains(passed));
            }
            E::OutputLost { node } => replay.passed.retain(|passed| passed != node),
            E::Resumed { .. }
            | E::NodeRefused { .. }
            | E::ProcessSpawned { .. }
            | E::RerunScheduled { .. }
            | E::AskRaised { .. }
            | E::AskAnswered { .. }
            | E::AskAbandoned { .. }
            | E::RunEnded { .. } => {}
        }
    }
    replay
}

fn settled(
    replay: &mut Replay,
    node: &NodeId,
    record: &super::line::AttemptLine,
    outcome: AttemptOutcome,
) {
    for invalidated in &record.invalidated {
        replay.passed.retain(|passed| passed != invalidated);
    }
    replay.spent = (&record.spent).into();
    crate::record::push_attempt(&mut replay.records, node, record.to_record(outcome));
}

fn event_of(record: &Record<E>) -> Option<&E> {
    match &record.body {
        Body::Event { event } => Some(event),
        Body::Opened { .. } => None,
    }
}

struct Lines {
    records: Vec<Record<E>>,
    torn: bool,
}

/// Every whole record, stopping at the first line that does not read. Flow
/// run logs never compact, so generation 0 holds the whole run.
fn read_records(run_dir: &Path) -> Lines {
    let path = super::log_dir(run_dir).join(super::FIRST_LOG_FILE);
    let Ok(bytes) = std::fs::read(path) else {
        return Lines {
            records: Vec::new(),
            torn: false,
        };
    };
    let mut records = Vec::new();
    let mut torn = false;
    for line in bytes.split(|&byte| byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Record<E>>(line) {
            Ok(record) if record.v <= daruda_runlog::FORMAT_VERSION => records.push(record),
            _ => {
                torn = true;
                break;
            }
        }
    }
    Lines { records, torn }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::machine::{NodeStart, Produces};

    /// Pins count as passed; a lost output and an invalidation take a pass
    /// back, the way the journal's reader did.
    #[test]
    fn passes_are_folded_in_order_and_taken_back() {
        let events = [
            E::RunStarted {
                nodes: vec![NodeStart {
                    id: "design".into(),
                    produces: Produces::File,
                }],
                until: None,
                pinned: vec!["pinned".into()],
                profile: Some("cheap".to_string()),
            },
            E::OutputLost {
                node: "pinned".into(),
            },
            E::Invalidated {
                gate: "gate".into(),
                members: vec!["missing".into()],
            },
        ];
        let replay = replay_events(&events);
        assert!(replay.passed.is_empty(), "{:?}", replay.passed);
        assert_eq!(replay.pinned, vec![NodeId::from("pinned")]);
        assert_eq!(replay.profile.as_deref(), Some("cheap"));
    }

    /// The selection comes from the run's first event, so the progress
    /// after it — even a torn tail — is not read before the lock (F14).
    #[test]
    fn the_selection_is_read_from_the_first_event_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log =
            crate::run_log::store::open_file_log(dir.path()).unwrap_or_else(|_| panic!("open"));
        log.commit(E::RunStarted {
            nodes: vec![NodeStart {
                id: "design".into(),
                produces: Produces::File,
            }],
            until: Some("design".into()),
            pinned: Vec::new(),
            profile: None,
        })
        .expect("commit");
        drop(log);
        let path = crate::run_log::log_dir(dir.path()).join(crate::run_log::FIRST_LOG_FILE);
        let mut text = std::fs::read(&path).expect("log");
        text.extend_from_slice(b"{not a record");
        std::fs::write(&path, text).expect("tear");

        assert_eq!(selection(dir.path()), Some(NodeId::from("design")));
    }
}
