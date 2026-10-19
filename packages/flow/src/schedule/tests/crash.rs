//! A run crashed at a write of its log, then picked up by a new process
//! (§6.5). The log lives in a `FaultBackend`, so "what survived" is exactly
//! what the backend kept; outputs live in a real run directory.

use super::*;
use crate::load::LoadedFlow;
use crate::run_log::machine::{FlowRunEvent, FlowRunMachine, FlowRunState, Produces, RunPhase};
use crate::run_log::store::{FlowRunStore, open_log};
use daruda_runlog::testing::{Fault, FaultBackend};
use daruda_runlog::{Body, LogBackend, Machine as _, Record};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Two agent nodes in a chain, both allowed enough attempts that a crash in
/// any one of them still leaves a retry.
pub(super) const TWO_RETRYING: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry: { max_attempts: 5, wait: 0s, hint: again }
  - id: review
    kind: agent
    deps: [design]
    output: review.md
    prompt: write
    on_fail:
      retry: { max_attempts: 5, wait: 0s, hint: again }
";

/// How one process of a run ended.
pub(super) enum Process {
    /// It drove the run to this outcome (the log may not have it all).
    Drove(RunReport),
    /// The log it found already said the run had ended.
    AlreadyEnded,
    /// It could not open the log, or the run could not be picked up.
    Refused(RunOutcome),
}

/// One process of the run in `run_dir`, with its log on `backend`: start it
/// if the log holds nothing, pick it up otherwise, drive it, record its end.
pub(super) fn process<B: LogBackend + Send + 'static>(
    backend: B,
    loaded: &LoadedFlow,
    dir: &Path,
    runner: &dyn NodeRunner,
) -> Process {
    process_asking(backend, loaded, dir, runner, None)
}

/// [`process`], with somewhere to put questions.
pub(super) fn process_asking<B: LogBackend + Send + 'static>(
    backend: B,
    loaded: &LoadedFlow,
    dir: &Path,
    runner: &dyn NodeRunner,
    ask: Option<&smol::channel::Sender<crate::runner::PendingAsk>>,
) -> Process {
    let run_dir = dir.join("run");
    let log = match open_log(backend, &crate::run_log::log_dir(&run_dir)) {
        Ok(log) => log,
        Err(outcome) => return Process::Refused(outcome),
    };
    let begun = match log.state().phase() {
        RunPhase::Ended(_) => return Process::AlreadyEnded,
        RunPhase::Fresh => FlowRunStore::spawn(log, &run_dir, None).and_then(|store| {
            let started = FlowRunEvent::RunStarted {
                nodes: crate::run_log::node_starts(loaded),
                until: None,
                pinned: Vec::new(),
                profile: None,
            };
            smol::block_on(store.commit(started))
                .map(|_| (store.clone(), None))
                .map_err(|e| store.failure(IoSite::Run, e))
        }),
        RunPhase::Running => {
            let mut warnings = Vec::new();
            crate::resume::pick_up(log, loaded, &run_dir, None, &mut warnings)
                .map(|(store, replay)| (store, Some(replay)))
        }
    };
    let (store, resume) = match begun {
        Ok(begun) => begun,
        Err(outcome) => return Process::Refused(outcome),
    };
    let report = smol::block_on(run_flow(
        RunInputs {
            loaded,
            flow_dir: dir,
            cwd: dir,
            run_dir: &run_dir,
            cancel: &CancelToken::default(),
            budget: &Budget::unlimited(),
            git_status: None,
            log: &store,
            ask,
            until: None,
            pinned: resume
                .as_ref()
                .map(|replay| replay.pinned.clone())
                .unwrap_or_default(),
            resume,
        },
        runner,
    ));
    // SILENT-OK: a crashed log refuses its end, which is the crash.
    let _ = smol::block_on(crate::schedule::run::record_end(&store, &report.outcome));
    Process::Drove(report)
}

/// Every whole record the backend holds, oldest first.
pub(super) fn records(backend: &FaultBackend) -> Vec<FlowRunEvent> {
    let bytes = backend.file("log.0.jsonl").unwrap_or_default();
    bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice::<Record<FlowRunEvent>>(line).ok())
        .filter_map(|record| match record.body {
            Body::Event { event } => Some(event),
            Body::Opened { .. } => None,
        })
        .collect()
}

