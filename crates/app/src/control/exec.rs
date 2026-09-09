//! App-level dispatch for control commands.
//!
//! Enumeration walks every user window for `/list` and `/brief`. Targeted
//! commands resolve their `PaneRef`'s workspace, and `/daruda` ensures the
//! orchestrator.
//!
//! The enumeration walk collects plain data inside the per-window callback and
//! assembles the result afterwards — building it inside would read entities
//! while their workspace is mid-`update` (CLAUDE.md pitfall 5).

use gpui::{App, AppContext as _};

use crate::control::approval::ApprovalOutcome;
use crate::control::result::{
    Activity, BriefSummary, ChatSummary, ControlError, ControlOutcome, ControlResult, FlowEntry,
    Health, LaneGroup, LaneHandle, Listing, PaneAnswer, ProjectGroup, SendDisposition, WindowGroup,
    ask_disposition,
};
use crate::control::spec::{GatedCommand, ResolvedCommand, ResolvedFlowCommand};
use crate::surface::strings as s;
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;
use daruda_store::project::ProjectId;

/// One row of the flat walk, before grouping.
struct Row {
    window: u32,
    /// Identity, not the name: two projects opened from different directories
    /// can share a basename, and grouping by name would merge them into one
    /// listing section that misreports the structure.
    project_id: ProjectId,
    project: String,
    lane: String,
    lane_order: u32,
    summary: ChatSummary,
}

pub(crate) fn listing(cx: &mut App) -> Listing {
    Listing {
        windows: group(collect_rows(cx)),
        omitted: 0,
    }
}

pub(crate) fn brief(cx: &mut App) -> BriefSummary {
    let rows = collect_rows(cx);
    BriefSummary {
        working: count(&rows, |s| s.activity == Activity::Working),
        awaiting_permission: count(&rows, |s| s.activity == Activity::AwaitingPermission),
        error: count(&rows, |s| s.health == Health::Error),
        total: rows.len() as u32,
    }
}

