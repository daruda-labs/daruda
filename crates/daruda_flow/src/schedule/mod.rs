//! Runs a validated flow: topological order, one node at a time, judging
//! each before moving on, and driving each node's `on_fail` policy until it
//! passes or gives up. Serial by design — every node shares one working
//! directory, so two running at once would corrupt each other.

use crate::NodeId;
use crate::error::{FlowIoError, IoSite};
use crate::event::FlowEvent;
use crate::graph::FlowGraph;
use crate::model::{AgentSpec, Flow, Node, NodeKind, Prompt};
use crate::record::{AttemptOutcome, AttemptRecord, GitStatus, Invalidation};
use crate::request::Budget;
use crate::runner::{CancelToken, NodeFailure, NodeRunner, RunContext, RunResult};
use crate::template::{Surface, TemplateContext, render};
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
    /// Where the run narrates itself, or `None` when nobody is watching.
    events: Option<&'a smol::channel::Sender<FlowEvent>>,
    /// Where a `permission: ask` node puts its question. `None` is a host
    /// that cannot answer, which `validate_request` has already refused
    /// for any flow that could reach `Ask`.
    ask: Option<crate::runner::AskChannel>,
    /// What the run has spent and what it has to say about it. Owned as
    /// one value so the drive loop cannot reach a counter that only
    /// `account` may raise — see [`budget::Accounting`].
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

