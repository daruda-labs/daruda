//! Commands that wait on the user's approval: the guards before the card,
//! the work once approved, and the task starts and worktree creation it runs.

use gpui::App;

use crate::agent::tasks_global::GlobalTasks;
use crate::control::approval::ApprovalOutcome;
use crate::control::result::{ControlError, ControlOutcome, ControlResult, LaneHandle};
use crate::control::spec::GatedCommand;
use crate::surface::strings as s;
use crate::telegram::bridge::PaneRef;

use super::{Dispatch, Pending, in_lane_window, in_pane_window, in_project_window, in_task_window};

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
        GatedCommand::TaskCreate {
            workspace,
            project,
            title,
            ..
        } => {
            if title.trim().is_empty() {
                return Err(ControlError::TaskTitleEmpty);
            }
            in_project_window(*workspace, *project, cx, |_ws, _window, _cx| Ok(()))
        }
        // Only a start that creates a worktree spends the budget, as
        // `daruda_worktree_create` does; an existing one is already there.
        GatedCommand::TaskStart { task } => {
            let tasks = cx.global::<GlobalTasks>();
            let found = tasks.get(task).ok_or(ControlError::TaskNotFound)?;
            if !matches!(found.state, daruda_store::tasks::TaskState::Backlog) {
                return Err(ControlError::TaskNotBacklog);
            }
            let creates_lane = task_creates_lane(found);
            in_task_window(task, cx, |_ws, _window, _cx| Ok(()))?;
            if creates_lane {
                crate::control::guards::guard_agent_budget(cx)?;
            }
            Ok(())
        }
        GatedCommand::TaskOpen { task } => in_task_window(task, cx, |_ws, _window, _cx| Ok(())),
    }
}

fn task_creates_lane(task: &daruda_store::tasks::Task) -> bool {
    matches!(task.run_in, daruda_store::tasks::TaskRunIn::NewWorktree)
}

/// A task's title, for the sentence that asks about it.
fn task_title(task: &str, cx: &App) -> String {
    cx.global::<GlobalTasks>()
        .get(task)
        .map(|t| t.title.clone())
        .unwrap_or_default()
}

/// One line naming what the user is being asked to allow. Built here rather
/// than on `GatedCommand`, which is the contract every adapter shares and
/// deliberately carries no rendered sentences.
fn approval_summary(cmd: &GatedCommand, cx: &mut App) -> String {
    match cmd {
        GatedCommand::LaneCreate { name, .. } => s::control::approval_lane_create(name),
        GatedCommand::ChatNew { lane, .. } => {
            let path = in_lane_window(*lane, cx, |ws, _window, _cx| {
                Ok(s::control::lane_path(
                    ws.control_project_name(lane.project),
                    ws.control_lane_name(lane.lane_ref()),
                ))
            })
            .unwrap_or_default();
            s::control::approval_chat_new(&path)
        }
        GatedCommand::TaskCreate {
            workspace,
            project,
            title,
            ..
        } => {
            let name = in_project_window(*workspace, *project, cx, |ws, _window, _cx| {
                Ok(ws.control_project_name(*project))
            })
            .unwrap_or_default();
            s::control::approval_task_create(title.trim(), &name)
        }
        GatedCommand::TaskStart { task } => s::control::approval_task_start(task_title(task, cx)),
        GatedCommand::TaskOpen { task } => s::control::approval_task_open(task_title(task, cx)),
    }
}

/// Do the thing, now that the user has said yes.
async fn perform_gated(cmd: GatedCommand, cx: &mut gpui::AsyncApp) -> ControlOutcome {
    let created = match cmd {
        GatedCommand::TaskCreate {
            workspace,
            project,
            title,
            prompt,
            worktree,
        } => {
            return cx.update(|cx| {
                in_project_window(workspace, project, cx, |ws, _window, cx| {
                    ws.control_task_create(
                        project,
                        &title,
                        prompt.clone(),
                        worktree.map(LaneHandle::lane_ref),
                        cx,
                    )
                })
                .map(|task| ControlResult::TaskCreated { task })
            });
        }
        GatedCommand::TaskStart { task } => return start_task(task, cx).await,
        GatedCommand::TaskOpen { task } => return cx.update(|cx| open_task(task, cx)),
        GatedCommand::ChatNew { lane, agent } => cx
            .update(|cx| {
                in_lane_window(lane, cx, |ws, window, cx| {
                    ws.control_chat_new(lane.lane_ref(), agent.clone(), window, cx)
                })
            })
            .map(|target| (target, None)),
        GatedCommand::LaneCreate {
            workspace,
            project,
            name,
            base_ref,
            agent,
            prompt,
        } => create_lane(workspace, project, name, base_ref, agent, prompt, cx)
            .await
            .map(|(target, chat)| (chat, Some(target))),
    };
    let (chat, lane) = created?;
    // Read back rather than echo the request: an agent id the catalog does
    // not hold resolves to the default, so what opened is the only honest
    // answer. Also what the phone is told — the user approved this a moment
    // ago, and without the ping reaching the new chat takes a `/list` and a
    // `/use`.
    let label = cx.update(|cx| announce_chat(chat, cx));
    let agent = label.map_or_else(String::new, |l| l.agent);
    Ok(match lane {
        Some(target) => ControlResult::LaneCreated {
            target,
            chat,
            agent,
        },
        None => ControlResult::ChatCreated {
            target: chat,
            agent,
        },
    })
}

