//! Reading a run directory back without taking it over: the run log when the
//! run has one, the `progress.jsonl` journal an older build wrote when not.
//! The log is read by `daruda_runlog`'s replay rules; a log those rules
//! refuse reads as nothing here, so a history list does not fail over one
//! damaged run, and a resume opening it under the lock says why.

use std::path::Path;
use std::time::Duration;

use daruda_runlog::{Body, FileBackend, History, Machine as _, Record, RunLog};

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
    pub started_by: Option<crate::request::StartedBy>,
    /// The selection the run was started under.
    pub until: Option<NodeId>,
    /// Nodes reused rather than run. Also in `passed`.
    pub pinned: Vec<NodeId>,
    /// The file ended in a line that could not be read.
    pub torn: bool,
}

/// Whether the run in `run_dir` keeps a run log.
pub fn has_log(run_dir: &Path) -> bool {
    // A directory that cannot be listed may hold one: claiming it does keeps
    // a new run out of it, and reading it reports the failure.
    backend(run_dir).holds_log().unwrap_or(true)
}

/// Whether the run got as far as recording that it started, in either format.
pub fn started(run_dir: &Path) -> bool {
    has_log(run_dir) || crate::journal::exists(run_dir)
}

/// The node the run was started up to, from its first event: a fact fixed
/// when the run started, so reading it before the lock cannot disagree with
/// what the run finds under it. An older build's journal, or a log still
/// waiting to import one, says it in the journal.
pub fn selection(run_dir: &Path) -> Option<NodeId> {
    match source(run_dir) {
        Source::Journal => crate::journal::read(run_dir).until,
        Source::Unreadable => None,
        Source::Log(history) => history
            .records
            .iter()
            .filter_map(event_of)
            .find_map(|event| match event {
                E::RunStarted { until, .. } => Some(until.clone()),
                E::Imported(imported) => Some(imported.until.clone()),
                _ => None,
            })
            .flatten(),
    }
}

/// What a run directory says it finished. A missing record is an empty
/// replay: a crash before the first line is a run with nothing done. So is
/// a log replay refuses.
pub fn read(run_dir: &Path) -> Replay {
    match source(run_dir) {
        Source::Journal => crate::journal::read(run_dir),
        Source::Unreadable => Replay::default(),
        Source::Log(history) => {
            let mut replay = replay_events(history.records.iter().filter_map(event_of));
            replay.torn = history.torn;
            replay
        }
    }
}

/// Where a run's progress is read from.
enum Source {
    /// An older build's journal: there is no log, or it holds nothing of the
    /// run yet.
    Journal,
    Log(Box<History<FlowRunMachine>>),
    /// A log replay refuses: corrupt, or written by a newer build.
    Unreadable,
}

fn source(run_dir: &Path) -> Source {
    match history(run_dir) {
        None => Source::Journal,
        Some(Ok(history)) if awaits(&history.state, run_dir) => Source::Journal,
        Some(Ok(history)) => Source::Log(Box::new(history)),
        Some(Err(_)) => Source::Unreadable,
    }
}

/// Whether an older build's journal still holds this run's progress: the
/// journal is there and the log has recorded nothing of the run. A resume
/// that crashed between opening the log and importing the journal leaves
/// exactly that, and the journal is still the record. A log replay refuses
/// may hold the run, so it never hands over to the journal.
fn awaits(state: &FlowRunState, run_dir: &Path) -> bool {
    state.phase() == RunPhase::Fresh && crate::journal::exists(run_dir)
}

/// The state the log replays to, or `None` when the run keeps no log or
/// keeps one replay refuses.
pub fn state(run_dir: &Path) -> Option<FlowRunState> {
    history(run_dir)?.ok().map(|history| history.state)
}

/// A strict read-only status inspection. Unlike history-list helpers, replay
/// errors remain visible to callers; this never opens a writable log.
#[derive(Debug)]
pub struct Inspection {
    /// Sequence of the latest durable resume, or zero for the initial episode.
    pub resumed_at: u64,
    pub state: FlowRunState,
    pub replay: Replay,
    pub end: Option<crate::event::RunEnd>,
}

