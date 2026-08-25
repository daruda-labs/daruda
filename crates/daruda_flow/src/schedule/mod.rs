//! Runs a validated flow: topological order, one node at a time, judging
//! each before moving on, and driving each node's `on_fail` policy until it
//! passes or gives up. Serial by design — every node shares one working
//! directory, so two running at once would corrupt each other.

use crate::NodeId;
use crate::contract::file::FileContract;
use crate::error::{FlowIoError, IoSite};
use crate::event::FlowEvent;
use crate::graph::FlowGraph;
use crate::model::{AgentSpec, Flow, Node, NodeKind};
use crate::record::{AttemptOutcome, AttemptRecord, GitStatus, Invalidation, Reported};
use crate::request::Budget;
use crate::runner::{CancelToken, NodeFailure, NodeRunner, OutputContract, RunContext, RunResult};
use std::cell::{Cell, RefCell};
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
    git_status: GitStatus<'a>,
    /// Nodes an earlier process finished, empty for a run that is starting.
    already_passed: Vec<NodeId>,
    /// The profile this run went by, for its record.
    profile: Option<String>,
    /// The node this run was asked to stop at, for its record. The
    /// worklist already applied it; `finish` only writes it down.
    until: Option<NodeId>,
    /// Nodes reused rather than run, for its record.
    pinned: Vec<NodeId>,
    /// Where the run narrates itself, or `None` when nobody is watching.
    events: Option<&'a smol::channel::Sender<FlowEvent>>,
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
    /// Never reset, unlike a policy attempt, so archive names cannot
    /// collide across repair generations.
    next_seq: Cell<u32>,
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
    pub(crate) git_status: GitStatus<'a>,
    /// Where the run narrates itself. `None` is a host that does not watch.
    pub(crate) events: Option<&'a smol::channel::Sender<FlowEvent>>,
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
    pub(crate) resume: Option<crate::journal::Replay>,
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
        events,
        ask,
        until,
        pinned,
        resume,
    } = inputs;
    let (flow, graph) = (loaded.flow(), loaded.graph());
    // Destructured before the run is built: every one of these is a field
    // the run starts from rather than something it can ask for later.
    let (passed, next_seq, spent, profile) = match resume {
        Some(replay) => (
            replay.passed,
            // Continued, never restarted: an attempt that re-used a
            // number would write its evidence over the log that is the
            // only account of the attempt before the crash.
            replay.next_seq.max(1),
            budget::Accounting::resumed(replay.spent, replay.records),
            // The journal's, not the spec's: `run.yaml` records the
            // settings a profile produced and never the name.
            replay.profile,
        ),
        None => (
            Vec::new(),
            1,
            budget::Accounting::default(),
            flow.profile.clone(),
        ),
    };
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
        events,
        ask: ask.cloned().map(crate::runner::AskChannel::new),
        spent,
        profile,
        until: until.clone(),
        pinned: pinned.clone(),
        // Seeded: a repair's `∩ executed` filter asks which nodes have run,
        // and the ones from before the crash did.
        executed: RefCell::new(passed.clone()),
        already_passed: passed,
        next_seq: Cell::new(next_seq),
        driving: RefCell::new(HashSet::new()),
    };

    let mut outcome = RunOutcome::Done;

    // A worklist over the topological order rather than a walk down it.
    //
    // The two are the same thing while one node runs at a time: the order
    // is Kahn's with a declaration-ordered ready set, so taking the first
    // *ready* node out of it yields exactly that order back. What the
    // worklist adds is the question "which nodes could start now" as
    // something the loop asks rather than something it assumes — which is
    // what running two at once will need.
    //
    // Nodes an earlier process finished start out done. Not skipped inside
    // `drive`: a gate's repair re-runs nodes *because* they already ran,
    // and a blanket skip there would turn every repair into a no-op.
    // A pin is the user's promise that this output is already valid, so the
    // node is done before the run starts. Kept out of `already_passed`: that
    // one means "a crash left this finished", which is a different story for
    // the record to tell.
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

    while !waiting.is_empty() {
        let batch = take_ready_batch(flow, cwd, &mut waiting, &done, flow.parallel);
        if batch.is_empty() {
            // Nothing ready and nothing in flight. Only a cycle can produce
            // that, and `FlowGraph::build` refuses those — so this is a
            // graph nobody could have handed us.
            break;
        }
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
            // A fresh generation: this node's own counter starts here and
            // only ever increases, which is what `max_attempts` bounds.
            let mut attempt = 1u32;
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

                let seq = self.take_seq();
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
                // A local because `ctx` borrows it, like `contract` above.
                let reserve = || self.reserve_extra_turn();
                let ctx = RunContext {
                    max_turns: node.kind.max_turns(),
                    node_id: id,
                    attempt,
                    started_at: std::time::SystemTime::now(),
                    cwd: &cwd,
                    run_dir: self.run_dir,
                    log_dir: &self.log_dir,
                    output: output.as_deref(),
                    contract: contract.as_ref().map(|c| c as &dyn OutputContract),
                    evidence_seq: seq,
                    // The node's own budget, but never past the run's
                    // deadline: a runner races only what it is given, so a
                    // one-minute run ceiling would not stop a ten-minute
                    // node if the node's number came through unclipped.
                    timeout: self.bounded_timeout(node.timeout),
                    permission: self.permission_for(node),
                    cancel: self.cancel,
                    reserve_extra_turn: &reserve,
                };

                // Before the directory is made and before the runner is
                // called: `create_dir_all` follows a link an earlier node
                // planted, so a write through one cannot be caught after
                // the fact — only prevented.
                if let Some(path) = output.as_deref()
                    && let Err(failure) = crate::contract::file::preflight(self.run_dir, path)
                {
                    // No session was paid for, so there is no attempt to
                    // record. The node still stopped the run, and a host
                    // drawing the graph has to be told which one.
                    self.emit(FlowEvent::NodeFailed {
                        node: id.clone(),
                        attempt,
                        failure: failure.clone(),
                    });
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
                    return Err(node_io(&ctx, doing::OUTPUT_PARENT, parent.to_path_buf(), e));
                }

                let prompt = match self.node_text(node, &ctx, &evidence, last_failure.as_ref()) {
                    Ok(text) => text,
                    Err((doing, path, e)) => return Err(node_io(&ctx, doing, path, e)),
                };
                // Emitted where the session is about to be paid for, so a
                // `NodeStarted` is one runner call and one recorded attempt.
                // Not the converse: the preflight above fails a node with
                // neither.
                self.emit(FlowEvent::NodeStarted {
                    node: id.clone(),
                    attempt,
                });
                let result = self.call(node, &ctx, &prompt).await;

                // Before `judge`, not after: a cancelled node judged as a
                // failure would fire its `on_fail` policy and re-run the
                // node the user just stopped. Its output goes aside too —
                // everything left live has to be a completed node's.
                if self.cancel.is_canceled() {
                    return Err(self.settle_cancel(&ctx, id, output.as_deref(), &result));
                }
                let artifacts = result.artifacts.clone();
                // Captured before `judge` consumes the result, like
                // `artifacts` — every `record` below is for this attempt.
                let reported = Reported::from(&result);

                let failure = match judge(&ctx, result) {
                    Ok(()) => {
                        // A pass archives nothing: its output stays live for
                        // the nodes downstream to read.
                        self.record(
                            &ctx,
                            AttemptOutcome::Passed,
                            Invalidation::default(),
                            reported.clone(),
                        );
                        self.emit(FlowEvent::NodePassed {
                            node: id.clone(),
                            attempt,
                        });
                        let mut executed = self.executed.borrow_mut();
                        if !executed.contains(id) {
                            executed.push(id.clone());
                        }
                        return Ok(());
                    }
                    Err(failure) => failure,
                };
                // One site for all three ways a failed attempt is disposed of
                // — capped, archived, or archived unsuccessfully. What the
                // record separates is `archived`, which no event carries.
                self.emit(FlowEvent::NodeFailed {
                    node: id.clone(),
                    attempt,
                    failure: failure.clone(),
                });

                let policy = self.policy_of(node);
                // A refusal cannot be argued with, and a node with no retry
                // policy has nothing to try.
                if failure.forbids_retry() || attempt >= policy.max_attempts {
                    // This returns before any invalidation set is computed,
                    // so the attempt that ends the run archives nothing.
                    self.record(
                        &ctx,
                        AttemptOutcome::Failed(failure.clone()),
                        Invalidation::default(),
                        reported.clone(),
                    );
                    return Err(RunOutcome::Failed {
                        node: id.clone(),
                        failure,
                    });
                }

                // Archive the whole invalidation set before anything else
                // runs — a fix that starts first would read a live output
                // instead of the evidence it is supposed to answer.
                let set = self.invalidation_set(node, &policy);
                match crate::archive::archive_attempt(&self.log_dir, &set, attempt, seq, &artifacts)
                {
                    Ok(paths) => {
                        // The archive's own return, not a second derivation
                        // of it: two computations of the same set drift.
                        self.record(
                            &ctx,
                            AttemptOutcome::Failed(failure.clone()),
                            Invalidation {
                                nodes: set.iter().map(|(id, _)| id.clone()).collect(),
                                archived: paths.clone(),
                            },
                            reported.clone(),
                        );
                        evidence.extend(paths);
                    }
                    Err(e) => {
                        // Same reasoning as the cancel path: the attempt is
                        // in the accounting whether or not its evidence
                        // could be moved, so it is in the record too.
                        self.record(
                            &ctx,
                            AttemptOutcome::Failed(failure.clone()),
                            Invalidation::default(),
                            reported.clone(),
                        );
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
                attempt += 1;
            }
        })
    }
    /// Every node-level event goes through here; `execute` emits the two
    /// run-level ones the same way, since only it sees the lock.
    fn emit(&self, event: FlowEvent) {
        crate::event::emit(self.events, event);
    }

    /// Called wherever an attempt's fate is sealed — five places, because
    /// three of them return before `judge` or before any archiving. The
    /// `git_status` ask happens here, so the number of asks equals the
    /// number of attempts.
    /// What the runner call reported, for the record. Grouped because all of
    /// it comes from one `RunResult` and every call site was passing the parts
    /// positionally — and `Default` is the refusal that never made a call.
    fn record(
        &self,
        ctx: &RunContext<'_>,
        outcome: AttemptOutcome,
        invalidated: Invalidation,
        reported: Reported,
    ) {
        let attempt = AttemptRecord {
            attempt: ctx.attempt,
            evidence_seq: ctx.evidence_seq,
            at: std::time::SystemTime::now(),
            // `Err` is a clock that moved backwards under us; the duration
            // is then simply not shown, which beats a wrong one.
            took: ctx.started_at.elapsed().unwrap_or_default(),
            outcome,
            invalidated,
            git_status: self.git_status.and_then(|ask| ask(ctx.cwd)),
            waited: reported.waited,
            turns: reported.turns,
            tools: reported.tools,
            usage: reported.usage,
        };
        // On disk before the next node starts, through the same funnel the
        // in-memory record goes through — a second write site is a second
        // chance for the two to disagree about what happened.
        //
        // Best-effort: a journal that cannot be written costs a resume, and
        // failing the run over it would cost the run. The warning is how
        // someone finds out the run stopped being resumable.
        if let Err(e) =
            crate::journal::append_attempt(self.run_dir, ctx.node_id, &attempt, &self.spent.spent())
        {
            self.spent.warn(format!(
                "this run's progress could not be written, so it cannot be resumed: {e}"
            ));
        }
        self.spent
            .record(|records| crate::record::push_attempt(records, ctx.node_id, attempt));
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

    fn take_seq(&self) -> u32 {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        seq
    }

    /// Dispatch finished text through the runner and its accounting funnel.
    async fn call(&self, node: &Node, ctx: &RunContext<'_>, text: &str) -> RunResult {
        match &node.kind {
            NodeKind::Agent { agent, .. } => {
                self.accounted_call(|| self.runner.run_agent(ctx, agent, text))
                    .await
            }
            NodeKind::Command { .. } => {
                self.accounted_call(|| self.runner.run_command(ctx, text))
                    .await
            }
        }
    }

    /// The user stopped the run while this node was in flight.
    ///
    /// Its output goes aside: everything left live in the run directory has
    /// to be a completed node's, or the next reader cannot tell which is
    /// which. The attempt is recorded either way — it happened and it cost
    /// a runner call, whether or not its output could be moved.
    fn settle_cancel(
        &self,
        ctx: &RunContext<'_>,
        id: &NodeId,
        output: Option<&Path>,
        result: &RunResult,
    ) -> RunOutcome {
        let mut archived = Vec::new();
        if let Some(output) = output {
            match crate::archive::archive_canceled(&self.log_dir, id, output) {
                Ok(moved) => archived.extend(moved),
                Err(e) => {
                    self.record(
                        ctx,
                        AttemptOutcome::Canceled,
                        Invalidation::default(),
                        Reported::from(result),
                    );
                    let (path, source) = e.into_io();
                    return node_io(ctx, doing::ARCHIVE_CANCELED, path, source);
                }
            }
        }
        // A cancel invalidates only what it interrupted, so the set is this
        // node and nothing downstream re-derives.
        self.record(
            ctx,
            AttemptOutcome::Canceled,
            Invalidation {
                nodes: Vec::new(),
                archived,
            },
            Reported::from(result),
        );
        RunOutcome::Canceled {
            node: Some(id.clone()),
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

fn node_output(node: &Node, run_dir: &Path) -> Option<PathBuf> {
    match &node.kind {
        NodeKind::Agent { output, .. } => Some(run_dir.join(output)),
        NodeKind::Command { .. } => None,
    }
}

fn permission_of(node: &Node) -> crate::model::PermissionPolicy {
    match &node.kind {
        NodeKind::Agent { agent, .. } => agent.permission,
        // A command node launches no agent, so nothing can ask for
        // permission; the value is inert and never read.
        NodeKind::Command { .. } => crate::model::PermissionPolicy::Deny,
    }
}

impl Run<'_> {
    /// Turn a node's declared policy into the capability the runner gets.
    ///
    /// `Ask` without a port cannot be built, so it degrades to `Deny` —
    /// unreachable in practice because `validate_request` refuses such a
    /// run before the lock, and safe rather than silent if it ever were.
    fn permission_for(&self, node: &Node) -> crate::runner::Permission<'_> {
        self.permission_for_policy(permission_of(node))
    }

    /// The repair's `fix` runs as `flow.default_agent` and inherits its
    /// policy, so a flow whose defaults say `ask` asks during repair too.
    fn permission_for_fix(&self, agent: &AgentSpec) -> crate::runner::Permission<'_> {
        self.permission_for_policy(agent.permission)
    }

    /// A policy becomes a capability: `ask` is only one if this run has
    /// somewhere to ask. Validation refuses that combination up front, so
    /// reaching `Deny` here means a host built a request by hand.
    fn permission_for_policy(
        &self,
        policy: crate::model::PermissionPolicy,
    ) -> crate::runner::Permission<'_> {
        match policy {
            crate::model::PermissionPolicy::Deny => crate::runner::Permission::Deny,
            crate::model::PermissionPolicy::AllowOnce => crate::runner::Permission::AllowOnce,
            crate::model::PermissionPolicy::Ask => match self.ask.as_ref() {
                Some(channel) => crate::runner::Permission::Ask(channel),
                None => crate::runner::Permission::Deny,
            },
        }
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
mod policy;
mod prompt;
mod ready;
mod repair;
mod report;
mod run;

pub use report::{BudgetLimit, RunOutcome, RunReport};

pub use run::execute;

use policy::PolicyKind;
use ready::take_ready_batch;

#[cfg(test)]
mod tests;
