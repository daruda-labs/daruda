//! An agent node: one fresh ACP session, one turn. The session is opened per
//! node and dropped when the turn ends, which is what ends the connection
//! task — so a node cannot inherit another node's context.

use crate::model::{AgentSpec, PermissionPolicy};
use crate::runner::{CANCELED, NodeFailure, NodeRunner, RunContext, RunResult, canceled, sleep};
use daruda_acp::{
    AcpEvent, AcpSessionHandle, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    ConfigValueView, LaunchSpec, ModeStateView, PermissionDecision, PermissionOption,
    PermissionOptionKind, UsageView, connect_agent_session,
};
use smol::stream::{Stream, StreamExt};
mod transcript;
use transcript::Transcript;

/// What a turn leaves behind besides its verdict: what the agent reported
/// spending, and what it said. Both are written from inside the stream
/// reader and read after it, so both are shared cells rather than borrows
/// threaded through every frame of the turn.
struct Recording<'a> {
    usage: &'a RefCell<Option<UsageView>>,
    log: &'a RefCell<Transcript>,
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The stream ended without a verdict — the adapter died mid-turn, so there
/// is nothing to attribute the stop to but the session itself.
const ENDED_EARLY: &str = "the session ended before the turn did";

/// Design §6: how long a cancelled turn has to end itself before the session
/// is dropped. The wait is what lets an adapter stop mid-write instead of
/// being killed with a half-written file on disk.
const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// How long a requested model or effort has to be confirmed. Its own budget
/// because an adapter that never answers must read as "could not apply", not
/// as the node's turn hanging until the node's whole budget is gone —
/// `daruda_acp` bounds its own handshake requests the same way.
const SETTINGS_BUDGET: Duration = Duration::from_secs(30);