/// `None` leaves inspection to the legacy journal, including an interrupted
/// resume that opened its modern log before importing the journal.
pub fn inspect(run_dir: &Path) -> Result<Option<Inspection>, daruda_runlog::OpenError> {
    let Some(history) = history(run_dir) else {
        return Ok(None);
    };
    let history = history?;
    if awaits(&history.state, run_dir) {
        return Ok(None);
    }
    let mut after = FlowRunState::default();
    let mut end = None;
    for event in history.records.iter().filter_map(event_of) {
        FlowRunMachine::apply(&mut after, event);
        if let Some(FlowEvent::RunEnded { end: ended }) =
            super::narrate::narrate(run_dir, event, &after)
        {
            end = Some(ended);
        }
        if matches!(event, E::Resumed { .. }) {
            end = None;
        }
    }
    Ok(Some(Inspection {
        resumed_at: history
            .records
            .iter()
            .rev()
            .find(|record| matches!(event_of(record), Some(E::Resumed { .. })))
            .map(|record| record.seq)
            .unwrap_or_default(),
        state: history.state,
        replay: replay_events(history.records.iter().filter_map(event_of)),
        end,
    }))
}

/// When the log was last written, for a run it says is still going: the
/// moment a dead holder stopped. `None` for a run that ended, keeps no log,
/// or keeps one replay refuses.
pub(crate) fn last_written_while_running(run_dir: &Path) -> Option<u64> {
    let history = history(run_dir)?.ok()?;
    if history.state.phase() != RunPhase::Running {
        return None;
    }
    history.records.last().map(|record| record.at_ms)
}

/// How a run ended, as its log says. The log wins over a marker.
pub(crate) enum Ending {
    NoLog,
    /// The log is there but replay refuses it, so it says nothing either way.
    Unreadable,
    Open,
    Ended(EndKind),
}

pub(crate) fn ending(run_dir: &Path) -> Ending {
    match history(run_dir) {
        None => Ending::NoLog,
        Some(Err(_)) => Ending::Unreadable,
        Some(Ok(history)) => match history.state.phase() {
            RunPhase::Ended(kind) => Ending::Ended(kind),
            RunPhase::Fresh | RunPhase::Running => Ending::Open,
        },
    }
}