pub(crate) fn run(cmd: ResolvedCommand, cx: &mut App) -> ControlOutcome {
    match cmd {
        ResolvedCommand::List => Ok(ControlResult::Listing(listing(cx))),
        ResolvedCommand::Brief => Ok(ControlResult::Brief(brief(cx))),
        ResolvedCommand::Say { target, text } => in_pane_window(target, cx, |ws, _window, cx| {
            ws.control_say(target.pane, text.clone(), cx)
        })
        .map(|disposition| ControlResult::Sent {
            target,
            disposition,
        }),
        ResolvedCommand::Stop { target } => in_pane_window(target, cx, |ws, _window, cx| {
            ws.control_stop(target.pane, cx)
        })
        .map(|disposition| ControlResult::Stopped {
            target,
            disposition,
        }),
        ResolvedCommand::Read { target } => in_pane_window(target, cx, |ws, _window, cx| {
            ws.control_read(target.pane, cx)
        })
        .map(|text| ControlResult::Transcript { target, text }),
        // A flow is worktree-scoped, and which flows one can run depends on
        // its repository — so two windows genuinely see different sets and the
        // listing may not stop at the first window.
        ResolvedCommand::Flow(ResolvedFlowCommand::List) => {
            let mut flows: Vec<FlowEntry> = Vec::new();
            WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
                flows.extend(ws.control_flow_list());
            });
            // Sorted by name so the listing reads as a vocabulary, but *not*
            // collapsed by it: the same name in two windows names two flows,
            // in two repositories, and each row carries the worktree that
            // says which. A phone renders one row per name — that is the
            // renderer's call, not this one's.
            flows.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then(a.origin.cmp(&b.origin))
                    .then(a.lane.workspace.cmp(&b.lane.workspace))
                    .then(a.lane.project.cmp(&b.lane.project))
                    .then(a.lane.lane.cmp(&b.lane.lane))
            });
            flows.dedup();
            Ok(ControlResult::FlowList { flows })
        }
        // One window, one worktree: the caller named it, so there is nothing
        // to search and nothing that could start in a place it did not ask
        // for.
        ResolvedCommand::Flow(ResolvedFlowCommand::Run { name, lane }) => {
            in_lane_window(lane, cx, |ws, window, cx| {
                ws.control_flow_run(lane.lane_ref(), &name, window, cx)
            })
            // Named by the file that resolved, not by what the caller typed —
            // `ship` may pick `ship.yaml` or `ship.yml`, and the answer has to
            // say which.
            .map(|entry| ControlResult::FlowStarting {
                name: entry.name,
                origin: entry.origin,
                lane: entry.lane,
            })
        }
        ResolvedCommand::Flow(ResolvedFlowCommand::Stop { lane }) => {
            in_lane_window(lane, cx, |ws, _window, cx| {
                ws.control_flow_stop(lane.lane_ref(), cx)
            })
            .map(|disposition| ControlResult::FlowStopped { lane, disposition })
        }
        // Every window's worktrees, not just the active one's: a caller
        // choosing where to open a chat needs the whole set.
        ResolvedCommand::LaneList => {
            let mut lanes = Vec::new();
            WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
                lanes.extend(ws.control_lane_list());
            });
            Ok(ControlResult::LaneListing { lanes })
        }
        // The destination is already concrete, like every other target here:
        // the adapter brought the orchestrator up to name it. Owned in this
        // match rather than in the Telegram adapter so an adapter speaking
        // `ResolvedCommand` directly inherits the same handling.
        ResolvedCommand::AskOrchestrator {
            text,
            destination,
            connecting,
        } => ask_orchestrator(text, destination, connecting, cx),
        // Handled by `run_waiting`, which can hand back a channel. Reaching
        // here would mean an adapter called the wrong entry point, and
        // answering "still working" about a prompt never sent would be a lie.
        ResolvedCommand::AskPane { target, .. } => Ok(ControlResult::Answer {
            target,
            answer: PaneAnswer::Queued,
        }),
    }
}

/// Prompt a pane and hand back a promise of the turn's answer.
///
/// A waiter is registered only for a prompt that went out *now*: a delivered
/// prompt means the pane was idle, so the next settle edge is this turn's and
/// at most one call is ever waiting on a pane. One that queued behind a turn
/// already in flight would be answered by that turn's edge instead, which is
/// somebody else's answer — so it is told `Queued` and can read the reply
/// later.
pub(crate) fn run_waiting(cmd: ResolvedCommand, cx: &mut App) -> Dispatch {
    let ResolvedCommand::AskPane { target, text } = cmd else {
        // Unreachable: `convert` builds `Command::Waiting` for exactly one
        // tool. Answering through the immediate path is the harmless way to
        // be wrong.
        return Dispatch::Ready(run(cmd, cx));
    };
    let sent = in_pane_window(target, cx, |ws, _window, cx| {
        ws.control_say(target.pane, text.clone(), cx)
    });
    let answered = |answer| Dispatch::Ready(Ok(ControlResult::Answer { target, answer }));
    match sent {
        Err(refusal) => Dispatch::Ready(Err(refusal)),
        Ok(SendDisposition::Queued) => answered(PaneAnswer::Queued),
        // `/clear` and its kin never reach the agent, so no turn will settle.
        Ok(SendDisposition::HandledLocally) => answered(PaneAnswer::NoAnswer),
        Ok(SendDisposition::Delivered) => {
            let (deadline, answer) = crate::control::ask::wait_for(target, cx);
            let (tx, rx) = smol::channel::bounded(1);
            cx.background_spawn(async move {
                // A closed channel is the app going away, which the caller
                // hears as the target being unreachable.
                let outcome = match answer.recv().await {
                    Ok(answer) => Ok(ControlResult::Answer { target, answer }),
                    Err(_) => Err(ControlError::TargetGone),
                };
                let _ = tx.send(Some(outcome)).await;
            })
            .detach();
            Dispatch::Deferred {
                outcome: rx,
                pending: Pending::Waiter {
                    pane: target,
                    deadline,
                },
            }
        }
    }
}