/// A start for the agent: approved already, so this only waits for it.
async fn start_task(task: String, cx: &mut gpui::AsyncApp) -> ControlOutcome {
    let creates_lane = cx.update(|cx| {
        cx.global::<GlobalTasks>()
            .get(&task)
            .is_some_and(task_creates_lane)
    });
    let outcome = cx.update(|cx| begin_task_start(task, cx))?.finish(cx).await;
    if creates_lane && outcome.is_ok() {
        // Spent only now: a refused or failed start costs the agent nothing.
        cx.update(crate::control::guards::note_agent_created_lane);
    }
    outcome
}

/// A task start under way, in the window that holds the task's project.
pub(crate) struct PendingTaskStart {
    pub(crate) task: String,
    pub(crate) title: String,
    workspace: daruda_store::project::WorkspaceUuid,
    answer: smol::channel::Receiver<crate::workspace::ControlTaskStart>,
}

/// Start `task` and hand back what to wait on. A refusal taken before git
/// runs is already in the answer — see [`PendingTaskStart::try_finish`].
pub(crate) fn begin_task_start(
    task: String,
    cx: &mut App,
) -> Result<PendingTaskStart, ControlError> {
    let title = task_title(&task, cx);
    let (workspace, answer) = in_task_window(&task, cx, |ws, window, cx| {
        Ok((ws.uuid(), ws.control_task_start(&task, window, cx)))
    })?;
    Ok(PendingTaskStart {
        task,
        title,
        workspace,
        answer,
    })
}

impl PendingTaskStart {
    /// The outcome if it is already known: a refusal, or a start that had
    /// no git to wait for.
    pub(crate) fn try_finish(&self, cx: &mut App) -> Option<ControlOutcome> {
        let outcome = self.answer.try_recv().ok()?;
        Some(self.started(outcome, cx))
    }

    pub(crate) async fn finish(self, cx: &mut gpui::AsyncApp) -> ControlOutcome {
        let outcome = self
            .answer
            .recv()
            .await
            .unwrap_or(Err(ControlError::TargetGone));
        cx.update(|cx| self.started(outcome, cx))
    }

    /// Name a started task's chat for the phone, which then talks to it.
    fn started(&self, outcome: crate::workspace::ControlTaskStart, cx: &mut App) -> ControlOutcome {
        let (lane, pane) = outcome?;
        let chat = pane.map(|pane| PaneRef {
            workspace: self.workspace,
            pane,
        });
        if let Some(chat) = chat {
            announce_chat(chat, cx);
        }
        Ok(ControlResult::TaskStarted {
            task: self.task.clone(),
            lane: LaneHandle::new(self.workspace, lane),
            chat,
        })
    }
}

/// Bring up the chat `task` ran in and make it the phone's target.
pub(crate) fn open_task(task: String, cx: &mut App) -> ControlOutcome {
    let chat = in_task_window(&task, cx, |ws, window, cx| {
        let pane = ws.control_task_open(&task, window, cx)?;
        Ok(PaneRef {
            workspace: ws.uuid(),
            pane,
        })
    })?;
    announce_chat(chat, cx);
    Ok(ControlResult::TaskOpened { task, chat })
}

/// Name the new chat for the phone and point the phone at it, handing the
/// label back for the tool result. Announcing is best effort — see
/// `remote_channel::announce_chat_created` — and a pane that vanished in the
/// same tick has no label, so there is nothing to announce or report.
fn announce_chat(pane: PaneRef, cx: &mut App) -> Option<crate::workspace::ChatLabel> {
    let label = in_pane_window(pane, cx, |ws, _window, cx| {
        ws.control_chat_label(pane.pane, cx)
            .ok_or(ControlError::TargetGone)
    })
    .ok()?;
    crate::remote_channel::announce_chat_created(pane, label.path.clone(), &label.agent_name, cx);
    Some(label)
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
) -> Result<(LaneHandle, PaneRef), ControlError> {
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
    Ok((target, chat))
}
