//! Runs a validated flow: a wave at a time in topological order, judging
//! each node before the next wave is chosen, and driving each node's
//! `on_fail` policy until it passes or gives up.
//!
//! How many run at once is the flow's `parallel`, and which of them may
//! overlap is `ready::take_ready_batch`'s answer — nodes that could
//! write in one directory never share a wave.

use crate::NodeId;
use crate::contract::file::FileContract;
use crate::error::{FlowIoError, IoSite};
use crate::graph::FlowGraph;
use crate::model::{Flow, Node, NodeKind};
use crate::record::{AttemptOutcome, Invalidation, Reported};
use crate::request::{AskGitStatus, Budget};
use crate::run_log::machine::FlowRunEvent;
use crate::run_log::store::FlowRunStore;
use crate::runner::{
    AskOwner, CancelToken, NodeFailure, NodeRunner, OutputContract, RunContext, RunResult,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// The id the fix session is recorded under. It is not a node — it has no
/// declaration, no output and no place in the graph — but it is a session
/// the run made, so the record needs a name for it. A flow declaring a node
/// with this id would collide; `crate::validate` rejects that.
pub const FIX_SESSION_ID: &str = "__fix__";

/// The one name inside a run directory the engine owns. Archive and runner
/// artifacts land here under names built from node ids, so a node writing
/// its `output` here could collide with them; `crate::validate` rejects
/// that, reading this same constant.
pub const LOG_DIR_NAME: &str = "logs";

/// What the engine was doing when a path failed it. Collected here so the
/// wording is one string per site rather than a literal at each `Err` arm.
mod doing {
    pub const OUTPUT_PARENT: &str = "creating the output directory";
    pub const READ_PROMPT: &str = "reading the prompt file";
    pub const READ_HINT: &str = "reading the hint file";
    pub const ARCHIVE: &str = "archiving the failed attempt";
    pub const ARCHIVE_CANCELED: &str = "archiving the canceled node's output";
}

/// One run's shared state. Interior mutability because driving a node can
/// recurse — a gate's repair re-runs nodes that may themselves repair.
struct Run<'a> {
    flow: &'a Flow,
    graph: &'a FlowGraph,
    flow_dir: &'a Path,
    cwd: &'a Path,
    run_dir: &'a Path,
    log_dir: PathBuf,
    /// Every node's resolved output path, for `{{node.<id>.output}}`.
    node_outputs: HashMap<NodeId, PathBuf>,
    runner: &'a dyn NodeRunner,
    cancel: &'a CancelToken,
    budget: &'a Budget,
    /// Asked once per attempt, wherever that attempt's fate is sealed.
    git_status: Option<AskGitStatus>,
    /// The run's log: every attempt and its numbers are committed here first.
    log: FlowRunStore,
    /// Nodes an earlier process finished, empty for a run that is starting.
    already_passed: Vec<NodeId>,
    /// The profile this run went by, for its record.
    profile: Option<String>,
    /// The node this run was asked to stop at, for its record. The
    /// worklist already applied it; `finish` only writes it down.
    until: Option<NodeId>,
    /// Nodes reused rather than run, for its record.
    pinned: Vec<NodeId>,
    /// Where a `permission: ask` node puts its question. `None` is a host
    /// that cannot answer, which `validate_request` has already refused
    /// for any flow that could reach `Ask`.
    ask: Option<crate::runner::AskChannel>,
    /// What the run has spent and what it has to say about it. Owned as
    /// one value so the drive loop cannot reach counters outside the
    /// accounted-call and correction paths — see [`budget::Accounting`].
    spent: budget::Accounting,
    /// Nodes that have produced a result this run, in run order. The rerun
    /// set is intersected with this — a node that never ran has nothing to
    /// re-derive.
    executed: RefCell<Vec<NodeId>>,
    /// Nodes whose `drive` call is currently on the stack. Two gates that
    /// both re-run from a shared ancestor would otherwise drive each other
    /// forever — the recursion is bounded by structure, not by a counter,
    /// because a stack overflow aborts the process before any run-level
    /// budget could notice.
    driving: RefCell<HashSet<NodeId>>,
}