/// The first open window whose active worktree can run `name`.
///
/// A text adapter's default target: a person typing `/flow ship` has not named
/// a worktree, and filling that in is what the adapter owes the executor,
/// which takes only concrete ones. It lives here rather than in the adapter
/// because walking windows needs the `App` that adapter's pure resolution step
/// does not have — the same reason `/daruda` finishes here.
///
/// Keeps the two refusals apart: no worktree anywhere means there is nowhere to
/// run, which is a different thing to tell someone than "no flow by that name".
pub(crate) fn first_lane_offering(name: &str, cx: &mut App) -> Result<LaneHandle, ControlError> {
    let mut any_lane = false;
    let mut found: Option<LaneHandle> = None;
    WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
        any_lane |= ws.control_has_active_lane();
        if found.is_none() {
            found = ws.control_active_lane_offers(name);
        }
    });
    match found {
        Some(lane) => Ok(lane),
        None if any_lane => Err(ControlError::FlowNotFound {
            name: name.to_string(),
        }),
        None => Err(ControlError::NoActiveLane),
    }
}

/// Put `text` on the orchestrator's pane.
///
/// The reply is `Accepted`, never the answer: the agent's response arrives
/// later as that pane's own completion ping.
fn ask_orchestrator(
    text: String,
    destination: PaneRef,
    connecting: bool,
    cx: &mut App,
) -> ControlOutcome {
    // `/list` never names the orchestrator, so fold a missing pane into its
    // own error.
    let send = in_pane_window(destination, cx, |ws, _window, cx| {
        ws.control_say(destination.pane, text.clone(), cx)
    })
    .map_err(|_| ControlError::OrchestratorUnavailable)?;
    Ok(ControlResult::Accepted {
        disposition: ask_disposition(connecting, send),
    })
}

/// A command's answer, or a promise of one.
///
/// The two shapes are not interchangeable: a refusal the guards can make on
/// the spot must reach the caller without a round trip, and everything else
/// has to wait for one.
pub(crate) enum Dispatch {
    Ready(ControlOutcome),
    /// Resolves once whatever [`Pending`] names has happened.
    Deferred {
        /// `None` when the caller took the question back: there is an outcome
        /// to stop waiting for, but none to answer with.
        outcome: smol::channel::Receiver<Option<ControlOutcome>>,
        pending: Pending,
    },
}

/// What a deferred call is waiting on, and so what taking it back means.
///
/// Carried rather than assumed, because the two are not the same act. A card
/// is a question whose work has not started, so withdrawing prevents it; a
/// turn is already running for the pane's own sake, so withdrawing is only
/// giving up on hearing how it went.
#[derive(Clone, Copy)]
pub(crate) enum Pending {
    Approval(crate::control::approval::ApprovalId),
    Waiter {
        pane: PaneRef,
        /// When this wait answers itself. What a caller still holding a record
        /// of it prunes by — a turn has no `is_waiting` a card does.
        deadline: std::time::Instant,
    },
}

impl Pending {
    /// Whether this is still worth holding a record of.
    pub(crate) fn is_live(self, cx: &App) -> bool {
        match self {
            Self::Approval(id) => crate::control::approval::is_waiting(id, cx),
            Self::Waiter { pane, deadline } => {
                std::time::Instant::now() < deadline && crate::control::ask::is_waiting(pane, cx)
            }
        }
    }

    /// Take the call back. The turn behind a `Waiter` keeps running.
    pub(crate) fn withdraw(self, cx: &mut App) {
        match self {
            Self::Approval(id) => {
                crate::control::approval::withdraw(id, cx);
            }
            Self::Waiter { pane, .. } => {
                crate::control::ask::withdraw(pane, cx);
            }
        }
    }
}