/// Prepare whatever runtime `launch` needs, before any node opens a session.
/// Routed through the very call [`connect_agent_session`] makes, so whether a
/// runtime is needed at all is decided in one place; the per-node call then
/// finds the check already satisfied instead of downloading inside a node's
/// budget. The assembled command is discarded — only the preparation matters.
pub fn provision(launch: &LaunchSpec, node_install_dir: &Path) -> Result<(), String> {
    daruda_acp::launch_env::prepare_adapter_command(launch, node_install_dir, &mut |_| {})
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Runs agent nodes. Built with finished values — the catalog the host
/// resolved and the directory a managed runtime installs into — so the
/// runner never reads a `RunRequest`.
pub struct AcpRunner {
    agents: HashMap<String, LaunchSpec>,
    node_install_dir: PathBuf,
    grace: Duration,
    settings_budget: Duration,
}

impl AcpRunner {
    /// `agents` is the run's resolved catalog, keyed by the `agent.id` a node
    /// names; `node_install_dir` is where a managed Node.js runtime lands.
    pub fn new(agents: HashMap<String, LaunchSpec>, node_install_dir: PathBuf) -> Self {
        Self {
            agents,
            node_install_dir,
            grace: CANCEL_GRACE,
            settings_budget: SETTINGS_BUDGET,
        }
    }

    /// How long a cancelled turn is given to end itself before the session is
    /// dropped. Only a test has reason to shorten [`CANCEL_GRACE`].
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// How long a requested setting has to be confirmed. Only a test has
    /// reason to shorten [`SETTINGS_BUDGET`].
    pub fn with_settings_budget(mut self, budget: Duration) -> Self {
        self.settings_budget = budget;
        self
    }

    /// One node, one fresh session, one turn. Dropping the handle at the end
    /// ends the connection task, so nothing of this node survives into the
    /// next one.
    async fn one_turn(&self, ctx: &RunContext<'_>, agent: &AgentSpec, prompt: &str) -> RunResult {
        let Some(launch) = self.agents.get(&agent.id) else {
            return failed(format!("`{}` is not in this run's agent catalog", agent.id));
        };
        // A runtime download inside a node's turn would eat that node's
        // budget, so the run provisions ahead of the first node and this
        // callback only ever sees an already-satisfied check.
        let connected = connect_agent_session(
            launch.clone(),
            self.node_install_dir.clone(),
            ctx.cwd.to_path_buf(),
            agent.mode.clone().into_iter().collect(),
            None,
            None,
            &agent.id,
            &mut |_| {},
        );
        let (session, mut events) = match connected {
            Ok(session) => session,
            Err(e) => return failed(format!("could not open a session for `{}`: {e}", agent.id)),
        };
        self.harvest(&mut events, &session, ctx, agent, prompt)
            .await
    }

    /// Read the stream until the turn settles, or until the node's budget runs
    /// out or the run is stopped. Returning drops the handle, which ends the
    /// connection task — so the grace below is the adapter's only chance to
    /// stop mid-write rather than be killed with a half-written file on disk.
    async fn harvest(
        &self,
        events: &mut (impl Stream<Item = AcpEvent> + Unpin),
        session: &AcpSessionHandle,
        ctx: &RunContext<'_>,
        agent: &AgentSpec,
        prompt: &str,
    ) -> RunResult {
        let started = Instant::now();
        let usage = RefCell::new(None);
        let mut opened =
            Transcript::create(ctx.log_dir, ctx.node_id, ctx.attempt, ctx.evidence_seq);
        opened.prompt(prompt);
        let log = RefCell::new(opened);
        let rec = Recording {
            usage: &usage,
            log: &log,
        };

        // Scoped so the losing future releases the stream before the wind-down
        // below reads it again.
        let settled = {
            let turn = self.settle(events, session, ctx, agent, prompt, &rec);
            let stop = interrupted(ctx, started);
            smol::future::or(async { Ok(turn.await) }, async { Err(stop.await) }).await
        };

        let outcome = match settled {
            Ok(outcome) => outcome,
            Err(interrupt) => {
                session.cancel();
                // The turn's own verdict is discarded: it is a cancel's, and
                // the node reports why it was stopped instead.
                let winding_down = drain(events, session, ctx.permission, &rec);
                smol::future::or(async { _ = winding_down.await }, sleep(self.grace)).await;
                Err(interrupt.into_failure())
            }
        };
        let mut log = log.into_inner();
        log.ended(&match &outcome {
            Ok(()) => "passed".to_string(),
            Err(failure) => failure.to_string(),
        });
        RunResult {
            outcome,
            artifacts: log.artifacts(),
            usage: usage.into_inner(),
        }
    }

    /// Bring the session to what the node asked for, prompt, and read the turn
    /// out. Settings go first and are waited on: a confirmation that arrives
    /// after the prompt leaves the first turn running on the adapter's
    /// defaults while the record claims otherwise.
    async fn settle(
        &self,
        events: &mut (impl Stream<Item = AcpEvent> + Unpin),
        session: &AcpSessionHandle,
        ctx: &RunContext<'_>,
        agent: &AgentSpec,
        prompt: &str,
        rec: &Recording<'_>,
    ) -> Result<(), NodeFailure> {
        let connected = announced(events, rec.usage, Announcement::Connected).await?;
        // Before the other two, because a mode that silently degraded is the
        // one axis that changes what the agent is *allowed* to do.
        check_mode(agent, connected.modes.as_ref())?;
        self.apply(events, session, connected.options, agent, rec.usage)
            .await?;
        session.send_prompt(prompt.to_string());
        drain(events, session, ctx.permission, rec).await
    }

    /// Apply each axis the node pinned, in order, and confirm each before
    /// moving on. Every failure here is [`NodeFailure::UnsupportedSetting`]:
    /// not advertised, not among the offered values, or requested and not
    /// applied all leave the node running as something other than its record.
    async fn apply(
        &self,
        events: &mut (impl Stream<Item = AcpEvent> + Unpin),
        session: &AcpSessionHandle,
        mut options: Vec<ConfigOptionView>,
        agent: &AgentSpec,
        usage: &RefCell<Option<UsageView>>,
    ) -> Result<(), NodeFailure> {
        for want in requested(agent) {
            let unsupported = |available: Vec<String>| NodeFailure::UnsupportedSetting {
                field: want.field,
                value: want.value.to_string(),
                available,
            };
            let Some(offer) = advertised(&options, want.category) else {
                return Err(unsupported(Vec::new()));
            };
            if !offer.choices.iter().any(|c| c == want.value) {
                return Err(unsupported(offer.choices));
            }
            session.set_config_option(offer.id, ConfigValueView::Id(want.value.to_string()));

            // The reply carries the agent's whole option set, so it replaces
            // what the next axis reads: an adapter that rebuilds its effort
            // list per model only offers the right one after the model lands.
            //
            // The budget is the backstop, not the mechanism: an adapter that
            // refuses says so, and waiting out 30s to conclude what it
            // already told us is the whole of this node's remaining time
            // spent learning nothing.
            let confirmed = smol::future::or(
                async { Some(announced(events, usage, Announcement::OptionsChanged).await) },
                async {
                    sleep(self.settings_budget).await;
                    None
                },
            )
            .await;
            options = match confirmed {
                Some(announced) => announced?.options,
                None => return Err(unsupported(offer.choices)),
            };
            match advertised(&options, want.category) {
                Some(now) if now.current == want.value => {}
                Some(now) => return Err(unsupported(now.choices)),
                None => return Err(unsupported(Vec::new())),
            }
        }
        Ok(())
    }
}

/// One axis a node pinned, and the category the adapter advertises it under.
/// `mode` is absent: it travels as `initial_modes` at connect time, and
/// `daruda_acp` strips the mode option out of every set the host sees.
struct Requested<'a> {
    field: &'static str,
    category: ConfigOptionCategoryView,
    value: &'a str,
}