/// Everything one drive needs, borrowed. `execute` builds it from a
/// `RunRequest`; the crate's own tests build it directly, which is what
/// keeps `run_flow` reachable without the lock-and-marker ceremony.
///
/// `pub(crate)` deliberately: this and [`run_flow`] exist for that test
/// seam, and a host has exactly one way in — [`execute`]. Exported, the
/// nine fields below would be contract, and "opened so tests are easier"
/// would be indistinguishable from "opened because a host needs it".
pub(crate) struct RunInputs<'a> {
    pub(crate) loaded: &'a crate::load::LoadedFlow,
    pub(crate) flow_dir: &'a Path,
    pub(crate) cwd: &'a Path,
    pub(crate) run_dir: &'a Path,
    pub(crate) cancel: &'a CancelToken,
    pub(crate) budget: &'a Budget,
    /// How the host reports the working tree's state. `None` is a host with
    /// nothing to say, which the record treats as an absent note, not an
    /// error.
    pub(crate) git_status: Option<AskGitStatus>,
    /// The run's log, already holding `RunStarted` (or a resume's records).
    /// It also narrates the run, once each record is durable.
    pub(crate) log: &'a FlowRunStore,
    /// Where a person is asked for permission. `None` is a host with no
    /// answering surface.
    pub(crate) ask: Option<&'a smol::channel::Sender<crate::runner::PendingAsk>>,
    /// Run no further than this node. Resolved by the caller so a resume
    /// re-applies the selection the first process ran under.
    pub(crate) until: Option<NodeId>,
    /// Nodes whose output was copied in rather than computed. Treated as
    /// done, and recorded apart from a resume's carried-over set — one was
    /// asked for, the other is what a crash left behind.
    pub(crate) pinned: Vec<NodeId>,
    /// What an earlier process finished, when this is a continuation.
    pub(crate) resume: Option<crate::run_log::reader::Replay>,
}