/// Run a command that has to wait: guards, then the user's approval, then the
/// work.
///
/// The order matters. A budget the agent has spent and a pane it may not
/// address are refusals the user has no say in, so they are answered before a
/// card is ever put on their phone.
pub(crate) fn run_gated(cmd: GatedCommand, cx: &mut App) -> Dispatch {
    if let Err(refusal) = guard_gated(&cmd, cx) {
        return Dispatch::Ready(Err(refusal));
    }
    let summary = approval_summary(&cmd, cx);
    let (approval, answer) = crate::control::approval::request(summary, cx);
    let (tx, rx) = smol::channel::bounded(1);
    cx.spawn(async move |cx| {
        let outcome: Option<ControlOutcome> = match answer.recv().await {
            // Re-checked here, not only before the card: the guard above ran
            // before *this* request waited, so N cards opened together would
            // all have passed a budget that only one of them can spend.
            Ok(ApprovalOutcome::Approved) => Some(match cx.update(|cx| guard_gated(&cmd, cx)) {
                Ok(()) => perform_gated(cmd, cx).await,
                Err(refusal) => Err(refusal),
            }),
            // Not a refusal: the user never said no, and the caller has freed
            // the id it would be answered on. Nothing ran and nothing is owed,
            // which is what `None` says and no error variant could.
            Ok(ApprovalOutcome::Withdrawn) => None,
            Ok(ApprovalOutcome::Refused) => Some(Err(ControlError::ApprovalRefused)),
            Ok(ApprovalOutcome::Undeliverable) => Some(Err(ControlError::ApprovalUnavailable)),
            Ok(ApprovalOutcome::TooManyPending) => Some(Err(ControlError::ApprovalsPending)),
            // A dropped channel means the app is going away, which the caller
            // cannot act on either — report it as the unanswered card it is.
            Ok(ApprovalOutcome::TimedOut) | Err(_) => Some(Err(ControlError::ApprovalTimedOut)),
        };
        let _ = tx.send(outcome).await;
    })
    .detach();
    Dispatch::Deferred {
        outcome: rx,
        pending: Pending::Approval(approval),
    }
}

/// The refusals that precede the card.
fn guard_gated(cmd: &GatedCommand, cx: &mut App) -> Result<(), ControlError> {
    match cmd {
        GatedCommand::LaneCreate { name, .. } => {
            // Before the card, not with the plan that quotes it: a name git
            // will reject cannot become a worktree however the user answers,
            // so asking them would cost a tap and then blame them for a
            // mistake they did not make. Pure — the same predicate
            // `control_lane_plan` sanitizes with, asked earlier.
            if daruda_core::git::sanitize_branch_name(name).is_none() {
                return Err(ControlError::LaneNameInvalid);
            }
            crate::control::guards::guard_agent_budget(cx)
        }
        // Opening a chat costs no budget: it is one pane in a worktree the
        // user already has, and the gate is the whole check.
        GatedCommand::ChatNew { lane, .. } => {
            in_lane_window(*lane, cx, |_ws, _window, _cx| Ok(())).map(|_| ())
        }
    }
}

/// Run `f` against the window `workspace` names, when `holds` agrees it is the
/// right one. `TargetGone` when no window matches — including when the window
/// is there but no longer holds what the command named.
fn in_window<T>(
    workspace: daruda_store::project::WorkspaceUuid,
    holds: impl Fn(&Workspace) -> bool,
    cx: &mut App,
    mut f: impl FnMut(
        &mut Workspace,
        &mut gpui::Window,
        &mut gpui::Context<Workspace>,
    ) -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let mut out = Err(ControlError::TargetGone);
    let run = |ws: &mut Workspace, window: &mut gpui::Window, cx: &mut gpui::Context<Workspace>| {
        if ws.uuid() == workspace && holds(ws) {
            out = f(ws, window, cx);
        }
    };
    WindowRegistry::for_each_workspace(cx, run);
    out
}