pub(super) fn state_of(events: &[FlowRunEvent]) -> FlowRunState {
    let mut state = FlowRunState::default();
    for event in events {
        FlowRunMachine::apply(&mut state, event);
    }
    state
}

/// The log index of the first record matching `wanted`, counting the
/// `opened` record as 0 — the numbering `Fault` uses for appends.
pub(super) fn append_index(
    backend: &FaultBackend,
    wanted: impl Fn(&FlowRunEvent) -> bool,
) -> usize {
    let bytes = backend.file("log.0.jsonl").expect("a log");
    bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .position(|line| {
            matches!(
                serde_json::from_slice::<Record<FlowRunEvent>>(line).map(|r| r.body),
                Ok(Body::Event { event }) if wanted(&event)
            )
        })
        .expect("the record is in the log")
}

/// The faults worth trying at every write of a clean run.
fn every_fault(clean: &FaultBackend) -> Vec<Fault> {
    let lens = clean.append_lens();
    let mut faults = Vec::new();
    // From 1: a fault on the `opened` record refuses the open, which is the
    // runlog crate's own case.
    for (k, &len) in lens.iter().enumerate().skip(1) {
        faults.push(Fault::TornAppend { k, keep: len / 2 });
        faults.push(Fault::FailSyncAfter { k });
    }
    faults.extend((1..clean.syncs()).map(|k| Fault::LoseUnsyncedAfter { k }));
    faults
}

/// **S1, S8.** Crash a run at every write, pick it up, and it finishes with
/// no evidence number reused, attempt numbers continued, and every passed
/// output on disk.
#[test]
fn a_crash_at_any_write_resumes_with_numbers_and_outputs_intact() {
    let loaded = load(TWO_RETRYING, None).expect("valid flow");
    let clean = FaultBackend::new();
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(matches!(
        process(clean.clone(), &loaded, dir.path(), &FakeRunner::new()),
        Process::Drove(_)
    ));

    let mut continued = 0;
    for fault in every_fault(&clean) {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = FaultBackend::new().with_fault(fault);
        let _ = process(first.clone(), &loaded, dir.path(), &FakeRunner::new());
        let second = first.crash();
        assert_one_fold_of_passes(&records(&second), &fault);
        match process(second.clone(), &loaded, dir.path(), &FakeRunner::new()) {
            Process::Drove(report) => assert!(
                matches!(report.outcome, RunOutcome::Done),
                "{fault:?}: {:?}",
                report.outcome
            ),
            Process::AlreadyEnded => {}
            Process::Refused(outcome) => panic!("{fault:?}: refused {outcome:?}"),
        }

        let events = records(&second);
        let mut evidence = Vec::new();
        let mut attempts: BTreeMap<(NodeId, u32), Vec<u32>> = BTreeMap::new();
        for event in &events {
            match event {
                FlowRunEvent::AttemptStarted {
                    node,
                    generation,
                    attempt,
                    evidence: seq,
                } => {
                    evidence.push(*seq);
                    attempts
                        .entry((node.clone(), *generation))
                        .or_default()
                        .push(*attempt);
                }
                FlowRunEvent::FixStarted { evidence: seq, .. } => evidence.push(*seq),
                _ => {}
            }
        }
        let distinct: BTreeSet<u32> = evidence.iter().copied().collect();
        assert_eq!(distinct.len(), evidence.len(), "{fault:?}: {evidence:?}");
        for (key, numbers) in &attempts {
            let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
            assert_eq!(numbers, &expected, "{fault:?}: {key:?}");
            continued += usize::from(numbers.len() > 1);
        }
        assert_one_fold_of_passes(&events, &fault);
        let state = state_of(&events);
        assert!(
            matches!(state.phase(), RunPhase::Ended(_)),
            "{fault:?}: {:?}",
            state.phase()
        );
        for id in state.finished() {
            if state.node(&id).map(|n| n.produces) == Some(Produces::File) {
                let output = dir.path().join("run").join(format!("{id}.md"));
                assert!(
                    std::fs::metadata(&output).is_ok_and(|m| m.len() > 0),
                    "{fault:?}: `{id}` passed with no output"
                );
            }
        }
    }
    assert!(continued > 0, "no crash interrupted an attempt");
}