/// Run every node in topological order, judging each before the next.
pub(crate) async fn run_flow(inputs: RunInputs<'_>, runner: &dyn NodeRunner) -> RunReport {
    let RunInputs {
        loaded,
        flow_dir,
        cwd,
        run_dir,
        cancel,
        budget,
        git_status,
        log,
        ask,
        until,
        pinned,
        resume,
    } = inputs;
    let (flow, graph) = (loaded.flow(), loaded.graph());
    // Destructured before the run is built: every one of these is a field
    // the run starts from rather than something it can ask for later.
    let (passed, spent, profile) = match resume {
        Some(replay) => (
            replay.passed,
            budget::Accounting::resumed(replay.spent, replay.records),
            // The record's, not the spec's: `run.yaml` records the
            // settings a profile produced and never the name.
            replay.profile,
        ),
        None => (
            Vec::new(),
            budget::Accounting::default(),
            flow.profile.clone(),
        ),
    };
    let spent = spent.with_clock(log.clock());
    let run = Run {
        flow,
        graph,
        flow_dir,
        cwd,
        run_dir,
        log_dir: run_dir.join(LOG_DIR_NAME),
        node_outputs: node_outputs(flow, run_dir).into_iter().collect(),
        runner,
        cancel,
        budget,
        git_status,
        log: log.clone(),
        ask: ask
            .cloned()
            .map(|tx| crate::runner::AskChannel::new(tx, log.clone())),
        spent,
        profile,
        until: until.clone(),
        pinned: pinned.clone(),
        // Seeded: a repair's `∩ executed` filter asks which nodes have run,
        // and the ones from before the crash did.
        executed: RefCell::new(passed.clone()),
        already_passed: passed,
        driving: RefCell::new(HashSet::new()),
    };

    let mut outcome = RunOutcome::Done;

    // A worklist, not a walk: it makes "which nodes could start now" a
    // question the loop asks rather than assumes, which is what running two
    // at once needs.
    //
    // Nodes an earlier process finished, and pinned ones, start out done —
    // not skipped inside `drive`, where a blanket skip would turn every
    // repair into a no-op. Pins stay out of `already_passed`, which means
    // "a crash left this finished".
    let mut done: HashSet<NodeId> = run
        .already_passed
        .iter()
        .chain(run.pinned.iter())
        .cloned()
        .collect();
    // A selection is applied to the worklist, not to the flow: `run.yaml`
    // records what the flow says, and the ready-set filter stays a question
    // about dependencies.
    let selected = crate::graph::Selection::of(flow, until.as_ref());
    let mut waiting: Vec<NodeId> = graph
        .topological_order()
        .into_iter()
        .filter(|id| !done.contains(id))
        .filter(|id| selected.includes(id))
        .collect();

    // Once for the run: which directories a node could write in is a
    // question about the flow, and the flow does not change while it runs.
    // Whether those directories resolve is asked every wave — see
    // `ready::Reachability`.
    //
    // After `waiting`, and for exactly it: a selection or a pin leaves
    // nodes nothing will ask about, and the fixpoint is not free.
    let reach = ready::Reachability::of(flow, graph, cwd, &waiting);
    while !waiting.is_empty() {
        let batch = match take_ready_batch(flow, &reach, &mut waiting, &done, flow.parallel) {
            ready::Batch::Ready(batch) => batch,
            // Every node left waits on a dependency nobody will finish.
            // Only a cycle can produce that, and `FlowGraph::build`
            // refuses those — so this is a graph nobody could have handed
            // us, and the run ends on whatever `outcome` already holds.
            ready::Batch::Exhausted => break,
            // Its own outcome, not `Done`: the run stopped with work still
            // to do. A cancel or a spent budget outranks it — naming the
            // hold would blame the filesystem for a decision made
            // elsewhere.
            //
            // No loop here. A hold that leaves other work runnable never
            // reaches this arm, so re-asking would be a spin; the recovery
            // is that `Stalled` resumes (`crate::resume`).
            ready::Batch::Held(nodes) => {
                outcome = run
                    .stop_before_more_work()
                    .unwrap_or(RunOutcome::Stalled { nodes });
                break;
            }
        };
        // A wave at a time: everything started together is awaited together
        // before the next set is chosen. A rolling window would start
        // newly-ready nodes sooner, and would also mean a node still in
        // flight when another one fails — which is the case that needs
        // cancelling half-written work out of a directory. Waves have no
        // such case: when the run stops, nothing is running.
        let results = join_all(batch.iter().map(|id| run.drive(id)).collect()).await;
        for (id, result) in batch.into_iter().zip(results) {
            match result {
                Ok(()) => {
                    done.insert(id);
                }
                // The first failure in declaration order, because the batch
                // was chosen in that order — so which of two simultaneous
                // failures ends the run does not depend on which agent was
                // slower.
                Err(stopped) if matches!(outcome, RunOutcome::Done) => outcome = stopped,
                // A node kept by its own stop is `node: None`; the sibling
                // it interrupted names where the run was actually cut.
                Err(stopped @ RunOutcome::Canceled { node: Some(_) })
                    if matches!(outcome, RunOutcome::Canceled { node: None }) =>
                {
                    outcome = stopped;
                }
                Err(_) => {}
            }
        }
        if !matches!(outcome, RunOutcome::Done) {
            break;
        }
    }
    run.finish(outcome)
}