/// A node that pins a mode must actually be in it. `daruda_acp` degrades an
/// unavailable or rejected mode to a fallback and only emits a `Notice`
/// (`session.rs`'s connect path), so a flow claiming `bypassPermissions`
/// can otherwise run in `auto` with nothing disagreeing.
///
/// Checked ahead of model and effort because this is the axis that decides
/// what the agent is allowed to do, not merely how well it does it.
fn check_mode(agent: &AgentSpec, modes: Option<&ModeStateView>) -> Result<(), NodeFailure> {
    let Some(want) = agent.mode.as_deref() else {
        return Ok(());
    };
    let unsupported = |available: Vec<String>| NodeFailure::UnsupportedSetting {
        field: "mode",
        value: want.to_string(),
        available,
    };
    // No mode state at all is an agent that advertises none — it cannot be
    // in the requested one.
    let Some(state) = modes else {
        return Err(unsupported(Vec::new()));
    };
    if state.current == want {
        return Ok(());
    }
    Err(unsupported(
        state.available.iter().map(|m| m.id.clone()).collect(),
    ))
}

/// What one announcement carried. `modes` only ever arrives with
/// `Connected` — `ConfigOptionsChanged` replaces options, not modes.
struct Announced {
    options: Vec<ConfigOptionView>,
    modes: Option<ModeStateView>,
}

/// What the node asked for, in the order it is applied.
fn requested(agent: &AgentSpec) -> Vec<Requested<'_>> {
    [
        (
            "model",
            ConfigOptionCategoryView::Model,
            agent.model.as_deref(),
        ),
        (
            "effort",
            ConfigOptionCategoryView::ThoughtLevel,
            agent.effort.as_deref(),
        ),
    ]
    .into_iter()
    .filter_map(|(field, category, value)| {
        Some(Requested {
            field,
            category,
            value: value?,
        })
    })
    .collect()
}

/// What the adapter offers on one axis. Owned because the option set it was
/// read from is replaced by the agent's next reply.
struct Advertised {
    id: String,
    current: String,
    choices: Vec<String>,
}