fn in_lane_window<T>(
    lane: LaneHandle,
    cx: &mut App,
    f: impl FnMut(
        &mut Workspace,
        &mut gpui::Window,
        &mut gpui::Context<Workspace>,
    ) -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    in_window(
        lane.workspace,
        |ws| ws.control_has_lane(lane.lane_ref()),
        cx,
        f,
    )
}

/// Same, for a project the caller named by window uuid plus local id.
fn in_project_window<T>(
    workspace: daruda_store::project::WorkspaceUuid,
    project: daruda_store::project::ProjectId,
    cx: &mut App,
    f: impl FnMut(
        &mut Workspace,
        &mut gpui::Window,
        &mut gpui::Context<Workspace>,
    ) -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    in_window(workspace, |ws| ws.control_has_project(project), cx, f)
}

/// One line naming what the user is being asked to allow. Built here rather
/// than on `GatedCommand`, which is the contract every adapter shares and
/// deliberately carries no rendered sentences.
fn approval_summary(cmd: &GatedCommand, cx: &mut App) -> String {
    match cmd {
        GatedCommand::LaneCreate { name, .. } => s::control_approval_lane_create(name),
        GatedCommand::ChatNew { lane, .. } => {
            let path = in_lane_window(*lane, cx, |ws, _window, _cx| {
                Ok(s::control_lane_path(
                    &ws.control_project_name(lane.project),
                    &ws.control_lane_name(lane.lane_ref()),
                ))
            })
            .unwrap_or_default();
            s::control_approval_chat_new(&path)
        }
    }
}

/// Do the thing, now that the user has said yes.
async fn perform_gated(cmd: GatedCommand, cx: &mut gpui::AsyncApp) -> ControlOutcome {
    match cmd {
        GatedCommand::ChatNew { lane, agent } => cx
            .update(|cx| {
                in_lane_window(lane, cx, |ws, window, cx| {
                    ws.control_chat_new(lane.lane_ref(), agent.clone(), window, cx)
                })
            })
            .map(|target| ControlResult::ChatCreated { target }),
        GatedCommand::LaneCreate {
            workspace,
            project,
            name,
            base_ref,
            agent,
            prompt,
        } => create_lane(workspace, project, name, base_ref, agent, prompt, cx).await,
    }
}

/// Create a worktree, on whichever window holds the project.
///
/// The three-step sequence (plan, background git, register) lives in
/// `workspace::control_lane_ops` so its `CreateWorktreePlan` never leaves
/// `crate::workspace`; this only picks the window and waits.
async fn create_lane(
    workspace: daruda_store::project::WorkspaceUuid,
    project: daruda_store::project::ProjectId,
    name: String,
    base_ref: Option<String>,
    agent: Option<String>,
    prompt: Option<String>,
    cx: &mut gpui::AsyncApp,
) -> ControlOutcome {
    let rx = cx.update(|cx| {
        in_project_window(workspace, project, cx, |ws, window, cx| {
            Ok(ws.control_create_lane(
                project,
                name.clone(),
                base_ref.clone(),
                agent.clone(),
                window,
                cx,
            ))
        })
    })?;
    let (target, chat) = rx.recv().await.unwrap_or(Err(ControlError::TargetGone))?;
    let target = LaneHandle::new(workspace, target);

    // Spent only now: a refused or failed creation costs the agent nothing.
    cx.update(crate::control::guards::note_agent_created_lane);

    if let Some(prompt) = prompt {
        // Best effort, and not its own outcome: the worktree exists either
        // way, and the caller can send the prompt again.
        cx.update(|cx| {
            let _ = in_pane_window(chat, cx, |ws, _window, cx| {
                ws.control_say(chat.pane, prompt.clone(), cx)
            });
        });
    }
    Ok(ControlResult::LaneCreated { target, chat })
}