impl<'a> Run<'a> {
    /// Run one node until it passes or its policy gives up. `Err` carries
    /// the outcome the whole run ends with.
    ///
    /// Boxed because it recurses: a gate's repair re-derives a set whose
    /// members have policies of their own.
    ///
    /// The lifetime is the method's own, not the struct's. Tying it to
    /// `Run<'a>` typechecks only while every field stays covariant in `'a`;
    /// one invariant field would break both call sites at once with an
    /// error that points nowhere useful.
    fn drive<'s>(
        &'s self,
        id: &'s NodeId,
    ) -> Pin<Box<dyn Future<Output = Result<(), RunOutcome>> + 's>> {
        Box::pin(async move {
            let Some(node) = self.flow.nodes.iter().find(|n| &n.id == id) else {
                return Ok(());
            };

            self.driving.borrow_mut().insert(id.clone());
            let result = self.drive_inner(node, id).await;
            self.driving.borrow_mut().remove(id);
            result
        })
    }

    fn drive_inner<'s>(
        &'s self,
        node: &'s Node,
        id: &'s NodeId,
    ) -> Pin<Box<dyn Future<Output = Result<(), RunOutcome>> + 's>> {
        Box::pin(async move {
            let policy = self.policy_of(node);
            // A resume can find this node's attempts in this generation
            // already spent; one a crash interrupted is not (D4-2).
            if let Some(spent) = self.attempts_spent_before(id, &policy) {
                return Err(spent);
            }
            // Accumulated across this generation's attempts, because a
            // repair reads "the previous attempts'" evidence.
            let mut evidence: Vec<PathBuf> = Vec::new();
            // The failure the next attempt has to answer. `None` on the
            // first attempt, which is why a hint never appears there.
            let mut last_failure: Option<NodeFailure> = None;

            loop {
                // Between nodes and between attempts: nothing is in flight,
                // so there is no output to move and no node to attribute.
                if let Some(outcome) = self.stop_before_more_work() {
                    return Err(outcome);
                }

                let output = self.node_outputs.get(id).cloned();
                // The node's own, when it names one. Validation has already
                // established it is inside the run's — which is what keeps
                // the run's single lock covering everywhere it works.
                let cwd = self.node_cwd(node);
                // A local because `ctx` borrows it; when there is no output
                // there is no contract, whatever the node's kind.
                let contract = output.as_deref().map(|p| {
                    FileContract::new(
                        self.run_dir,
                        p,
                        node.kind.output_schema(),
                        node.kind.continue_until(),
                    )
                });
                // The number the attempt will get, for the failures below
                // that happen before one exists.
                let next = self.next_attempt(id);

                // Before the directory is made and before the runner is
                // called: `create_dir_all` follows a link an earlier node
                // planted, so a write through one cannot be caught after
                // the fact — only prevented.
                if let Some(path) = output.as_deref()
                    && let Err(failure) = crate::contract::file::preflight(self.run_dir, path)
                {
                    // No session was paid for, so there is no attempt. The
                    // refusal is still recorded: a host drawing the graph
                    // has to be told which node stopped the run.
                    let refused = FlowRunEvent::node_refused(id.clone(), (&failure).into());
                    self.log
                        .commit(refused)
                        .await
                        .map_err(|e| self.log.failure(site(id, next), e))?;
                    return Err(RunOutcome::Failed {
                        node: id.clone(),
                        failure,
                    });
                }

                // An agent writes into `run_dir`, and a nested `output`
                // such as `reports/out.md` is legal — nothing else creates
                // that directory, so a real agent would fail the same way
                // the fake does.
                if let Some(parent) = output.as_deref().and_then(|p| p.parent())
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    return Err(io_at(
                        id,
                        next,
                        doing::OUTPUT_PARENT,
                        parent.to_path_buf(),
                        e,
                    ));
                }

                let prompt =
                    match self.node_text(node, output.as_deref(), &evidence, last_failure.as_ref())
                    {
                        Ok(text) => text,
                        Err((doing, path, e)) => return Err(io_at(id, next, doing, path, e)),
                    };

                // O2: the attempt and its evidence number are durable before
                // the session is paid for, so a crash cannot reuse them (F2).
                let started = self.start_attempt(id, next).await?;
                // A local because `ctx` borrows it, like `contract` above.
                let reserve = || self.reserve_extra_turn();
                // Queued, not awaited: the runner reports from inside its
                // spawn, and the committer keeps this ahead of the settle.
                let spawned = |group: crate::runner::SpawnedGroup| {
                    self.log.commit_detached(FlowRunEvent::ProcessSpawned {
                        node: id.clone(),
                        evidence: started.evidence,
                        leader: group.leader,
                        identity: group.identity.to_raw(),
                    });
                };
                let owner = AskOwner::Node {
                    node: id.clone(),
                    generation: started.generation,
                    attempt: started.attempt,
                };
                let ctx = RunContext {
                    max_turns: node.kind.max_turns(),
                    node_id: id,
                    attempt: started.attempt,
                    started_at: std::time::SystemTime::now(),
                    cwd: &cwd,
                    run_dir: self.run_dir,
                    log_dir: &self.log_dir,
                    output: output.as_deref(),
                    contract: contract.as_ref().map(|c| c as &dyn OutputContract),
                    evidence_seq: started.evidence,
                    // The node's own budget, but never past the run's
                    // deadline: a runner races only what it is given, so a
                    // one-minute run ceiling would not stop a ten-minute
                    // node if the node's number came through unclipped.
                    timeout: self.bounded_timeout(node.timeout),
                    permission: self.permission_for(node, owner),
                    cancel: self.cancel,
                    reserve_extra_turn: &reserve,
                    spawned: &spawned,
                };
                let attempt = ctx.attempt;
                let result = self.call(node, &ctx, &prompt).await;

                // Before `judge`, not after: a cancelled node judged as a
                // failure would fire its `on_fail` policy and re-run the
                // node the user just stopped. Its output goes aside too —
                // everything left live has to be a completed node's.
                // The exception is a clean turn whose own report stopped the
                // run: it is judged and kept, and the run ends after it.
                let stopped_here = result.stopped_the_run && result.outcome.is_ok();
                if self.cancel.is_canceled() && !stopped_here {
                    return Err(self
                        .settle_cancel(&ctx, id, output.as_deref(), &result)
                        .await);
                }
                let artifacts = result.artifacts.clone();
                // Captured before `judge` consumes the result, like
                // `artifacts` — every settle below is for this attempt.
                let reported = Reported::from(&result);

                let failure = match judge(&ctx, result) {
                    Ok(()) => {
                        // O1: the output is on disk before its pass is.
                        if let Some(path) = output.clone() {
                            durable::sync_file(path).await;
                        }
                        // A pass archives nothing: its output stays live for
                        // the nodes downstream to read.
                        self.settle(
                            &ctx,
                            AttemptOutcome::Passed,
                            Invalidation::default(),
                            reported,
                        )
                        .await?;
                        let mut executed = self.executed.borrow_mut();
                        if !executed.contains(id) {
                            executed.push(id.clone());
                        }
                        if stopped_here {
                            return Err(RunOutcome::Canceled { node: None });
                        }
                        return Ok(());
                    }
                    Err(failure) => failure,
                };

                // A refusal cannot be argued with, and a node with no retry
                // policy has nothing to try. A run this node stopped retries
                // nothing either; it ends `Canceled`.
                if stopped_here
                    || failure.forbids_retry()
                    || self.spent_attempts(id) >= policy.max_attempts
                {
                    // This returns before any invalidation set is computed,
                    // so the attempt that ends the run archives nothing.
                    self.settle(
                        &ctx,
                        AttemptOutcome::Failed(failure.clone()),
                        Invalidation::default(),
                        reported,
                    )
                    .await?;
                    if stopped_here {
                        return Err(RunOutcome::Canceled { node: None });
                    }
                    return Err(RunOutcome::Failed {
                        node: id.clone(),
                        failure,
                    });
                }

                // Archive the whole invalidation set before anything else
                // runs — a fix that starts first would read a live output
                // instead of the evidence it is supposed to answer.
                let set = self.invalidation_set(node, &policy);
                // O2: which outputs stop being current is durable before any
                // of them is moved, so a crash in between is a resume's to
                // finish rather than a pass with no output (F3).
                let members: Vec<NodeId> = set
                    .iter()
                    .map(|(member, _)| member.clone())
                    .filter(|member| member != id)
                    .collect();
                if !members.is_empty() {
                    let gate = id.clone();
                    self.log
                        .commit(FlowRunEvent::Invalidated { gate, members })
                        .await
                        .map_err(|e| self.log.failure(site(id, attempt), e))?;
                }
                match crate::archive::archive_attempt(
                    &self.log_dir,
                    &set,
                    attempt,
                    ctx.evidence_seq,
                    &artifacts,
                ) {
                    Ok(paths) => {
                        durable::sync_dirs(&set, &self.log_dir).await;
                        // The archive's own return, not a second derivation
                        // of it: two computations of the same set drift.
                        let invalidation = Invalidation {
                            nodes: set.iter().map(|(id, _)| id.clone()).collect(),
                            archived: paths.clone(),
                        };
                        self.settle(
                            &ctx,
                            AttemptOutcome::Failed(failure.clone()),
                            invalidation,
                            reported,
                        )
                        .await?;
                        evidence.extend(paths);
                    }
                    Err(e) => {
                        // Same reasoning as the cancel path: the attempt is
                        // in the accounting whether or not its evidence
                        // could be moved, so it is in the record too.
                        self.settle(
                            &ctx,
                            AttemptOutcome::Failed(failure.clone()),
                            Invalidation::default(),
                            reported,
                        )
                        .await?;
                        let (path, source) = e.into_io();
                        return Err(node_io(&ctx, doing::ARCHIVE, path, source));
                    }
                }

                if !policy.wait.is_zero() {
                    // ALLOW: this crate is GPUI-free, so there is no
                    // BackgroundExecutor to time on. `wait` is resolved and
                    // clamped config, and the suite sets it explicitly.
                    #[allow(clippy::disallowed_methods)]
                    let timer = smol::Timer::after(policy.wait);
                    timer.await;
                }

                // The wait is the longest the run spends doing nothing, so a
                // stop that arrives during it must land here rather than
                // after a fresh agent session has already been paid for.
                if let Some(outcome) = self.stop_before_more_work() {
                    return Err(outcome);
                }

                if let PolicyKind::Repair { fix, rerun } = &policy.kind {
                    self.repair(fix, rerun, &ctx, id, &evidence, &failure)
                        .await?;
                }

                last_failure = Some(failure);
            }
        })
    }

    /// Where a node runs: its own directory, or the run's.
    ///
    /// The spelling the file used, not the resolved form — a runner shows
    /// this to a person and reports it in the record, and a canonical path
    /// with every symlink expanded is not what they wrote. Overlap safety
    /// asks the resolved question separately, in `working_tree_of`.
    fn node_cwd(&self, node: &Node) -> PathBuf {
        match &node.cwd {
            Some(relative) => self.cwd.join(relative),
            None => self.cwd.to_path_buf(),
        }
    }

    /// Dispatch finished text through the runner and its accounting funnel.
    async fn call(&self, node: &Node, ctx: &RunContext<'_>, text: &str) -> RunResult {
        match &node.kind {
            NodeKind::Agent(body) => {
                self.accounted_call(|| self.runner.run_agent(ctx, &body.agent, text))
                    .await
            }
            NodeKind::Command { .. } => {
                self.accounted_call(|| self.runner.run_command(ctx, text))
                    .await
            }
        }
    }
}