/// The selectable option advertised for `category`. A boolean where a choice
/// of named values belongs is as unusable as a missing option, so both read as
/// "not advertised" rather than as something to coerce.
fn advertised(
    options: &[ConfigOptionView],
    category: ConfigOptionCategoryView,
) -> Option<Advertised> {
    let option = options.iter().find(|o| o.category == category)?;
    let ConfigOptionKindView::Select {
        current_value,
        options: choices,
    } = &option.kind
    else {
        return None;
    };
    Some(Advertised {
        id: option.id.clone(),
        current: current_value.clone(),
        choices: choices.iter().map(|c| c.value.clone()).collect(),
    })
}

/// Which announcement the runner is waiting on before it can go further.
#[derive(Clone, Copy, PartialEq)]
enum Announcement {
    /// The session is up, and the adapter has said what it offers.
    Connected,
    /// The agent re-advertised its options — the only confirmation a
    /// `set_config_option` gets, and a full replacement of the set.
    OptionsChanged,
}

/// Read events until the agent announces `wanted`, carrying its option set
/// out. Everything else describes a conversation nobody is here to watch —
/// except usage, the run's only cost meter, and a terminal error.
async fn announced(
    events: &mut (impl Stream<Item = AcpEvent> + Unpin),
    usage: &RefCell<Option<UsageView>>,
    wanted: Announcement,
) -> Result<Announced, NodeFailure> {
    loop {
        let Some(event) = events.next().await else {
            return Err(NodeFailure::SessionError(ENDED_EARLY.to_string()));
        };
        match event {
            AcpEvent::Connected {
                config_options,
                modes,
                ..
            } if wanted == Announcement::Connected => {
                return Ok(Announced {
                    options: config_options,
                    modes,
                });
            }
            AcpEvent::ConfigOptionsChanged(options) if wanted == Announcement::OptionsChanged => {
                return Ok(Announced {
                    options,
                    modes: None,
                });
            }
            AcpEvent::UsageChanged(reported) => *usage.borrow_mut() = Some(reported),
            // The adapter said no. Waiting for a confirmation it has
            // already refused to send is the whole of the settings budget
            // spent learning what this event just said.
            AcpEvent::ConfigOptionRejected { config_id, reason }
                if wanted == Announcement::OptionsChanged =>
            {
                return Err(NodeFailure::SettingRejected { config_id, reason });
            }
            AcpEvent::Error(message) => return Err(NodeFailure::SessionError(message)),
            _ => {}
        }
    }
}

/// Why the harvest stopped early. The procedure is identical — cancel the
/// turn, wait out the grace, drop the session — so the two differ only in
/// what the node reports.
enum Interrupt {
    Timeout { elapsed: Duration },
    Canceled,
}

impl Interrupt {
    fn into_failure(self) -> NodeFailure {
        match self {
            Interrupt::Timeout { elapsed } => NodeFailure::Timeout { elapsed },
            Interrupt::Canceled => NodeFailure::SessionError(CANCELED.to_string()),
        }
    }
}

/// The node's budget and the run's stop switch, whichever comes first.
async fn interrupted(ctx: &RunContext<'_>, started: Instant) -> Interrupt {
    let timeout = ctx.timeout;
    smol::future::or(
        async move {
            sleep(timeout).await;
            Interrupt::Timeout {
                elapsed: started.elapsed(),
            }
        },
        async move {
            canceled(ctx.cancel).await;
            Interrupt::Canceled
        },
    )
    .await
}