/// The passes the scheduler resumes from and the ones resume's own steps
/// keep are the same set.
fn assert_one_fold_of_passes(events: &[FlowRunEvent], fault: &Fault) {
    let scheduled: BTreeSet<NodeId> = crate::run_log::reader::replay_events(events)
        .passed
        .into_iter()
        .collect();
    assert_eq!(scheduled, state_of(events).finished(), "{fault:?}");
}

/// One agent node allowed `max_attempts` tries.
fn retrying_design(max_attempts: u32) -> LoadedFlow {
    let flow = format!(
        "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry: {{ max_attempts: {max_attempts}, wait: 0s, hint: again }}
"
    );
    load(&flow, None).expect("valid flow")
}

/// The attempt numbers and evidence each `design` start was given.
fn design_starts(events: &[FlowRunEvent]) -> Vec<(u32, u32)> {
    events
        .iter()
        .filter_map(|event| match event {
            FlowRunEvent::AttemptStarted {
                node,
                attempt,
                evidence,
                ..
            } if node == "design" => Some((*attempt, *evidence)),
            _ => None,
        })
        .collect()
}

/// **D4-2, S8.** An attempt a crash interrupted does not count: with two
/// allowed, two crashes mid-attempt still leave the next pick-up a try,
/// numbered past both.
#[test]
fn crash_interrupted_attempts_do_not_count_toward_the_cap() {
    let loaded = retrying_design(2);
    let dir = tempfile::tempdir().expect("tempdir");
    // opened 0, run_started 1, attempt_started 2, attempt_passed 3.
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: 3, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &FakeRunner::new());
    // opened 0, resumed 1, attempt_started 2, attempt_passed 3.
    let second = first
        .crash()
        .with_fault(Fault::TornAppend { k: 3, keep: 0 });
    let runner = FakeRunner::new();
    let _ = process(second.clone(), &loaded, dir.path(), &runner);
    assert_eq!(runner.calls().len(), 1, "the second process tried once");
    assert_eq!(runner.calls()[0].attempt, 2, "and as the second attempt");

    let third = FakeRunner::new();
    let last = second.crash();
    let Process::Drove(report) = process(last.clone(), &loaded, dir.path(), &third) else {
        panic!("the run could not be picked up");
    };
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let attempts: Vec<u32> = third.calls().iter().map(|call| call.attempt).collect();
    assert_eq!(attempts, [3], "the crashed attempts used up the cap");
    assert_eq!(design_starts(&records(&last)), [(1, 1), (2, 2), (3, 3)]);
}

/// **D4-2.** A node with no retry policy that a crash caught mid-attempt
/// runs again on resume, under fresh attempt and evidence numbers, rather
/// than failing as interrupted.
#[test]
fn a_node_without_retries_runs_again_after_a_crash() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
";
    let loaded = load(flow, None).expect("valid flow");
    let dir = tempfile::tempdir().expect("tempdir");
    // opened 0, run_started 1, attempt_started 2, attempt_passed 3.
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: 3, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &FakeRunner::new());

    let runner = FakeRunner::new();
    let second = first.crash();
    let Process::Drove(report) = process(second.clone(), &loaded, dir.path(), &runner) else {
        panic!("the run could not be picked up");
    };

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.calls().len(), 1);
    assert_eq!(design_starts(&records(&second)), [(1, 1), (2, 2)]);
}

/// **D4-2, F10.** A failure that reached a verdict still counts across a
/// resume; only the crashed attempt after it does not. With two allowed,
/// one failure and one crash leave exactly one more try.
#[test]
fn a_verdict_failure_still_counts_after_a_crash() {
    let loaded = retrying_design(2);
    let failing = || {
        FakeRunner::new().script(
            "design",
            vec![
                Step::fail(NodeFailure::Exit { code: Some(1) }),
                Step::Ok {
                    writes: Some("design ran\n".to_string()),
                },
                Step::fail(NodeFailure::Exit { code: Some(1) }),
            ],
        )
    };
    let probe = FaultBackend::new();
    let _ = process(
        probe.clone(),
        &loaded,
        tempfile::tempdir().expect("t").path(),
        &failing(),
    );
    let passed = append_index(&probe, |e| matches!(e, FlowRunEvent::AttemptPassed { .. }));

    let dir = tempfile::tempdir().expect("tempdir");
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: passed, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &failing());
    let runner = failing();
    let second = first.crash();
    let Process::Drove(report) = process(second.clone(), &loaded, dir.path(), &runner) else {
        panic!("the run could not be picked up");
    };

    // Attempt 3 fails too, and is the last: the verdict on attempt 1 and
    // this one spend both tries; the crash on attempt 2 spent none.
    assert!(
        matches!(
            &report.outcome,
            RunOutcome::Failed { node, failure: NodeFailure::Exit { .. } } if node == "design"
        ),
        "{:?}",
        report.outcome
    );
    let attempts: Vec<u32> = runner.calls().iter().map(|call| call.attempt).collect();
    assert_eq!(attempts, [3], "the failed attempt was forgotten");
}

