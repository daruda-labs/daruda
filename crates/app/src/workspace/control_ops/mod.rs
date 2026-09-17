//! `impl Workspace` for the external control surface.
//!
//! Two shapes only: a read-only snapshot the app-level dispatcher aggregates
//! across windows, and targeted actions that delegate to the existing agent
//! chat ops. Nothing here is a new mutation site — every write goes through a
//! method that already owned it.

use gpui::{App, Context};

use crate::control::agent_text::{bound_agent_text, sanitize_title};
use crate::control::result::{
    Activity, ChatSummary, ControlError, Health, LaneEntry, LaneHandle, SendDisposition,
    StopDisposition,
};
use crate::telegram::bridge::PaneRef;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::{
    ActivityState, AgentChatView, AgentSessionStatus, PromptDispatch,
};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_store::project::{LaneRef, ProjectId};

/// What an agent says about a `/name` daruda does not own.
///
/// Three-valued because the two questions asked of it want different things
/// from the middle. "The session has not advertised yet" cannot claim the
/// name, but it cannot rule it out either, and folding that into either
/// answer makes one of the callers wrong: as a claim, one cold pane vetoes
/// every name for the whole window — and after a restart every pane but the
/// focused one is cold, since a restored chat stays `Idle` until first focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashClaim {
    /// The agent advertises the name. It is not ours to answer.
    Claims,
    /// The agent has advertised its list and the name is not on it.
    Disclaims,
    /// Nobody has said — cold, connecting, or no agent at all. An abstention:
    /// it never outvotes an agent that did answer, and on its own it earns no
    /// conclusion in either direction.
    Unsaid,
}

impl SlashClaim {
    /// Fold two answers. A claim anywhere settles it; failing that a
    /// disclaimer counts; silence yields to either.
    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Claims, _) | (_, Self::Claims) => Self::Claims,
            (Self::Disclaims, _) | (_, Self::Disclaims) => Self::Disclaims,
            (Self::Unsaid, Self::Unsaid) => Self::Unsaid,
        }
    }

    /// Whether this answer earns the typo suggestion: some agent positively
    /// ruled the name out, and none claimed it. Silence alone never does —
    /// the honest answer with no evidence is the missing target.
    pub(crate) fn rules_out(self) -> bool {
        self == Self::Disclaims
    }
}

#[cfg(test)]
mod slash_claim_tests {
    use super::SlashClaim::{self, Claims, Disclaims, Unsaid};

    /// `merge` is folded over panes and again over windows, so the answer must
    /// not depend on how the panes were grouped or ordered, and an empty fold
    /// must come out `Unsaid`. Associative, commutative, idempotent, with
    /// `Unsaid` as the identity — checked exhaustively, since there are nine
    /// pairs.
    #[test]
    fn merge_is_a_semilattice_with_unsaid_as_identity() {
        let all = [Claims, Disclaims, Unsaid];
        for a in all {
            assert_eq!(a.merge(Unsaid), a, "identity");
            assert_eq!(a.merge(a), a, "idempotent");
            for b in all {
                assert_eq!(a.merge(b), b.merge(a), "commutative");
                for c in all {
                    assert_eq!(
                        a.merge(b).merge(c),
                        a.merge(b.merge(c)),
                        "associative: grouping must not change the answer"
                    );
                }
            }
        }
        // Precedence, stated once rather than inferred from the fold.
        assert_eq!(Claims.merge(Disclaims), Claims, "a claim outranks a denial");
        assert_eq!(
            Disclaims.merge(Unsaid),
            Disclaims,
            "an answer outranks silence"
        );
    }

    /// Only a positive denial earns the suggestion.
    #[test]
    fn only_disclaims_rules_out() {
        assert!(SlashClaim::Disclaims.rules_out());
        assert!(!SlashClaim::Claims.rules_out());
        assert!(!SlashClaim::Unsaid.rules_out());
    }
}

/// What a newly opened chat is called, for the two surfaces that have to name
/// it: the phone ping and the tool result.
pub(crate) struct ChatLabel {
    pub(crate) path: String,
    /// The agent the pane actually resolved to — not what the caller asked
    /// for. A request naming an id the catalog does not hold opens the
    /// default agent, and reporting the request back would hide that.
    pub(crate) agent: String,
    pub(crate) agent_name: String,
}