fn count(rows: &[Row], pred: impl Fn(&ChatSummary) -> bool) -> u32 {
    rows.iter().filter(|r| pred(&r.summary)).count() as u32
}

fn collect_rows(cx: &mut App) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut window = 0u32;
    // Every workspace: a listing answers "what is the user working on?", and
    // it is `control_snapshot` — through `lane_agent_chats` — that keeps the
    // orchestrator's own chat out of it, not the choice of walk here.
    WindowRegistry::for_each_workspace(cx, |ws, _win, cx| {
        for (lane_ref, summary) in ws.control_snapshot(cx) {
            rows.push(Row {
                window,
                project_id: lane_ref.project,
                project: ws.control_project_name(lane_ref.project),
                lane: ws.control_lane_name(lane_ref),
                lane_order: ws.control_lane_tab_order(lane_ref),
                summary,
            });
        }
        window += 1;
    });
    // `runtimes` is a HashMap, and window registration order is not a
    // guarantee we want ordinals to depend on. Sort once, here.
    rows.sort_by(|a, b| {
        a.window
            .cmp(&b.window)
            // Name first so the listing reads alphabetically, then id so two
            // same-named projects stay apart instead of interleaving.
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.project_id.cmp(&b.project_id))
            .then_with(|| a.lane_order.cmp(&b.lane_order))
            .then_with(|| a.summary.target.pane.cmp(&b.summary.target.pane))
    });
    rows
}

fn group(rows: Vec<Row>) -> Vec<WindowGroup> {
    let mut windows: Vec<WindowGroup> = Vec::new();
    // Tracks which project the open group belongs to, since `ProjectGroup`
    // carries only the name and two projects can share one.
    let mut open_project: Option<ProjectId> = None;
    for row in rows {
        let win = match windows.last_mut() {
            Some(w) if w.index == row.window => w,
            _ => {
                windows.push(WindowGroup {
                    index: row.window,
                    projects: Vec::new(),
                });
                windows.last_mut().expect("just pushed")
            }
        };
        let proj = match win.projects.last_mut() {
            Some(p) if open_project == Some(row.project_id) => p,
            _ => {
                open_project = Some(row.project_id);
                win.projects.push(ProjectGroup {
                    name: row.project,
                    lanes: Vec::new(),
                });
                win.projects.last_mut().expect("just pushed")
            }
        };
        match proj.lanes.last_mut() {
            Some(l) if l.name == row.lane => l.chats.push(row.summary),
            _ => proj.lanes.push(LaneGroup {
                name: row.lane,
                chats: vec![row.summary],
            }),
        }
    }
    windows
}