/// Attribute an I/O failure to the attempt that was in flight. One place
/// builds the site so every node-level `Err` arm reads the same.
fn node_io(
    ctx: &RunContext<'_>,
    doing: &'static str,
    path: PathBuf,
    source: std::io::Error,
) -> RunOutcome {
    RunOutcome::Io(FlowIoError {
        site: IoSite::Node {
            node: ctx.node_id.clone(),
            attempt: ctx.attempt,
        },
        doing,
        path,
        source,
    })
}

/// The absolute path an agent node owes, resolved against `run_dir`.
/// Every node that writes, and where. One derivation, because a resume
/// sweeps the same paths the run is about to write to — two would be two
/// chances to disagree about which file belongs to which node.
pub(crate) fn node_outputs(flow: &Flow, run_dir: &Path) -> Vec<(NodeId, PathBuf)> {
    flow.nodes
        .iter()
        .filter_map(|n| node_output(n, run_dir).map(|p| (n.id.clone(), p)))
        .collect()
}

pub(crate) fn node_output(node: &Node, run_dir: &Path) -> Option<PathBuf> {
    match &node.kind {
        NodeKind::Agent(body) => Some(run_dir.join(&body.output)),
        NodeKind::Command { .. } => None,
    }
}

/// Every future to completion, in one place.
///
/// Hand-rolled because `futures-lite` — what `smol` brings — has `zip` for
/// two and nothing for a list, and one screenful here is a better trade
/// than a dependency the rest of the crate would inherit. Re-polling the
/// pending ones on every wake is wasteful in principle and invisible at
/// eight futures, none of which is CPU-bound.
async fn join_all<T>(mut futures: Vec<Pin<Box<dyn Future<Output = T> + '_>>>) -> Vec<T> {
    let mut settled: Vec<Option<T>> = (0..futures.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (slot, future) in settled.iter_mut().zip(futures.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Ready(value) => *slot = Some(value),
                std::task::Poll::Pending => pending = true,
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    settled.into_iter().flatten().collect()
}

/// A runner reporting success is necessary but not sufficient for a node
/// that owes a file: a turn that ended cleanly without writing anything
/// still fails.
///
/// The runner's verdict is answered first: asking the filesystem about an
/// attempt that already failed would only rename its failure.
fn judge(ctx: &RunContext<'_>, result: RunResult) -> Result<(), NodeFailure> {
    result.outcome?;
    if let Some(contract) = ctx.contract {
        contract.check()?;
    }
    Ok(())
}

mod budget;
mod durable;
mod permission;
mod policy;
mod prompt;
mod ready;
mod repair;
mod report;
mod run;
mod settle;

pub use report::{BudgetLimit, RunOutcome, RunReport};

pub use run::execute;

use settle::{io_at, site};

use policy::PolicyKind;
use ready::take_ready_batch;

#[cfg(test)]
pub(crate) mod tests;