impl Workspace {
    /// Every agent-chat pane in this window, paired with the lane it lives in.
    ///
    /// Read-only on purpose: the caller runs this inside
    /// `WindowRegistry::for_each_workspace`, which is already inside a
    /// `Workspace::update`. Mutating an entity there, or reading one twice,
    /// is the re-entrancy panic in CLAUDE.md pitfall 5.
    pub(crate) fn control_snapshot(&self, cx: &App) -> Vec<(LaneRef, ChatSummary)> {
        let active = self.active;
        let uuid = self.uuid();
        self.lane_agent_chats()
            .filter_map(|(pane_id, view)| {
                let lane_ref = self.lane_ref_for_pane(pane_id)?;
                let unread = self.lane_for(lane_ref).is_some_and(|l| l.is_unread);
                let v = view.read(cx);
                Some((
                    lane_ref,
                    ChatSummary {
                        target: PaneRef {
                            workspace: uuid,
                            pane: pane_id,
                        },
                        agent: v.agent_id.clone(),
                        agent_name: v.agent_name.clone(),
                        is_active_lane: lane_ref == active,
                        activity: map_activity(v.activity_state()),
                        health: map_health(&v.status),
                        unread,
                        title: v.activity_title().and_then(sanitize_title),
                        last_activity: last_activity_unix(v),
                    },
                ))
            })
            .collect()
    }