/// The narration the run's records produce, numbered by their seq: what a
/// watcher of the live run was sent.
pub fn narration(run_dir: &Path) -> Vec<Sequenced<FlowEvent>> {
    let Some(Ok(history)) = history(run_dir) else {
        return Vec::new();
    };
    let mut state = FlowRunState::default();
    let mut said = Vec::new();
    for record in history.records {
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

/// Fold a run's events into what a resumed run starts from. Which nodes
/// passed is the machine's answer, the same one resume's own steps read.
pub(crate) fn replay_events<'e>(events: impl IntoIterator<Item = &'e E>) -> Replay {
    let mut replay = Replay::default();
    let mut state = FlowRunState::default();
    let fix = NodeId::from(FIX_SESSION_ID);
    for event in events {
        FlowRunMachine::apply(&mut state, event);
        match event {
            E::RunStarted {
                started_by,
                until,
                pinned,
                profile,
                ..
            } => {
                replay.started_by = *started_by;
                replay.profile = profile.clone();
                replay.until = until.clone();
                replay.pinned = pinned.clone();
            }
            E::Imported(imported) => {
                replay.profile = imported.profile.clone();
                replay.until = imported.until.clone();
                replay.pinned = imported.pinned.clone();
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
            E::Invalidated { .. }
            | E::OutputLost { .. }
            | E::Resumed { .. }
            | E::NodeRefused { .. }
            | E::ProcessSpawned { .. }
            | E::RerunScheduled { .. }
            | E::AskRaised { .. }
            | E::AskAnswered { .. }
            | E::AskAbandoned { .. }
            | E::RunEnded { .. } => {}
        }
    }
    replay.passed = state.passed_in_order();
    replay
}

fn settled(
    replay: &mut Replay,
    node: &NodeId,
    record: &super::line::AttemptLine,
    outcome: AttemptOutcome,
) {
    replay.spent = (&record.spent).into();
    crate::record::push_attempt(&mut replay.records, node, record.to_record(outcome));
}

fn event_of(record: &Record<E>) -> Option<&E> {
    match &record.body {
        Body::Event { event } => Some(event),
        Body::Opened { .. } => None,
    }
}

fn backend(run_dir: &Path) -> FileBackend {
    FileBackend::new(super::log_dir(run_dir))
}

/// The run's log as replay reads it, or `None` when the run keeps none.
/// Flow run logs never compact, so its records are the whole run.
fn history(run_dir: &Path) -> Option<Result<History<FlowRunMachine>, daruda_runlog::OpenError>> {
    has_log(run_dir).then(|| RunLog::read_history(&backend(run_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::machine::{NodeStart, Produces};

    #[test]
    fn strict_inspection_preserves_fine_ending_and_leaves_bytes_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = crate::load::load(
            "version: 1\nnodes:\n  - id: a\n    kind: command\n    run: echo a\n",
            None,
        )
        .unwrap();
        let store = super::super::testing::started_store(dir.path(), &loaded, None, vec![]);
        let event = serde_json::from_value(serde_json::json!({"type":"run_ended","end":{"kind":"budget_exhausted","limit":"node_runs"}})).unwrap();
        smol::block_on(store.commit(event)).unwrap();
        drop(store);
        let path = super::super::log_file(dir.path());
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            inspect(dir.path()).unwrap().unwrap().end,
            Some(crate::event::RunEnd::BudgetExhausted { .. })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"broken\nbroken\n").unwrap();
        assert!(inspect(dir.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken\nbroken\n");
    }

    /// Pins count as passed; a lost output and an invalidation take a pass
    /// back, the way the journal's reader did.
    #[test]
    fn passes_are_folded_in_order_and_taken_back() {
        let events = [
            E::RunStarted {
                started_by: None,
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

    /// The passes a resume schedules from are the state machine's, in the
    /// order they happened: a node started again after its pass is not
    /// passed, whatever the event that took the pass away.
    #[test]
    fn the_passes_are_the_machines_in_pass_order() {
        let line = |attempt, evidence| {
            Box::new(crate::run_log::line::AttemptLine::new(
                &crate::record::AttemptRecord {
                    attempt,
                    evidence_seq: evidence,
                    outcome: AttemptOutcome::Passed,
                    invalidated: crate::record::Invalidation::default(),
                    git_status: None,
                    at: std::time::UNIX_EPOCH,
                    took: Duration::ZERO,
                    waited: crate::runner::Waiting::default(),
                    tools: Vec::new(),
                    turns: 1,
                    usage: None,
                },
                &Default::default(),
            ))
        };
        let node = |id: &str| NodeStart {
            id: id.into(),
            produces: Produces::Nothing,
        };
        let events = [
            E::RunStarted {
                started_by: None,
                nodes: vec![node("a"), node("b"), node("c")],
                until: None,
                pinned: Vec::new(),
                profile: None,
            },
            E::attempt_started("b".into(), 0),
            E::AttemptPassed {
                node: "b".into(),
                record: line(1, 1),
            },
            E::attempt_started("a".into(), 0),
            E::AttemptPassed {
                node: "a".into(),
                record: line(1, 2),
            },
            E::attempt_started("c".into(), 0),
            E::AttemptPassed {
                node: "c".into(),
                record: line(1, 3),
            },
            E::attempt_started("c".into(), 0),
        ];
        let numbered = numbered(&events);
        let replay = replay_events(&numbered);
        let state = state_of(&numbered);
        assert_eq!(replay.passed, vec![NodeId::from("b"), NodeId::from("a")]);
        let passed: std::collections::BTreeSet<NodeId> = replay.passed.iter().cloned().collect();
        assert_eq!(passed, state.finished());
    }

    /// Each event as `assign` would have numbered it, applied in turn.
    fn numbered(events: &[E]) -> Vec<E> {
        let mut state = FlowRunState::default();
        events
            .iter()
            .map(|event| {
                let mut event = event.clone();
                FlowRunMachine::assign(&state, &mut event);
                FlowRunMachine::check(&state, &event).expect("a valid event");
                FlowRunMachine::apply(&mut state, &event);
                event
            })
            .collect()
    }

    fn state_of(events: &[E]) -> FlowRunState {
        let mut state = FlowRunState::default();
        for event in events {
            FlowRunMachine::apply(&mut state, event);
        }
        state
    }

    /// The selection comes from the run's first event, so the progress
    /// after it — even a torn tail — is not read before the lock.
    #[test]
    fn the_selection_is_read_from_the_first_event_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log =
            crate::run_log::store::open_file_log(dir.path()).unwrap_or_else(|_| panic!("open"));
        log.commit(E::RunStarted {
            started_by: None,
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
        let path = crate::run_log::log_file(dir.path());
        let mut text = std::fs::read(&path).expect("log");
        text.extend_from_slice(b"{not a record");
        std::fs::write(&path, text).expect("tear");

        assert_eq!(selection(dir.path()), Some(NodeId::from("design")));
    }
}