/// **D4-1, D4-2.** A stopped attempt counts like a failed one: picked up
/// after a crash that kept the stop but lost the run's end, a node allowed
/// two tries gets one more, not two.
#[test]
fn a_stopped_attempt_still_counts() {
    let loaded = retrying_design(2);
    let stopping = || FakeRunner::new().cancel_at("design", 1);
    let probe = FaultBackend::new();
    let _ = process(
        probe.clone(),
        &loaded,
        tempfile::tempdir().expect("t").path(),
        &stopping(),
    );
    let ended = append_index(&probe, |e| matches!(e, FlowRunEvent::RunEnded { .. }));

    let dir = tempfile::tempdir().expect("tempdir");
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: ended, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &stopping());
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let Process::Drove(report) = process(first.crash(), &loaded, dir.path(), &runner) else {
        panic!("the run could not be picked up");
    };

    assert!(
        matches!(
            &report.outcome,
            RunOutcome::Failed { node, failure: NodeFailure::Exit { .. } } if node == "design"
        ),
        "{:?}",
        report.outcome
    );
    let attempts: Vec<u32> = runner.calls().iter().map(|call| call.attempt).collect();
    assert_eq!(attempts, [2], "the stop did not count");
}

/// **D4-2.** A stop that used up the node's only try still blocks it: picked
/// up after a crash that kept the stop but lost the run's end, the node does
/// not run past its cap, and the run ends the way that stop would have.
#[test]
fn a_stop_that_used_up_the_cap_is_not_run_past_it() {
    // No retry policy: one try, which the stop spends.
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
";
    let loaded = load(flow, None).expect("valid flow");
    let stopping = || FakeRunner::new().cancel_at("design", 1);
    let probe = FaultBackend::new();
    let _ = process(
        probe.clone(),
        &loaded,
        tempfile::tempdir().expect("t").path(),
        &stopping(),
    );
    let ended = append_index(&probe, |e| matches!(e, FlowRunEvent::RunEnded { .. }));

    let dir = tempfile::tempdir().expect("tempdir");
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: ended, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &stopping());
    let runner = FakeRunner::new();

    let Process::Drove(report) = process(first.crash(), &loaded, dir.path(), &runner) else {
        panic!("the run could not be picked up");
    };

    assert!(
        matches!(&report.outcome, RunOutcome::Canceled { node: Some(node) } if node == "design"),
        "{:?}",
        report.outcome
    );
    assert!(runner.calls().is_empty(), "the node ran past its cap");
}

/// **F4.** A pass whose output is gone when the run is picked up is undone,
/// and only that node runs again.
#[test]
fn a_lost_output_reruns_only_its_node() {
    let loaded = load(TWO_RETRYING, None).expect("valid flow");
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = FaultBackend::new();
    let _ = process(
        probe.clone(),
        &loaded,
        tempfile::tempdir().expect("t").path(),
        &FakeRunner::new(),
    );
    let end = append_index(&probe, |e| matches!(e, FlowRunEvent::RunEnded { .. }));

    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: end, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &FakeRunner::new());
    std::fs::remove_file(dir.path().join("run/design.md")).expect("lose the output");

    let runner = FakeRunner::new();
    let second = first.crash();
    let Process::Drove(report) = process(second.clone(), &loaded, dir.path(), &runner) else {
        panic!("the run could not be picked up");
    };
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.ids(), vec![NodeId::from("design")]);
    let lost: Vec<NodeId> = records(&second)
        .into_iter()
        .filter_map(|event| match event {
            FlowRunEvent::OutputLost { node } => Some(node),
            _ => None,
        })
        .collect();
    assert_eq!(lost, vec![NodeId::from("design")]);
}