    /// This project's display name, or the empty string when the id no longer
    /// resolves — a listing row is grouped by name, so a missing project must
    /// not drop the row it owns.
    pub(crate) fn control_project_name(&self, id: ProjectId) -> String {
        self.project_for(id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    /// This lane's display name. Same fallback reasoning as
    /// [`Self::control_project_name`].
    pub(crate) fn control_lane_name(&self, target: LaneRef) -> String {
        self.lane_for(target)
            .map(|l| l.display_name())
            .unwrap_or_default()
    }

    /// How a chat the control surface just opened is named: its worktree path,
    /// the agent id it actually resolved to, and that agent's display name.
    /// Both halves of the agent, because they answer different questions — the
    /// id is what a caller passes back, the name is what a person reads — and
    /// both are workspace-private, so this is the one way out. `None` when the
    /// pane is gone.
    pub(crate) fn control_chat_label(&self, pane: PaneId, cx: &App) -> Option<ChatLabel> {
        let lane = self.lane_ref_for_pane(pane)?;
        let view = self.agent_chat_view(pane)?.read(cx);
        Some(ChatLabel {
            path: crate::surface::strings::control_lane_path(
                &self.control_project_name(lane.project),
                &self.control_lane_name(lane),
            ),
            agent: view.agent_id.clone(),
            agent_name: view.agent_name.clone(),
        })
    }

    /// This lane's position in its project's tab strip — the listing's sort
    /// key, so ordinals follow the order the left dock shows.
    pub(crate) fn control_lane_tab_order(&self, target: LaneRef) -> u32 {
        self.lane_for(target).map(|l| l.tab_order).unwrap_or(0)
    }

    /// Every worktree in this window, grouped by project and then in tab
    /// order within it.
    ///
    /// Deliberately not derived from [`Self::control_snapshot`]: that one is
    /// chat-scoped, so a worktree with no agent chat in it — which is exactly
    /// the one a caller wants to open a chat in — has no row there.
    pub(crate) fn control_lane_list(&self) -> Vec<LaneEntry> {
        let active = self.active;
        let uuid = self.uuid();
        let mut lanes: Vec<(u32, LaneEntry)> = self
            .projects
            .iter()
            .flat_map(|project| {
                project.lanes.iter().map(move |lane| {
                    let target = LaneRef {
                        project: project.id,
                        lane: lane.id,
                    };
                    (
                        lane.tab_order,
                        LaneEntry {
                            target: LaneHandle::new(uuid, target),
                            project: project.name.clone(),
                            name: lane.display_name(),
                            is_active: target == active,
                            chats: self.control_chat_count(target),
                        },
                    )
                })
            })
            .collect();
        // Name first so the listing reads alphabetically, then id so two
        // projects sharing a basename stay apart instead of interleaving
        // their worktrees — the same key `control::exec::collect_rows` sorts
        // its chat rows by, for the same reason.
        lanes.sort_by(|a, b| {
            a.1.project
                .cmp(&b.1.project)
                .then_with(|| a.1.target.project.cmp(&b.1.target.project))
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.target.lane.cmp(&b.1.target.lane))
        });
        lanes.into_iter().map(|(_, entry)| entry).collect()
    }

    /// How many agent-chat panes one worktree holds. A worktree that has never
    /// been activated has no runtime entry, which is zero rather than missing.
    fn control_chat_count(&self, target: LaneRef) -> u32 {
        self.main_area
            .runtimes
            .get(&target)
            .map_or(0, |rt| {
                rt.panes
                    .iter()
                    .filter(|p| !self.is_orchestrator_pane(p.id) && p.agent_chat_view().is_some())
                    .count()
            })
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// The worktree this window is currently showing. Test-only: a control
    /// command always names its target, so nothing in production asks.
    #[cfg(test)]
    pub(crate) fn control_active_lane(&self) -> LaneRef {
        self.active
    }

    /// Whether this window holds `target`. The control surface's question:
    /// its commands name a worktree that may live in any window, so the
    /// dispatcher asks each one rather than reaching into `lane_for`.
    pub(crate) fn control_has_lane(&self, target: LaneRef) -> bool {
        self.lane_for(target).is_some()
    }

    /// Whether this window is showing a worktree at all.
    ///
    /// A predicate, not a getter: the active worktree is resolved to a concrete
    /// handle in exactly one place — the adapter boundary, where
    /// `control_active_lane_offers` mints one — and everything downstream
    /// names its target. This exists only to keep two refusals apart while
    /// that resolution runs: nowhere to run is a different answer than no such
    /// flow.
    pub(crate) fn control_has_active_lane(&self) -> bool {
        self.lane_for(self.active).is_some()
    }

    /// Same, for a project.
    pub(crate) fn control_has_project(&self, project: ProjectId) -> bool {
        self.project_for(project).is_some()
    }

    /// Open another agent chat in an existing worktree.
    ///
    /// Activates the worktree first, because that is the only way daruda opens
    /// a pane: the runtime a pane is inserted into is the *active* one, and
    /// `finalize_create_lane` does the same for the pane it makes. So this
    /// changes what is on screen — an agent's tool call moves the user's view.
    pub(crate) fn control_chat_new(
        &mut self,
        target: LaneRef,
        agent: Option<String>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneRef, ControlError> {
        let pane = self.control_insert_chat(target, agent, window, cx)?;
        // Revealing focuses the pane, which starts the session — so it is the
        // last step, the same split `open_agent_chat_pane_with_agent` uses.
        self.reveal_new_agent_chat_pane(pane, window, cx);
        Ok(PaneRef {
            workspace: self.uuid(),
            pane,
        })
    }

    /// Everything [`Self::control_chat_new`] does except revealing the pane.
    ///
    /// Split out because revealing spawns a real ACP adapter, whose task
    /// outlives a test and then trips gpui's determinism assert in whichever
    /// test runs next — so a test can assert placement without a session.
    fn control_insert_chat(
        &mut self,
        target: LaneRef,
        agent: Option<String>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, ControlError> {
        // Both checks *before* activating: revealing a worktree and then
        // refusing to open a pane in it leaves the user staring at an empty
        // state for a tool call that failed.
        let Some(lane) = self.lane_for(target) else {
            return Err(ControlError::TargetGone);
        };
        if lane.availability != crate::lane::availability::LaneAvailability::Present {
            return Err(ControlError::TargetGone);
        }
        self.activate_lane(target, window, cx);
        let agent_id =
            crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id(
                &self.agents,
                agent.as_deref().or(self.last_agent_id.as_deref()),
            );
        let cwds = self.active_lane_cwds();
        self.insert_agent_chat_pane(agent_id, cwds, window, cx)
            .ok_or(ControlError::TargetGone)
    }

    /// The insert half of [`Self::control_chat_new`], for a test that must not
    /// start a session.
    #[cfg(test)]
    pub(crate) fn control_insert_chat_for_test(
        &mut self,
        target: LaneRef,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, ControlError> {
        self.control_insert_chat(target, None, window, cx)
    }

    /// The worktree-creation plan `git worktree add` needs, derived from a
    /// tool call's arguments.
    ///
    /// Separate from [`Self::control_lane_finalize`] because the step between
    /// them is blocking git I/O that has to run off the UI thread — the same
    /// shape the task workflow already uses.
    pub(in crate::workspace) fn control_lane_plan(
        &self,
        project: ProjectId,
        name: &str,
        base_ref: Option<String>,
    ) -> Result<crate::workspace::lane_ops::CreateWorktreePlan, ControlError> {
        // Validated here, before anything is spawned and before the approval
        // card quotes it: an agent-supplied name reaches both `git worktree
        // add -b` and a filesystem path, and the create form has always run it
        // through the same rules (`create_modal`'s `sanitize_branch_name`).
        // Letting git reject it instead would cost the user a tap and then
        // blame them for a mistake they did not make.
        let branch =
            daruda_core::git::sanitize_branch_name(name).ok_or(ControlError::LaneNameInvalid)?;
        let Some(repo_root) = self.project_for(project).map(|p| p.root.clone()) else {
            return Err(ControlError::TargetGone);
        };
        Ok(crate::workspace::lane_ops::CreateWorktreePlan {
            new_path: crate::workspace::lane_ops::lane_checkout_path(&repo_root, &branch),
            branch,
            repo_root,
            base_ref: self.resolve_lane_base_ref_for(project, base_ref),
            description: None,
            // No host picker on this path, so the lane stays at `Lane::git`'s
            // unanswered/Local default, exactly like a task-created one.
            session_host: None,
        })
    }

    /// Register a worktree that is already on disk, and report both handles.
    ///
    /// Delegates to `finalize_create_lane`, which owns branch bookkeeping and
    /// the initial pane, so nothing here becomes a second way to make a lane.
    pub(in crate::workspace) fn control_lane_finalize(
        &mut self,
        plan: crate::workspace::lane_ops::CreateWorktreePlan,
        project: ProjectId,
        agent: Option<String>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<(LaneRef, PaneRef), ControlError> {
        let pane = self
            .finalize_create_lane(
                plan,
                project,
                daruda_store::tasks::TaskAgentSurface::AgentChat,
                agent.as_deref(),
                window,
                cx,
            )
            // The checkout is on disk and only the bookkeeping failed, which
            // is not something a caller can fix by asking differently — so
            // the detail says that rather than forwarding the toast's
            // localized wording onto a machine payload.
            .map_err(|_| ControlError::LaneCreateFailed {
                detail: "the worktree was created but daruda could not register it".to_owned(),
            })?;
        Ok((
            self.active,
            PaneRef {
                workspace: self.uuid(),
                pane,
            },
        ))
    }

    /// How many prompts one pane has waiting. `None` when the pane is gone,
    /// which is a different refusal from a full queue.
    ///
    /// Counts both queues: a paused prompt still has to drain before a new
    /// one does, so leaving it out would let the cap be bypassed by stopping.
    pub(crate) fn agent_chat_queue_depth(&self, pane: PaneId, cx: &App) -> Option<usize> {
        Some(self.agent_chat_view(pane)?.read(cx).queued_prompt_count())
    }

    /// Fill a pane's queue to `depth`, for the guard's test.
    #[cfg(test)]
    pub(crate) fn fill_prompt_queue_for_test(
        &mut self,
        pane: PaneId,
        depth: usize,
        cx: &mut Context<Self>,
    ) {
        let view = self.agent_chat_view(pane).expect("pane").clone();
        view.update(cx, |v, _| v.fill_queue_for_test(depth));
    }

    /// Send a prompt to one pane. Delegates to the existing Telegram-origin
    /// prompt path so slash classification stays on its single funnel; only
    /// the disposition is surfaced instead of relayed.
    ///
    /// The `None` arm is not a connection failure. Having already excluded a
    /// missing pane, the only way that funnel declines to dispatch is a local
    /// slash command it handled itself — today `/clear`, which *resets the
    /// session*. Reporting that as "sent" would hide a destroyed transcript,
    /// so it gets its own disposition.
    pub(crate) fn control_say(
        &mut self,
        pane: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) -> Result<SendDisposition, ControlError> {
        if self.agent_chat_view(pane).is_none() {
            return Err(ControlError::TargetGone);
        }
        match self.send_agent_prompt_text_from_telegram(pane, text, cx) {
            Some(PromptDispatch::SentNow) => Ok(SendDisposition::Delivered),
            Some(PromptDispatch::Queued) => Ok(SendDisposition::Queued),
            Some(PromptDispatch::QueueFull) => Err(ControlError::QueueFull),
            None => Ok(SendDisposition::HandledLocally),
        }
    }

    /// What this pane's agent last said, bounded for a tool result.
    ///
    /// The *last* completed assistant text, not the whole transcript: the
    /// caller asked what came of a prompt, and an agent's closing message is
    /// that answer. Reading further back would hand over other turns' text
    /// nobody asked for.
    ///
    /// A still-streaming message does not count as said. This surface can be
    /// called mid-turn, so it needs the filter that
    /// `telegram_completion_parts` does not: that one runs only at the
    /// completion tee, where nothing is in flight. Reporting half a sentence
    /// as the agent's answer is the worse of the two failures — a caller
    /// wanting the answer to its *own* prompt has `daruda_chat_ask`, and one
    /// checking progress has `activity`.
    pub(crate) fn control_read(
        &self,
        pane: PaneId,
        cx: &App,
    ) -> Result<Option<String>, ControlError> {
        let Some(view) = self.agent_chat_view(pane) else {
            return Err(ControlError::TargetGone);
        };
        Ok(last_assistant_text(view.read(cx)).and_then(|t| bound_agent_text(&t)))
    }

    /// Whether a `/name` from outside should go to this pane's agent.
    ///
    /// The agent advertises its own slash commands over ACP, and the in-app
    /// completion menu already offers them from this same list — so a phone
    /// answering "unknown command" for one the menu would happily complete is
    /// the surface disagreeing with itself.
    ///
    /// A pane that has not advertised yet forwards too — [`SlashClaim::Unsaid`]
    /// is not "it does not have that command", and a name daruda does not own
    /// is far likelier the agent's than a typo of one of ours. So does a pane
    /// that is not there, so the answer comes from the delivery attempt, which
    /// can say `target_gone`, rather than from a guess here.
    ///
    /// Asked about a *named* pane, so it answers about that pane whatever it
    /// is — including the orchestrator's, which can be `last_pinged` and so
    /// can be the target of a reply. [`Self::rules_out_slash_command`] asks
    /// about the population instead and scopes itself accordingly.
    pub(crate) fn agent_takes_slash_command(&self, pane: PaneId, name: &str, cx: &App) -> bool {
        match self.agent_chat_view(pane) {
            Some(view) => pane_slash_claim(view.read(cx), name) != SlashClaim::Disclaims,
            None => true,
        }
    }

    /// The agent chat a phone message falls back to when nothing names a
    /// target: the active lane's focused pane when that pane is a chat,
    /// otherwise that lane's only chat.
    ///
    /// Deliberately narrow. Two chats with focus on neither is ambiguous, and
    /// guessing there would start a turn in the wrong agent — worse than
    /// telling the sender to pick. The orchestrator is excluded for the same
    /// reason `/list` excludes it: the phone is never shown it, so it must not
    /// be reached by accident either.
    pub(crate) fn fallback_agent_chat(&self) -> Option<PaneRef> {
        let runtime = self.main_area.runtimes.get(&self.active)?;
        let mut chats = runtime
            .panes
            .iter()
            .filter(|p| p.agent_chat_view().is_some() && !self.is_orchestrator_pane(p.id));
        let focused = runtime.focused_pane_id;
        let pane = match chats.clone().find(|p| p.id == focused) {
            Some(p) => p.id,
            None => {
                let only = chats.next()?;
                chats.next().is_none().then_some(only.id)?
            }
        };
        Some(PaneRef {
            workspace: self.uuid(),
            pane,
        })
    }

    /// What this window's agents say about `/name` — asked when nothing names
    /// a target, so there is no one pane to ask.
    ///
    /// Scoped to [`Self::lane_agent_chats`]: every lane's chats, minus the
    /// orchestrator. The orchestrator exclusion is the part
    /// [`Self::fallback_agent_chat`] shares — an unowned slash can only ever
    /// be delivered to a pane `/list` offers, so a chat the phone is never
    /// shown must not decide the answer, and being usually cold and hidden it
    /// would decide every one.
    ///
    /// The lane scope deliberately does *not* match: a fallback target must
    /// come from the lane the user is in, but a vocabulary answer is about
    /// what the agents know, and `/list` offers every lane's chat. A
    /// background lane that claims the name suppresses the typo answer, which
    /// is the conservative direction.
    ///
    /// Folded rather than reduced to a bool because silence has to stay
    /// distinguishable: see [`SlashClaim`].
    pub(crate) fn slash_claim(&self, name: &str, cx: &App) -> SlashClaim {
        self.lane_agent_chats()
            .map(|(_, view)| pane_slash_claim(view.read(cx), name))
            .fold(SlashClaim::Unsaid, SlashClaim::merge)
    }

    /// Stop whatever this pane has in flight. `cancel_agent_turn_if_active`
    /// owns the settle edge and the completion firing, so nothing here
    /// duplicates the activity state machine.
    pub(crate) fn control_stop(
        &mut self,
        pane: PaneId,
        cx: &mut Context<Self>,
    ) -> Result<StopDisposition, ControlError> {
        if self.agent_chat_view(pane).is_none() {
            return Err(ControlError::TargetGone);
        }
        if self.cancel_agent_turn_if_active(pane, cx) {
            Ok(StopDisposition::Stopped)
        } else {
            Ok(StopDisposition::AlreadyIdle)
        }
    }

    /// Test hook: put `names` on `pane`'s advertised command list. Slash
    /// ownership turns on that one field, and a test outside
    /// `crate::workspace` cannot reach the view that holds it.
    #[cfg(test)]
    pub(crate) fn advertise_slash_commands_for_test(
        &self,
        pane: PaneId,
        names: &[&str],
        cx: &mut App,
    ) {
        let view = self.agent_chat_view(pane).expect("pane").clone();
        view.update(cx, |v, _| {
            v.session_config.available_commands = names
                .iter()
                .map(|n| daruda_acp::SlashCommand {
                    name: (*n).to_string(),
                    description: String::new(),
                    input: daruda_acp::SlashCommandInput::NoInput,
                })
                .collect();
        });
    }
}

/// What one pane's agent says about `/name`: an empty advertised list is
/// [`SlashClaim::Unsaid`] (the session has not spoken, which is neither a
/// claim nor a denial), a list naming it is `Claims`, and any other list is
/// `Disclaims`. The one predicate both slash-ownership questions aggregate —
/// one pane's answer, and every pane's.
fn pane_slash_claim(view: &AgentChatView, name: &str) -> SlashClaim {
    let advertised = &view.session_config.available_commands;
    if advertised.is_empty() {
        SlashClaim::Unsaid
    } else if advertised.iter().any(|c| c.name == name) {
        SlashClaim::Claims
    } else {
        SlashClaim::Disclaims
    }
}

/// The last assistant message this pane finished saying.
///
/// Skips an empty one for the reason every other reader does — it would put a
/// blank body under a header — and a streaming one because it is not finished.
fn last_assistant_text(view: &AgentChatView) -> Option<String> {
    view.items.iter().rev().find_map(|item| match item {
        daruda_acp::ChatItem::AssistantText {
            text,
            streaming: false,
            ..
        } if !text.trim().is_empty() => Some(text.clone()),
        _ => None,
    })
}

/// The single conversion between the workspace-private activity state and the
/// control surface's own enum. Keeping it here is what lets
/// `control/result.rs` stay outside `crate::workspace`.
fn map_activity(state: ActivityState) -> Activity {
    match state {
        ActivityState::Idle => Activity::Idle,
        ActivityState::Working => Activity::Working,
        ActivityState::AwaitingPermission => Activity::AwaitingPermission,
    }
}

/// Whether the pane can be talked to. A dormant `Idle` has no session yet and
/// a phone prompt would only wake it on the desktop's terms, so it reports
/// `Unavailable` rather than a healthy idle.
fn map_health(status: &AgentSessionStatus) -> Health {
    match status {
        AgentSessionStatus::Error { .. } => Health::Error,
        AgentSessionStatus::Idle => Health::Unavailable,
        AgentSessionStatus::PreparingRuntime(_)
        | AgentSessionStatus::Connecting
        | AgentSessionStatus::Handshaking(_)
        | AgentSessionStatus::Connected => Health::Ok,
    }
}

/// The pane's last-activity stamp as unix seconds. The view keeps it as
/// RFC 3339 for display; a control result is consumed by machines as often as
/// by people, so it carries the numeric form.
fn last_activity_unix(view: &AgentChatView) -> Option<u64> {
    let raw = view.session_updated_at.as_deref()?;
    let parsed = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    u64::try_from(parsed.timestamp()).ok()
}

/// Scaffolding for the control-surface tests, which need scenarios the pane
/// ops can reach but `crate::test_support` cannot — everything driven below is
/// `pub(in crate::workspace)` at its source.
#[cfg(test)]
impl Workspace {
    /// Insert a pane the way [`Self::control_insert_chat`] does — *without*
    /// revealing it.
    ///
    /// Same reason: revealing focuses the pane, focusing starts a real ACP
    /// adapter, and its task outlives the test and then trips gpui's
    /// determinism assert in whichever test runs next. Every control test
    /// addresses panes by id, so none of them needs the focus.
    pub(crate) fn open_agent_chat_pane_for_test(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        let agent_id =
            crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id(
                &self.agents,
                self.last_agent_id.as_deref(),
            );
        let cwds = self.active_lane_cwds();
        self.insert_agent_chat_pane(agent_id, cwds, window, cx)
            .expect("pane opened")
    }

    /// Add `count` lanes to the active project, each holding one agent-chat
    /// pane, and return their pane ids. What makes `main_area.runtimes` big
    /// enough for hash iteration order to diverge from the order a listing
    /// must report.
    pub(crate) fn open_agent_chat_panes_in_fresh_lanes_for_test(
        &mut self,
        count: u32,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Vec<PaneId> {
        let project = self.active.project;
        let root = self
            .project_for(project)
            .map(|p| p.root.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        (0..count)
            .map(|i| {
                let lane_id = self.alloc_id();
                let mut lane = crate::lane::Lane::default_for_project(lane_id, root.clone());
                lane.tab_order = i + 1;
                let target = LaneRef {
                    project,
                    lane: lane_id,
                };
                self.project_for_mut(project)
                    .expect("active project")
                    .lanes
                    .push(lane);
                self.activate_lane(target, window, cx);
                self.open_agent_chat_pane_for_test(window, cx)
            })
            .collect()
    }

    /// Drive one pane into each of the three states `/brief` counts. Each
    /// writes the field the sanctioned predicate reads — `activity_state`
    /// folds turn + permissions, and `map_health` reads `status`.
    pub(crate) fn set_pane_working_for_test(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        let view = self.agent_chat_view(pane).expect("pane").clone();
        view.update(cx, |v, _| v.set_turn_in_flight());
    }

    pub(crate) fn set_pane_awaiting_permission_for_test(
        &mut self,
        pane: PaneId,
        cx: &mut Context<Self>,
    ) {
        let view = self.agent_chat_view(pane).expect("pane").clone();
        view.update(cx, |v, _| {
            v.pending_permissions.insert(1);
        });
    }

    pub(crate) fn set_pane_errored_for_test(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        let view = self.agent_chat_view(pane).expect("pane").clone();
        view.update(cx, |v, cx| {
            v.set_error("boom".into(), daruda_acp::Remedy::NoneAvailable, cx)
        });
    }
}

#[cfg(test)]
mod tests;
