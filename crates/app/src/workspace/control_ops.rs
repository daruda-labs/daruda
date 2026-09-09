//! `impl Workspace` for the external control surface.
//!
//! Two shapes only: a read-only snapshot the app-level dispatcher aggregates
//! across windows, and targeted actions that delegate to the existing agent
//! chat ops. Nothing here is a new mutation site — every write goes through a
//! method that already owned it.

use gpui::{App, Context};

use std::path::Path;

use crate::control::result::{
    Activity, ChatSummary, ControlError, FlowEntry, FlowOriginKind, Health, LaneEntry, LaneHandle,
    SendDisposition, StopDisposition, sanitize_title,
};
use crate::telegram::bridge::PaneRef;
use crate::workspace::Workspace;
use crate::workspace::flow_paths::{FlowOrigin, FoundFlow, flow_label};
use crate::workspace::flow_request::FlowSelection;
use crate::workspace::main_area::agent_chat_pane::view::{
    ActivityState, AgentChatView, AgentSessionStatus, PromptDispatch,
};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_store::project::{LaneRef, ProjectId};

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
        self.main_area
            .runtimes
            .iter()
            .flat_map(|(lane_ref, rt)| {
                // Lane-scoped, so it is resolved once per lane rather than per
                // pane: daruda tracks "not looked at yet" on the lane row.
                let unread = self.lane_for(*lane_ref).is_some_and(|l| l.is_unread);
                rt.panes.iter().filter_map(move |pane| {
                    let v = pane.agent_chat_view()?.read(cx);
                    Some((
                        *lane_ref,
                        ChatSummary {
                            target: PaneRef {
                                workspace: uuid,
                                pane: pane.id,
                            },
                            is_active_lane: *lane_ref == active,
                            activity: map_activity(v.activity_state()),
                            health: map_health(&v.status),
                            unread,
                            title: v.activity_title().and_then(sanitize_title),
                            last_activity: last_activity_unix(v),
                        },
                    ))
                })
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
                    .filter(|p| p.agent_chat_view().is_some())
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
    /// Split from [`Self::control_lane_finalize`] because the step between
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
    /// prompt path so slash classification and post-turn flush stay on their
    /// single funnel; only the disposition is surfaced instead of relayed.
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

    /// Every flow the active lane can run, in the order the flows panel shows
    /// them (by file name, whatever scope each came from).
    ///
    /// `FlowSources::list_flows` already decided which file a name resolves
    /// to — the repository's copy shadows the person's — so this list holds
    /// one entry per name and an ambiguous name is unrepresentable.
    pub(crate) fn control_flow_list(&self) -> Vec<FlowEntry> {
        self.flow_sources()
            .map(|sources| {
                sources
                    .list_flows()
                    .into_iter()
                    .map(|found| FlowEntry {
                        name: flow_label(&found.path),
                        origin: map_origin(found.origin),
                        lane: LaneHandle::new(self.uuid(), self.active),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Start `name` in the active lane.
    ///
    /// Every refusal is taken *before* dispatch, because a refusal after it
    /// surfaces as a desktop toast the caller never sees and would leave a
    /// phone told a run started that never did. A flow is lane-scoped, so with
    /// no active lane there is nowhere to put it; one that would open a desktop
    /// dialog (an `ask` permission policy, or a profile question) would hang a
    /// caller that cannot answer it; a lane runs one flow at a time, enforced
    /// by an on-disk lock; and a flow that does not pass its own static checks
    /// would be refused on submit.
    pub(crate) fn control_flow_run(
        &mut self,
        name: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<FlowEntry, ControlError> {
        let Some(cwd) = self.active_lane_root() else {
            return Err(ControlError::NoActiveLane);
        };
        let found = self
            .resolve_flow_name(name)
            .ok_or_else(|| ControlError::FlowNotFound {
                name: name.to_string(),
            })?;
        let label = flow_label(&found.path);
        if needs_desktop_answer(&found.path) {
            return Err(ControlError::FlowNeedsInteraction { name: label });
        }
        // Both halves of "a run is already going": the on-disk lock catches
        // another process, and `runs` catches this one. Asking only the lock
        // leaves the guard inside `run_flow_at` to answer the second case by
        // opening its "stop it?" picker — a desktop dialog raised by a phone
        // command, which is the one thing this surface must never do.
        if self.lane_holder(&cwd).is_some() || self.runs.is_running(self.active) {
            return Err(ControlError::FlowLocked { name: label });
        }
        // Everything the run would refuse *after* dispatch surfaces on the
        // desktop as a toast, which a phone never sees — so the static verdict
        // is taken here, where it can still become an answer. Same check
        // `Validate Flow…` runs: no lock, no run directory.
        let runnable = matches!(
            self.check_flow(&found.path, None, cx),
            Ok(issues) if issues.is_empty()
        );
        if !runnable {
            return Err(ControlError::FlowRefused { name: label });
        }

        let entry = FlowEntry {
            name: label,
            origin: map_origin(found.origin),
            lane: LaneHandle::new(self.uuid(), self.active),
        };
        // The guard reads the lock again, so a run that took it in between is
        // still caught — and answered, rather than reported as started.
        if !self.run_flow_at(
            &found.path,
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            FlowSelection::default(),
            window,
            cx,
        ) {
            return Err(ControlError::FlowLocked { name: entry.name });
        }
        // The guard passing does not mean a run exists: the request builder
        // can still refuse after it (an unusable session host, an account
        // directory it cannot prepare). Marking the run is also how that is
        // detected — no run, nothing to mark.
        //
        // WORKAROUND: `submit_flow_run` has the typed `FlowSubmitError` and
        // discards it into `report_flow_refusal`, so the real reason is only
        // ever on screen. Surfacing it would mean threading a `Result` through
        // `run_flow_at` → `start_flow` → `dispatch_flow`, whose other branches
        // (`validate_flow`, `open_flow_graph`) are not runs and whose guard's
        // third outcome is a picker rather than an error — a restructure of
        // the flow host's dispatch, which is a separate subsystem from this
        // one. Deferred; until then the answer says only what is known.
        // Marked after dispatch rather than threaded into the submission: the
        // run is inserted synchronously inside `run_flow_at`, and nothing here
        // awaits, so the event pump cannot retire it in between. Threading the
        // origin instead would touch five signatures on a path desktop callers
        // share, for one flag.
        if !self.runs.answer_telegram_on_end(self.active) {
            return Err(ControlError::FlowNotStarted { name: entry.name });
        }
        Ok(entry)
    }

    /// The flow `name` points at, accepting either the file name as written
    /// on disk or its stem — a phone keyboard should not have to type
    /// `.yaml`.
    fn resolve_flow_name(&self, name: &str) -> Option<FoundFlow> {
        let sources = self.flow_sources()?;
        sources.list_flows().into_iter().find(|found| {
            let file = found.path.file_name().map(|n| n.to_string_lossy());
            let stem = found.path.file_stem().map(|n| n.to_string_lossy());
            file.as_deref() == Some(name) || stem.as_deref() == Some(name)
        })
    }
}

/// The single conversion between the workspace-private flow origin and the
/// control surface's own enum, for the same reason [`map_activity`] exists.
fn map_origin(origin: FlowOrigin) -> FlowOriginKind {
    match origin {
        FlowOrigin::Repo => FlowOriginKind::Repo,
        FlowOrigin::Project => FlowOriginKind::Project,
        FlowOrigin::Global => FlowOriginKind::Global,
    }
}

/// Would running this flow put a dialog on the desktop that only a person at
/// the machine can answer? Two shapes do: an agent node (or the repair agent)
/// whose permission policy is `ask`, and a file declaring profiles, which
/// makes the picker ask which one to run.
///
/// A file that cannot be read or parsed answers `false`: the run that follows
/// fails on the same read and names it properly, and refusing here would
/// report a parse error as an interaction problem.
fn needs_desktop_answer(path: &Path) -> bool {
    use daruda_flow::model::{NodeKind, PermissionPolicy};

    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if daruda_flow::load::profiles(&text).is_ok_and(|p| !p.is_empty()) {
        return true;
    }
    let Ok(inspected) = daruda_flow::inspect(&text, None) else {
        return false;
    };
    let flow = inspected.loaded.flow();
    let node_asks = flow.nodes.iter().any(|node| {
        matches!(&node.kind, NodeKind::Agent(body) if body.agent.permission == PermissionPolicy::Ask)
    });
    let repair_asks = flow
        .default_agent
        .as_ref()
        .is_some_and(|agent| agent.permission == PermissionPolicy::Ask);
    node_asks || repair_asks
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
mod tests {
    use super::*;
    use crate::test_support::workspace_with_agent_chat;
    use gpui::AppContext as _;

    #[test]
    fn health_separates_a_dead_session_from_one_that_never_started() {
        assert_eq!(
            map_health(&AgentSessionStatus::Error {
                message: "boom".into(),
                remedy: daruda_acp::Remedy::NoneAvailable,
            }),
            Health::Error
        );
        assert_eq!(map_health(&AgentSessionStatus::Idle), Health::Unavailable);
        assert_eq!(map_health(&AgentSessionStatus::Connected), Health::Ok);
        assert_eq!(map_health(&AgentSessionStatus::Connecting), Health::Ok);
    }

    #[test]
    fn every_activity_state_maps_to_its_own_control_variant() {
        assert_eq!(map_activity(ActivityState::Idle), Activity::Idle);
        assert_eq!(map_activity(ActivityState::Working), Activity::Working);
        assert_eq!(
            map_activity(ActivityState::AwaitingPermission),
            Activity::AwaitingPermission
        );
    }

    /// A worktree with no agent chat in it still exists and is still a place
    /// to open one — which is exactly why this is not derived from
    /// `control_snapshot`.
    #[gpui::test]
    async fn lane_list_reports_every_lane_not_just_chatty_ones(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let extra = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    let project = ws.active.project;
                    let root = ws.project_for(project).expect("project").root.clone();
                    let lane_id = ws.alloc_id();
                    let lane = crate::lane::Lane::default_for_project(lane_id, root);
                    ws.project_for_mut(project)
                        .expect("project")
                        .lanes
                        .push(lane);
                    let _ = (window, cx);
                    LaneRef {
                        project,
                        lane: lane_id,
                    }
                })
            })
            .expect("window is live");

        fixture.workspace.read_with(cx, |ws, _| {
            let lanes = ws.control_lane_list();
            assert!(
                lanes.len() >= 2,
                "the chatless lane is listed too: {lanes:?}"
            );
            assert!(lanes.iter().all(|l| !l.name.is_empty()));
            let chatless = lanes
                .iter()
                .find(|l| l.target.lane_ref() == extra)
                .expect("the added lane");
            assert_eq!(chatless.chats, 0);
            assert!(!chatless.is_active);
            let active = lanes.iter().find(|l| l.is_active).expect("one active lane");
            assert_eq!(active.chats, 1, "the fixture's chat is counted");
        });
    }

    #[gpui::test]
    async fn chat_new_opens_a_pane_in_the_named_lane(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let before = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_snapshot(cx).len());
        let target = fixture.workspace.read_with(cx, |ws, _| ws.active);

        // The insert half only: revealing spawns a real ACP adapter, whose
        // background task outlives the test and then trips gpui's determinism
        // assert in whichever test runs next.
        let pane = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.control_insert_chat_for_test(target, window, cx)
                })
            })
            .expect("window is live")
            .expect("created");

        fixture.workspace.read_with(cx, |ws, cx| {
            let snap = ws.control_snapshot(cx);
            assert_eq!(snap.len(), before + 1);
            assert!(snap.iter().any(|(_, s)| s.target.pane == pane));
            assert_eq!(
                ws.control_active_lane(),
                target,
                "the pane went into the worktree that was named"
            );
        });
    }

    #[gpui::test]
    async fn chat_new_on_a_missing_lane_is_target_gone(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let bogus = LaneRef {
            project: 9_999,
            lane: 9_999,
        };
        let result = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture
                    .workspace
                    .update(cx, |ws, cx| ws.control_chat_new(bogus, None, window, cx))
            })
            .expect("window is live");
        assert_eq!(result, Err(ControlError::TargetGone));
    }

    #[gpui::test]
    async fn a_lane_plan_for_a_missing_project_is_target_gone(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.read_with(cx, |ws, _| {
            assert!(matches!(
                ws.control_lane_plan(9_999, "x", None),
                Err(ControlError::TargetGone)
            ));
        });
    }

    /// The plan has to name a sibling of the repo, so a lane an agent made
    /// sits where a lane the user made would.
    /// An agent-supplied name reaches `git worktree add -b` and a filesystem
    /// path, so it is validated before anything is spawned and before the
    /// approval card quotes it. Letting git reject it instead would cost the
    /// user a tap and then blame them for it.
    #[gpui::test]
    async fn a_hostile_lane_name_is_refused_before_anything_runs(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.read_with(cx, |ws, _| {
            let project = ws.control_active_lane().project;
            for name in [
                "",
                "   ",
                "../../pwn",
                "has:colon",
                "has space",
                "trailing.",
                "/leading",
                "tilde~",
                "star*",
                "back\\slash",
            ] {
                assert!(
                    matches!(
                        ws.control_lane_plan(project, name, None),
                        Err(ControlError::LaneNameInvalid)
                    ),
                    "{name:?} must not reach git"
                );
            }
        });
    }

    /// A slash is legal in a branch name but must not nest the checkout: the
    /// path suffix folds it, exactly as the create form does.
    #[gpui::test]
    async fn a_slashed_branch_name_stays_one_directory_deep(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.read_with(cx, |ws, _| {
            let project = ws.control_active_lane().project;
            let root = ws.project_for(project).expect("project").root.clone();
            let plan = ws
                .control_lane_plan(project, "feat/x", None)
                .expect("a slash is a legal branch name");
            assert_eq!(plan.branch, "feat/x");
            assert_eq!(plan.new_path.parent(), root.parent());
            assert!(
                plan.new_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-feat-x")),
                "{:?}",
                plan.new_path
            );
        });
    }

    #[gpui::test]
    async fn a_lane_plan_puts_the_checkout_beside_its_repo(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.read_with(cx, |ws, _| {
            let project = ws.active.project;
            let root = ws.project_for(project).expect("project").root.clone();
            let plan = ws
                .control_lane_plan(project, "fix-picker", None)
                .expect("planned");
            assert_eq!(plan.branch, "fix-picker");
            assert_eq!(plan.repo_root, root);
            assert_eq!(plan.new_path.parent(), root.parent());
            let name = plan
                .new_path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("name");
            assert!(name.ends_with("-fix-picker"), "{name}");
            assert!(plan.session_host.is_none());
        });
    }

    #[gpui::test]
    async fn snapshot_lists_agent_chat_panes_only(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.read_with(cx, |ws, cx| {
            let snap = ws.control_snapshot(cx);
            assert_eq!(
                snap.len(),
                1,
                "one agent chat pane, terminal panes excluded"
            );
            assert_eq!(snap[0].1.target.pane, fixture.pane());
            assert_eq!(snap[0].1.target.workspace, ws.uuid());
            assert!(
                snap[0].1.is_active_lane,
                "the pane opened in the active lane"
            );
        });
    }

    #[gpui::test]
    async fn snapshot_carries_the_session_title(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            let view = ws.agent_chat_view(fixture.pane()).expect("view").clone();
            view.update(cx, |v, _| {
                v.set_session_title_for_test("restore invariants")
            });
        });
        fixture.workspace.read_with(cx, |ws, cx| {
            let snap = ws.control_snapshot(cx);
            assert_eq!(snap[0].1.title.as_deref(), Some("restore invariants"));
        });
    }

    /// The fixture pane has no live session, so the prompt lands in the pane
    /// queue — which is exactly the branch `Queued` exists to report rather
    /// than pass off as delivered.
    #[gpui::test]
    async fn say_on_a_pane_with_no_live_session_reports_queued(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_say(fixture.pane(), "hello".into(), cx),
                Ok(SendDisposition::Queued)
            );
        });
    }

    #[gpui::test]
    async fn say_on_a_missing_pane_is_target_gone(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_say(9_999, "hello".into(), cx),
                Err(ControlError::TargetGone)
            );
        });
    }

    #[gpui::test]
    async fn stop_on_an_idle_pane_reports_already_idle(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_stop(fixture.pane(), cx),
                Ok(StopDisposition::AlreadyIdle)
            );
        });
    }

    #[gpui::test]
    async fn stop_on_a_missing_pane_is_target_gone(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        fixture.workspace.update(cx, |ws, cx| {
            assert_eq!(ws.control_stop(9_999, cx), Err(ControlError::TargetGone));
        });
    }
}