/// **F3.** A crash between invalidating a repair's members and settling the
/// gate: whether or not the members' outputs were moved, the resume moves
/// what is left live and re-derives them, so nothing reads as passed with
/// its output gone.
#[test]
fn a_crash_inside_a_repair_re_derives_what_it_invalidated() {
    let loaded = load(GATED, None).expect("valid flow");
    let failing_gate = || {
        FakeRunner::new().script(
            "gate",
            vec![
                Step::fail(NodeFailure::Exit { code: Some(1) }),
                Step::Ok { writes: None },
            ],
        )
    };
    let probe = FaultBackend::new();
    let _ = process(
        probe.clone(),
        &loaded,
        tempfile::tempdir().expect("t").path(),
        &failing_gate(),
    );
    let invalidated = append_index(&probe, |e| matches!(e, FlowRunEvent::Invalidated { .. }));

    for archived in [true, false] {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path().join("run");
        // The gate's settle, right after the invalidation and the archive.
        let first = FaultBackend::new().with_fault(Fault::TornAppend {
            k: invalidated + 1,
            keep: 0,
        });
        let _ = process(first.clone(), &loaded, dir.path(), &failing_gate());
        let moved = run_dir.join("logs/review.attempt-1.evidence-3.md");
        assert!(moved.is_file(), "the archive ran before the crash");
        if !archived {
            // As if the crash had come before the archive.
            std::fs::rename(&moved, run_dir.join("review.md")).expect("put it back");
        }

        let runner = failing_gate();
        let second = first.crash();
        let Process::Drove(report) = process(second.clone(), &loaded, dir.path(), &runner) else {
            panic!("archived={archived}: the run could not be picked up");
        };
        assert!(
            matches!(report.outcome, RunOutcome::Done),
            "archived={archived}: {:?}",
            report.outcome
        );
        assert_eq!(
            runner.ids(),
            vec![NodeId::from("review"), NodeId::from("gate")],
            "archived={archived}: the invalidated member was not re-derived"
        );
        let state = state_of(&records(&second));
        for id in state.finished() {
            if state.node(&id).map(|n| n.produces) == Some(Produces::File) {
                assert!(
                    run_dir.join(format!("{id}.md")).is_file(),
                    "archived={archived}: `{id}` passed with no output"
                );
            }
        }
        if !archived {
            let swept = std::fs::read_dir(run_dir.join("logs"))
                .expect("logs")
                .filter_map(Result::ok)
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("review.canceled")
                });
            assert!(swept, "the live invalidated output was not moved aside");
        }
    }
}

/// A log whose fsync takes half a second once `slow` is set.
struct SlowSync {
    inner: daruda_runlog::FileBackend,
    slow: Arc<AtomicBool>,
}

impl LogBackend for SlowSync {
    fn generations(&self) -> std::io::Result<Vec<u64>> {
        self.inner.generations()
    }
    fn read(&self, generation: u64) -> std::io::Result<daruda_runlog::Generation> {
        self.inner.read(generation)
    }
    fn select(&mut self, generation: u64) -> std::io::Result<()> {
        self.inner.select(generation)
    }
    fn append(&mut self, line: &[u8]) -> std::io::Result<()> {
        self.inner.append(line)
    }
    fn sync(&mut self) -> std::io::Result<()> {
        if self.slow.load(Ordering::SeqCst) {
            std::thread::sleep(SLOW_SYNC);
        }
        self.inner.sync()
    }
    fn truncate_tail(&mut self, len: u64) -> std::io::Result<()> {
        self.inner.truncate_tail(len)
    }
    fn replace_snapshot(&mut self, generation: u64, bytes: &[u8]) -> std::io::Result<()> {
        self.inner.replace_snapshot(generation, bytes)
    }
    fn drop_generation(&mut self, generation: u64) -> std::io::Result<()> {
        self.inner.drop_generation(generation)
    }
}

const SLOW_SYNC: Duration = Duration::from_millis(500);