/// Read events until the turn settles. Every other event describes a
/// conversation, which is the UI's business and not the engine's — except
/// usage, the only cumulative cost the run's budget can see, and permission,
/// which nobody is here to answer.
async fn drain(
    events: &mut (impl Stream<Item = AcpEvent> + Unpin),
    session: &AcpSessionHandle,
    permission: PermissionPolicy,
    rec: &Recording<'_>,
) -> Result<(), NodeFailure> {
    loop {
        let Some(event) = events.next().await else {
            return Err(NodeFailure::SessionError(ENDED_EARLY.to_string()));
        };
        match event {
            AcpEvent::UsageChanged(reported) => *rec.usage.borrow_mut() = Some(reported),
            AcpEvent::PermissionRequested { id, request } => {
                // Answered before anything else, so a policy that lets the
                // turn continue never leaves the agent parked on a request.
                session.respond_permission(id, decide(permission, &request.options));
                // Under `Deny` the request arriving at all means the mode
                // this node was launched in was wrong. Refusing and letting
                // the turn run on would let the agent work around it and
                // pass — `model.rs` chose failing loud over that. The session
                // ends here, so whether the refusal reaches the wire is moot.
                if permission == PermissionPolicy::Deny {
                    return Err(NodeFailure::PermissionDenied {
                        tool: request
                            .tool_call
                            .fields
                            .title
                            .clone()
                            .unwrap_or_else(|| request.tool_call.tool_call_id.0.to_string()),
                    });
                }
            }
            AcpEvent::TurnEnded { stop_reason, .. } => {
                return failure_for(&stop_reason).map_or(Ok(()), Err);
            }
            // The session survives this one, but the node's attempt does not.
            AcpEvent::TurnFailed(message) => return Err(NodeFailure::TurnFailed(message)),
            AcpEvent::Error(message) => return Err(NodeFailure::SessionError(message)),
            // What the agent actually said. Recorded here rather than in
            // the caller because this is the only place the stream is read.
            AcpEvent::Update(update) => rec.log.borrow_mut().update(&update),
            _ => {}
        }
    }
}

/// The node's policy is the whole answer — a run nobody is watching has no
/// one to ask. `AllowAlways` is never selected: it outlives the session, and
/// one approval must not become standing policy. An option of the wanted kind
/// is the only one selectable, so an agent that offers none gets a cancel
/// rather than a choice made on its behalf.
fn decide(policy: PermissionPolicy, options: &[PermissionOption]) -> PermissionDecision {
    let wanted = match policy {
        PermissionPolicy::Deny => PermissionOptionKind::RejectOnce,
        PermissionPolicy::AllowOnce => PermissionOptionKind::AllowOnce,
    };
    let Some(option) = options.iter().find(|option| option.kind == wanted) else {
        return PermissionDecision::Cancelled;
    };
    let option_id = option.option_id.0.to_string();
    match policy {
        PermissionPolicy::Deny => PermissionDecision::Reject { option_id },
        PermissionPolicy::AllowOnce => PermissionDecision::Allow { option_id },
    }
}

/// `stop_reason` arrives as a Debug-formatted string, so this is the one
/// place in the crate that reads it — a second `match` on the same text would
/// drift the moment an adapter's wording changes.
///
/// `daruda_acp` reports everything but a cancel as a normal completion, which
/// would pass a node that exhausted its context after writing half its
/// output. Only `EndTurn` passes here, and an unrecognized reason fails:
/// stopping loudly beats letting a node through on a reason nobody has read.
fn failure_for(stop_reason: &str) -> Option<NodeFailure> {
    match stop_reason {
        "EndTurn" => None,
        "MaxTokens" => Some(NodeFailure::ContextExhausted),
        "MaxTurnRequests" => Some(NodeFailure::TurnLimit),
        "Refusal" => Some(NodeFailure::Refused),
        other => Some(NodeFailure::TurnFailed(format!(
            "the agent stopped for an unrecognized reason: {other}"
        ))),
    }
}

/// A failure of the session rather than of the turn: nothing ran, so there is
/// no stop reason to report and no usage to carry.
fn failed(message: String) -> RunResult {
    RunResult {
        outcome: Err(NodeFailure::SessionError(message)),
        artifacts: Vec::new(),
        usage: None,
    }
}

impl NodeRunner for AcpRunner {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a AgentSpec,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        Box::pin(async move { self.one_turn(ctx, agent, prompt).await })
    }

    fn run_command<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            failed("command nodes are not this runner's — a process runner takes them".to_string())
        })
    }
}

#[cfg(test)]
mod tests;