/// A read that failed, labelled and with the path it was given. Everything
/// a `FlowIoError` needs except the node and attempt, which the reader does
/// not have.
type ReadFailure = (&'static str, PathBuf, std::io::Error);

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
    } = inputs;
    let (flow, graph) = (loaded.flow(), loaded.graph());
    let run = Run {
        flow,
        graph,
        flow_dir,
        cwd,
        run_dir,
        log_dir: run_dir.join(LOG_DIR_NAME),
        node_outputs: flow
            .nodes
            .iter()
            .filter_map(|n| node_output(n, run_dir).map(|p| (n.id.clone(), p)))
            .collect(),
        runner,
        cancel,
        budget,
        git_status,
        events,
        ask: ask.cloned().map(crate::runner::AskChannel::new),
        spent: budget::Accounting::default(),
        executed: RefCell::new(Vec::new()),
        next_seq: Cell::new(1),
        driving: RefCell::new(HashSet::new()),
    };

    let mut outcome = RunOutcome::Done;
    for id in graph.topological_order() {
        if let Err(stopped) = run.drive(&id).await {
            outcome = stopped;
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
                let ctx = RunContext {
                    node_id: id,
                    attempt,
                    cwd: self.cwd,
                    run_dir: self.run_dir,
                    log_dir: &self.log_dir,
                    output: output.as_deref(),
                    evidence_seq: seq,
                    // The node's own budget, but never past the run's
                    // deadline: a runner races only what it is given, so a
                    // one-minute run ceiling would not stop a ten-minute
                    // node if the node's number came through unclipped.
                    timeout: self.bounded_timeout(node.timeout),
                    permission: self.permission_for(node),
                    cancel: self.cancel,
                };

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
                // Emitted where the session is about to be paid for, so the
                // stream's node events pair one-for-one with the runner calls
                // and with the record's attempts.
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
                    return Err(self.settle_cancel(
                        &ctx,
                        id,
                        output.as_deref(),
                        result.waiting.clone(),
                    ));
                }
                let artifacts = result.artifacts.clone();
                // Captured before `judge` consumes the result, like
                // `artifacts` — every `record` below is for this attempt.
                let waited = result.waiting.clone();

                let failure = match judge(node, &ctx, result) {
                    Ok(()) => {
                        // A pass archives nothing: its output stays live for
                        // the nodes downstream to read.
                        self.record(
                            &ctx,
                            AttemptOutcome::Passed,
                            Invalidation::default(),
                            waited,
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
                        waited,
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
                            waited.clone(),
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
                            waited.clone(),
                        );
                        return Err(node_io(&ctx, doing::ARCHIVE, e.path, e.source));
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
    fn record(
        &self,
        ctx: &RunContext<'_>,
        outcome: AttemptOutcome,
        invalidated: Invalidation,
        waited: crate::runner::Waiting,
    ) {
        let attempt = AttemptRecord {
            attempt: ctx.attempt,
            evidence_seq: ctx.evidence_seq,
            outcome,
            invalidated,
            git_status: self.git_status.and_then(|ask| ask()),
            waited,
        };
        self.spent
            .record(|records| crate::record::push_attempt(records, ctx.node_id, attempt));
    }

    fn take_seq(&self) -> u32 {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        seq
    }

    /// The text this attempt hands the runner: for a command the rendered
    /// `run`, for an agent the rendered prompt — plus, when `failure` is
    /// `Some` and the policy is `Retry`, a separator and the rendered hint.
    /// Fallible because `Prompt::File` and a file-backed hint are both read
    /// here; nothing downstream can read them. The error carries which of
    /// the two it was, so a missing hint is never reported as a missing
    /// prompt; the caller adds the node and attempt it has and this does
    /// not.
    fn node_text(
        &self,
        node: &Node,
        ctx: &RunContext<'_>,
        evidence: &[PathBuf],
        failure: Option<&NodeFailure>,
    ) -> Result<String, ReadFailure> {
        let tctx = TemplateContext {
            run_dir: self.run_dir,
            output: ctx.output,
            node_outputs: &self.node_outputs,
            failure,
            attempts: evidence,
        };
        match &node.kind {
            NodeKind::Command { run, .. } => Ok(render(run, &tctx, Surface::Shell)),
            NodeKind::Agent { prompt, .. } => {
                let mut text = render(
                    &self.read_prompt(prompt, doing::READ_PROMPT)?,
                    &tctx,
                    Surface::Prompt,
                );
                if let (Some(_), PolicyKind::Retry { hint }) = (failure, self.policy_of(node).kind)
                {
                    // Two channels, not one: the node's own prompt is
                    // unchanged and the hint answers the failure that made
                    // this attempt happen.
                    text.push_str("\n\n---\n");
                    text.push_str(&render(
                        &self.read_prompt(&hint, doing::READ_HINT)?,
                        &tctx,
                        Surface::Prompt,
                    ));
                }
                Ok(text)
            }
        }
    }

    /// `doing` is the caller's label because this reads both the node's
    /// prompt and its hint, and only the caller knows which one it asked for.
    fn read_prompt(&self, prompt: &Prompt, doing: &'static str) -> Result<String, ReadFailure> {
        match prompt {
            Prompt::Inline(text) => Ok(text.clone()),
            Prompt::File(path) => {
                let path = self.flow_dir.join(path);
                std::fs::read_to_string(&path).map_err(|e| (doing, path, e))
            }
        }
    }

    /// Dispatch finished text to the trait. Reads nothing.
    async fn call(&self, node: &Node, ctx: &RunContext<'_>, text: &str) -> RunResult {
        let result = match &node.kind {
            NodeKind::Agent { agent, .. } => self.runner.run_agent(ctx, agent, text).await,
            NodeKind::Command { .. } => self.runner.run_command(ctx, text).await,
        };
        self.account(&result);
        result
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
        waited: crate::runner::Waiting,
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
                        waited,
                    );
                    return node_io(ctx, doing::ARCHIVE_CANCELED, e.path, e.source);
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
            waited,
        );
        RunOutcome::Canceled {
            node: Some(id.clone()),
        }
    }

    /// One repair generation: run the `fix`, then re-derive what the gate's
    /// failure invalidated. `Err` ends the run — a failed fix changed
    /// nothing, so re-deriving would only re-prove the same verdict.
    async fn repair(
        &self,
        fix: &str,
        rerun: &[NodeId],
        ctx: &RunContext<'_>,
        gate: &NodeId,
        evidence: &[PathBuf],
        failure: &NodeFailure,
    ) -> Result<(), RunOutcome> {
        self.run_fix(fix, ctx, evidence, failure).await?;
        // Each member starts a fresh generation of its own — that is the
        // rule that gives a nested gate its cap back.
        let members = self.rerun_members(rerun, gate);
        // The computed set, not the declared roots: a host cannot infer the
        // closure, the `∩ executed` filter or the recursion guard. An empty
        // one still says work resumed.
        self.emit(FlowEvent::Rerunning {
            gate: gate.clone(),
            members: members.clone(),
        });
        for member in members {
            self.drive(&member).await?;
        }
        Ok(())
    }

    /// Run `fix` as `flow.default_agent` under `FIX_SESSION_ID`. `Err` when
    /// the fix itself fails — the set is then not re-derived, because
    /// nothing was changed.
    async fn run_fix(
        &self,
        fix: &str,
        ctx: &RunContext<'_>,
        evidence: &[PathBuf],
        failure: &NodeFailure,
    ) -> Result<(), RunOutcome> {
        // `crate::validate` rejects a repair without an agent, so reaching
        // this arm means that rule has a hole; reporting beats a panic.
        let Some(agent) = &self.flow.default_agent else {
            return Err(RunOutcome::Failed {
                node: ctx.node_id.clone(),
                failure: NodeFailure::SessionError(
                    "this flow names no agent for a repair's fix session".to_string(),
                ),
            });
        };

        let tctx = TemplateContext {
            run_dir: self.run_dir,
            output: None,
            node_outputs: &self.node_outputs,
            failure: Some(failure),
            attempts: evidence,
        };
        let text = render(fix, &tctx, Surface::Prompt);

        let fix_id = FIX_SESSION_ID.to_string();
        let fix_ctx = RunContext {
            node_id: &fix_id,
            attempt: ctx.attempt,
            cwd: self.cwd,
            run_dir: self.run_dir,
            log_dir: &self.log_dir,
            // The fix owes no file: it edits the tree, and the re-derived
            // nodes are what produce evidence of that.
            output: None,
            evidence_seq: self.take_seq(),
            // The gate's own timeout, by design (§6): a fix is a prompt
            // inside a policy rather than a node, so nothing else would
            // bound it and a hung fix session's only defence would be the
            // run's wall clock — which every other gate then has to share.
            //
            // The sharp edge that buys: a gate declared `timeout: 30s`
            // because it only runs `grep` gives its fix session 30s too.
            // An author wanting a longer repair raises the gate's.
            timeout: ctx.timeout,
            permission: self.permission_for_fix(agent),
            cancel: self.cancel,
        };
        // A fix is a real agent session and can take minutes. With no event
        // for it a host sits on `NodeFailed` and looks hung.
        self.emit(FlowEvent::FixStarted {
            gate: ctx.node_id.clone(),
        });
        // A fix is a session the run paid for, so it counts like any other.
        let result = self.runner.run_agent(&fix_ctx, agent, &text).await;
        self.account(&result);
        // The fix never reaches `drive_inner` or `judge`, so its fate is
        // sealed here instead.
        let recorded = if self.cancel.is_canceled() {
            AttemptOutcome::Canceled
        } else {
            match &result.outcome {
                Ok(()) => AttemptOutcome::Passed,
                Err(failure) => AttemptOutcome::Failed(failure.clone()),
            }
        };
        self.record(
            &fix_ctx,
            recorded,
            Invalidation::default(),
            result.waiting.clone(),
        );
        // Like any node: a cancel that interrupted the session is not a
        // failure of it, and reporting one would be wrong about why the run
        // stopped. The fix owes no output, so there is nothing to archive.
        if self.cancel.is_canceled() {
            // No `FixEnded`: a stop is not an ending the fix reached, and
            // `failure: None` would say it succeeded. `RunEnded` follows and
            // says why — the same rule an interrupted node follows.
            return Err(RunOutcome::Canceled { node: Some(fix_id) });
        }
        self.emit(FlowEvent::FixEnded {
            gate: ctx.node_id.clone(),
            failure: result.outcome.as_ref().err().cloned(),
        });
        match result.outcome {
            Ok(()) => Ok(()),
            Err(failure) => Err(RunOutcome::Failed {
                node: fix_id,
                failure,
            }),
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

/// A runner reporting success is necessary but not sufficient for an agent
/// node: the file contract is the scheduler's to enforce, so a turn that
/// ended cleanly without writing anything still fails.
fn judge(node: &Node, ctx: &RunContext<'_>, result: RunResult) -> Result<(), NodeFailure> {
    result.outcome?;
    if let (NodeKind::Agent { .. }, Some(path)) = (&node.kind, ctx.output) {
        let wrote_something = std::fs::metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        if !wrote_something {
            return Err(NodeFailure::NoOutput {
                expected: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

mod budget;
mod policy;
mod report;
mod run;

pub use report::{BudgetLimit, RunOutcome, RunReport};

pub use run::execute;

use policy::PolicyKind;

#[cfg(test)]
mod tests;