/// `left` slows the log down and stops the run from another thread while
/// its own pass is being written; `right` watches for the stop.
struct StopDuringSlowWrite {
    inner: FakeRunner,
    slow: Arc<AtomicBool>,
    stopped_at: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    seen_at: std::cell::Cell<Option<std::time::Instant>>,
}

impl NodeRunner for StopDuringSlowWrite {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            if ctx.node_id == "left" {
                self.slow.store(true, Ordering::SeqCst);
                let (cancel, stopped_at) = (ctx.cancel.clone(), self.stopped_at.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    *stopped_at.lock().expect("lock") = Some(std::time::Instant::now());
                    cancel.cancel();
                });
            } else {
                while !ctx.cancel.is_canceled() {
                    crate::runner::sleep(Duration::from_millis(5)).await;
                }
                self.seen_at.set(Some(std::time::Instant::now()));
            }
            self.inner.run_agent(ctx, agent, prompt).await
        })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.inner.run_command(ctx, run)
    }
}

/// **F11, S12.** A slow fsync holds up only the committer thread: a sibling
/// in the same wave sees a stop at its own polling pace, not after the write.
#[test]
fn a_slow_log_write_does_not_stall_the_rest_of_the_wave() {
    let flow = "\
version: 1
defaults: { agent: { id: claude }, parallel: 2 }
nodes:
  - id: left
    kind: agent
    cwd: a
    output: left.md
    prompt: write
  - id: right
    kind: agent
    cwd: b
    output: right.md
    prompt: write
";
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let log_dir = crate::run_log::log_dir(&dir.path().join("run"));
    std::fs::create_dir_all(&log_dir).expect("mkdir");
    let slow = Arc::new(AtomicBool::new(false));
    let backend = SlowSync {
        inner: daruda_runlog::FileBackend::new(log_dir),
        slow: slow.clone(),
    };
    let runner = StopDuringSlowWrite {
        inner: FakeRunner::new(),
        slow,
        stopped_at: Arc::new(std::sync::Mutex::new(None)),
        seen_at: std::cell::Cell::new(None),
    };
    let loaded = load(flow, None).expect("valid flow");

    let _ = process(backend, &loaded, dir.path(), &runner);

    let stopped = runner
        .stopped_at
        .lock()
        .expect("lock")
        .expect("the stop was sent");
    let seen = runner.seen_at.get().expect("the sibling saw the stop");
    let lag = seen.saturating_duration_since(stopped);
    assert!(
        lag < SLOW_SYNC / 2,
        "the sibling saw the stop {lag:?} late, behind a {SLOW_SYNC:?} write"
    );
}

/// Panics inside its runner call.
struct Panics;

impl NodeRunner for Panics {
    fn run_agent<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _agent: &'a crate::model::AgentSpec,
        _prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        panic!("a runner bug");
    }

    fn run_command<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        panic!("a runner bug");
    }
}

/// **F5, S7.** A run thread that panics gives its lock back, so the next run
/// in the same working tree starts instead of being refused as held.
#[test]
fn a_panicking_run_gives_its_lock_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&request, &Panics, &CancelToken::default())
    }));
    assert!(panicked.is_err(), "the runner was meant to panic");

    let mut next = request_for(CHAIN, dir.path());
    next.run_dir = dir.path().join(".daruda/flow-runs/01K");
    let report = execute(&next, &FakeRunner::new(), &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
}

/// **S6.** What a watcher was sent is what the log replays to: the stream
/// is the log's narration, number for number.
#[test]
fn the_stream_is_the_log_replayed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (tx, rx) = smol::channel::unbounded();
    let mut request = request_for(GATED, dir.path());
    request.events = Some(tx);
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let live: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
        .map(|notice| match notice {
            crate::event::RunNotice::Recorded(said) => format!("{} {:?}", said.seq, said.event),
            crate::event::RunNotice::Unrecorded(end) => panic!("an unrecorded end: {end:?}"),
            crate::event::RunNotice::Settling { wait } => panic!("a fresh run waited {wait:?}"),
        })
        .collect();
    let replayed: Vec<String> = crate::run_log::reader::narration(&report.run_dir)
        .into_iter()
        .map(|said| format!("{} {:?}", said.seq, said.event))
        .collect();
    assert_eq!(live, replayed);
    assert!(live.len() > 10, "too little narrated to compare: {live:?}");
}