/// Run `f` against the workspace `target` names. `WindowRegistry` has no
/// uuid lookup, so this is the same scan `telegram::global::dispatch_to_workspace`
/// does — kept separate because that one returns nothing.
fn in_pane_window<T>(
    target: PaneRef,
    cx: &mut App,
    f: impl FnMut(
        &mut Workspace,
        &mut gpui::Window,
        &mut gpui::Context<Workspace>,
    ) -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    // Every window: a pane the caller named by uuid may be the orchestrator's,
    // and a stop aimed at it is how a runaway is ended.
    in_window(target.workspace, |_| true, cx, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{workspace_for_control, workspace_with_agent_chat};
    use gpui::BorrowAppContext as _;

    #[gpui::test]
    async fn listing_spans_every_window(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        let _b = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            let listing = listing(cx);
            assert_eq!(listing.windows.len(), 2, "both windows enumerated");
            assert_eq!(listing.windows[0].index, 0);
            assert_eq!(listing.windows[1].index, 1);
        });
    }

    /// Calling `listing` twice against an unmutated `HashMap` yields the same
    /// order whether or not `collect_rows` sorts — `RandomState` varies per
    /// map, not per iteration — so equality alone proves nothing. This asserts
    /// the *expected* sequence over a fixture with several lanes, which is
    /// what actually fails if the sort is dropped.
    #[gpui::test]
    async fn listing_order_follows_the_sort_key_not_hashmap_order(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        // Extra lanes, each with a chat pane, so `runtimes` holds enough
        // entries for hash order to diverge from tab order.
        let extra = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.open_agent_chat_panes_in_fresh_lanes_for_test(3, window, cx)
                })
            })
            .expect("window is live");

        let panes: Vec<u64> = cx.update(|cx| {
            listing(cx)
                .windows
                .into_iter()
                .flat_map(|w| w.projects)
                .flat_map(|p| p.lanes)
                .flat_map(|l| l.chats)
                .map(|c| c.target.pane)
                .collect()
        });

        let mut expected = vec![fixture.pane()];
        expected.extend(extra);
        expected.sort_unstable();
        assert_eq!(
            panes, expected,
            "rows must follow (window, project, lane order, pane), not hash order"
        );
    }

    /// The counts `/brief` answers with, each non-zero and each distinct — the
    /// earlier test only checked `total`, so three of the four fields of the
    /// reply were unverified.
    #[gpui::test]
    async fn brief_counts_each_state_separately(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let panes = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.open_agent_chat_panes_in_fresh_lanes_for_test(2, window, cx)
                })
            })
            .expect("window is live");

        fixture.workspace.update(cx, |ws, cx| {
            ws.set_pane_working_for_test(fixture.pane(), cx);
            ws.set_pane_awaiting_permission_for_test(panes[0], cx);
            ws.set_pane_errored_for_test(panes[1], cx);
        });

        cx.update(|cx| {
            let brief = brief(cx);
            assert_eq!(brief.total, 3);
            assert_eq!(brief.working, 1, "{brief:?}");
            assert_eq!(brief.awaiting_permission, 1, "{brief:?}");
            assert_eq!(brief.error, 1, "{brief:?}");
        });
    }

    #[gpui::test]
    async fn brief_counts_across_windows(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        let _b = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            assert_eq!(brief(cx).total, 2);
        });
    }

    #[gpui::test]
    async fn a_window_with_no_chat_pane_contributes_no_rows(cx: &mut gpui::TestAppContext) {
        let _empty = workspace_for_control(cx);
        cx.update(|cx| {
            assert!(listing(cx).windows.is_empty());
            assert_eq!(brief(cx).total, 0);
        });
    }

    /// Run a gated `ChatNew` with a bridge that can actually deliver the card.
    /// Enabled *and* paired, because `send_approval_card` refuses otherwise —
    /// an undeliverable card settles the request instead of waiting.
    fn paired_bridge_run_gated(lane: LaneHandle, cx: &mut gpui::App) -> Dispatch {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(true, Some(42), cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(daruda_config::Config {
                telegram: daruda_config::TelegramConfig {
                    enabled: true,
                    authorized_chat_id: Some(42),
                    ..Default::default()
                },
                ..daruda_config::Config::default()
            });
        });
        run_gated(GatedCommand::ChatNew { lane, agent: None }, cx)
    }

    /// A name git will reject cannot become a worktree however the user
    /// answers, so it must not cost them a tap — the same rule the budget
    /// follows.
    #[gpui::test]
    async fn an_unusable_branch_name_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::telegram::global::install_for_test(false, None, cx);
            // `..` is a name git refuses outright, unlike one that is merely
            // already taken (which needs a `git` call to notice).
            let dispatch = run_gated(
                GatedCommand::LaneCreate {
                    workspace,
                    project,
                    name: "..".into(),
                    base_ref: None,
                    agent: None,
                    prompt: None,
                },
                cx,
            );
            match dispatch {
                Dispatch::Ready(outcome) => {
                    assert_eq!(outcome, Err(ControlError::LaneNameInvalid));
                }
                Dispatch::Deferred { .. } => panic!("an unusable name must not ask"),
            }
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                0,
                "no card was put on the phone"
            );
        });
    }

    /// A budget the agent has spent is not a decision the user gets to make,
    /// so it is answered before a card ever reaches their phone.
    #[gpui::test]
    async fn a_spent_budget_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let (workspace, project) = fixture
            .workspace
            .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::telegram::global::install_for_test(false, None, cx);
            for _ in 0..crate::control::guards::AGENT_LANE_BUDGET {
                crate::control::guards::note_agent_created_lane(cx);
            }
            let dispatch = run_gated(
                GatedCommand::LaneCreate {
                    workspace,
                    project,
                    name: "x".into(),
                    base_ref: None,
                    agent: None,
                    prompt: None,
                },
                cx,
            );
            match dispatch {
                Dispatch::Ready(outcome) => {
                    assert_eq!(outcome, Err(ControlError::AgentLimitReached));
                }
                Dispatch::Deferred { .. } => panic!("a spent budget must not ask"),
            }
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                0,
                "no card was put on the phone"
            );
        });
    }

    /// Same reasoning for a worktree that is not there: nothing to approve.
    #[gpui::test]
    async fn an_unknown_lane_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
        let _fixture = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            crate::telegram::global::install_for_test(false, None, cx);
            let dispatch = run_gated(
                GatedCommand::ChatNew {
                    lane: LaneHandle {
                        workspace: daruda_store::project::WorkspaceUuid::new(),
                        project: 9_999,
                        lane: 9_999,
                    },
                    agent: None,
                },
                cx,
            );
            match dispatch {
                Dispatch::Ready(outcome) => assert_eq!(outcome, Err(ControlError::TargetGone)),
                Dispatch::Deferred { .. } => panic!("a missing worktree must not ask"),
            }
            assert_eq!(crate::control::approval::waiting_count_for_test(cx), 0);
        });
    }

    /// A command the guards allow does reach the user — and waits.
    #[gpui::test]
    async fn an_allowed_command_asks_and_waits(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let lane = fixture.workspace.read_with(cx, |ws, _| {
            LaneHandle::new(ws.uuid(), ws.control_active_lane())
        });
        let dispatch = cx.update(|cx| paired_bridge_run_gated(lane, cx));
        let rx = match dispatch {
            Dispatch::Deferred { outcome, .. } => outcome,
            Dispatch::Ready(outcome) => panic!("expected a wait, got {outcome:?}"),
        };
        cx.update(|cx| {
            assert_eq!(
                crate::control::approval::waiting_count_for_test(cx),
                1,
                "the user is being asked"
            );
        });
        assert!(rx.is_empty(), "nothing is answered until they decide");
    }

    /// A refusal is the answer, not a silent nothing — the tool call is
    /// waiting on it.
    #[gpui::test]
    async fn a_refused_command_reports_the_refusal(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let lane = fixture.workspace.read_with(cx, |ws, _| {
            LaneHandle::new(ws.uuid(), ws.control_active_lane())
        });
        let dispatch = cx.update(|cx| paired_bridge_run_gated(lane, cx));
        let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
            panic!("expected a wait");
        };
        cx.update(|cx| {
            crate::control::approval::resolve_only_pending_for_test(
                crate::control::approval::ApprovalChoice::Refused,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            rx.recv().await,
            Ok(Some(Err(ControlError::ApprovalRefused)))
        );
    }

    #[gpui::test]
    async fn say_to_an_unknown_workspace_is_target_gone(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            let target = PaneRef {
                workspace: daruda_store::project::WorkspaceUuid::new(),
                pane: 1,
            };
            assert_eq!(
                run(
                    ResolvedCommand::Say {
                        target,
                        text: "hi".into()
                    },
                    cx
                ),
                Err(ControlError::TargetGone)
            );
        });
    }
}
