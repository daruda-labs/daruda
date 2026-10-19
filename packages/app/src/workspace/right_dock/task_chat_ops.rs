//! Task execution ownership and reopening its exact conversation.

use daruda_store::accounts::AccountSelection;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneRef, PaneCwd};
use daruda_store::tasks::{ExecutionRef, TaskEvent, TaskExecution, TaskExecutionSource};
use gpui::{Context, Window};

use super::task_start::PaneIdentity;
use crate::runs::{GlobalTasks, RunsHost};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{AgentChatContent, Pane, TabEntry};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

fn matches_session(chat: &AgentChatContent, execution: &TaskExecution, cx: &gpui::App) -> bool {
    execution.session_id.is_some()
        && chat.view.read(cx).session_id() == execution.session_id.as_deref()
        && chat.agent_id == execution.agent_id
        && chat.account.to_persisted() == execution.account_id
        && chat
            .cwd
            .as_ref()
            .and_then(PaneCwd::as_local)
            .is_some_and(|cwd| daruda_core::path::same_path(cwd, &execution.cwd))
}

/// Why a task's chat could not be brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskChatError {
    NotFound,
    /// The task has not run, or its run has not reported a session yet.
    NoSession,
    /// The agent or account the run used is no longer configured.
    AgentUnavailable,
    /// The worktree the run happened in is gone.
    WorktreeMissing,
}

impl TaskChatError {
    /// What the desktop says; `None` for a task the Tasks UI cannot name.
    fn message(self) -> Option<String> {
        match self {
            Self::NotFound => None,
            Self::NoSession => Some(s::task::chat_missing_session()),
            Self::AgentUnavailable => Some(s::task::chat_missing_agent()),
            Self::WorktreeMissing => Some(s::task::chat_missing_worktree()),
        }
    }
}

impl Workspace {
    /// Make `pane_id` the chat that runs `exec`, and read the identity the
    /// run records. `None` when the pane cannot run it: not a chat, or its
    /// directory is not local — a remote lane is refused at dispatch rather
    /// than left with a run no local pane can settle.
    pub(super) fn bind_task_chat_execution(
        &mut self,
        exec: &ExecutionRef,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<PaneIdentity> {
        let chat = self
            .main_area
            .pane_mut(pane_id)
            .and_then(Pane::agent_chat_content_mut)?;
        let cwd = chat
            .cwd
            .as_ref()
            .and_then(PaneCwd::as_local)
            .map(std::path::Path::to_path_buf)?;
        // A fresh pane has no session yet; `record_task_chat_session` names
        // the run's once the pane connects.
        let identity = PaneIdentity {
            agent_id: chat.agent_id.clone(),
            account_id: chat.account.to_persisted(),
            cwd,
        };
        RunsHost::bind_execution(self.run_pane(pane_id), exec.clone(), cx);
        Some(identity)
    }

    /// Run task `id` in chat `pane` the way a start does: request the run,
    /// bind the pane, record its identity, and dispatch. The pane's session,
    /// if it has one, is the run's.
    #[cfg(test)]
    pub(in crate::workspace) fn run_task_in_chat_for_test(
        &mut self,
        id: &str,
        pane: PaneId,
        cx: &mut Context<Self>,
    ) -> ExecutionRef {
        let exec = ExecutionRef {
            task_id: id.to_owned(),
            execution_id: uuid::Uuid::new_v4().to_string(),
        };
        let at = chrono::Utc::now();
        crate::runs::seed::commit_now(
            vec![TaskEvent::DispatchRequested {
                started_by: None,
                id: id.to_owned(),
                execution_id: exec.execution_id.clone(),
                source: daruda_store::tasks::TaskExecutionSource::AgentChat,
                at,
            }],
            cx,
        );
        let identity = self
            .bind_task_chat_execution(&exec, pane, cx)
            .expect("a local chat pane takes the run");
        let session_id = self
            .agent_chat_view(pane)
            .and_then(|view| view.read(cx).session_id().map(str::to_owned));
        crate::runs::seed::commit_now(
            vec![
                TaskEvent::LanePrepared {
                    exec: exec.clone(),
                    worktree: identity.cwd.clone(),
                    agent_id: identity.agent_id,
                    account_id: identity.account_id,
                    cwd: identity.cwd,
                    at,
                },
                TaskEvent::Dispatched {
                    exec: exec.clone(),
                    session_id,
                    at,
                },
            ],
            cx,
        );
        exec
    }

    pub(in crate::workspace) fn run_pane(&self, pane: PaneId) -> crate::runs::RunPane {
        crate::runs::RunPane {
            workspace: self.uuid(),
            pane,
        }
    }

    pub(in crate::workspace) fn task_chat_execution(
        &self,
        pane_id: PaneId,
        cx: &gpui::App,
    ) -> Option<ExecutionRef> {
        let chat = self.main_area.pane(pane_id)?.agent_chat_content()?;
        let run = RunsHost::pane_execution(self.run_pane(pane_id), cx)?;
        let tasks = cx.global::<GlobalTasks>();
        let execution = run.resolve(tasks.book())?;
        (matches!(execution.source, TaskExecutionSource::AgentChat)
            && chat.agent_id == execution.agent_id
            && chat.account.to_persisted() == execution.account_id
            && chat
                .cwd
                .as_ref()
                .and_then(PaneCwd::as_local)
                .is_some_and(|cwd| daruda_core::path::same_path(cwd, &execution.cwd))
            && execution
                .session_id
                .as_ref()
                .is_none_or(|id| chat.view.read(cx).session_id() == Some(id.as_str())))
        .then(|| run.clone())
    }

    pub(in crate::workspace) fn record_task_chat_session(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(exec) = self.task_chat_execution(pane_id, cx) else {
            return;
        };
        let Some(session_id) = self
            .agent_chat_view(pane_id)
            .and_then(|view| view.read(cx).session_id().map(str::to_owned))
        else {
            return;
        };
        let Some(task) = cx.global::<GlobalTasks>().get(&exec.task_id) else {
            return;
        };
        let unbound = task
            .execution
            .as_ref()
            .is_some_and(|execution| execution.session_id.is_none());
        let Some(exec) = task.current_exec().cloned().filter(|_| unbound) else {
            return;
        };
        RunsHost::propose_fact(
            TaskEvent::ExecutionSessionBound {
                exec,
                session_id,
                at: chrono::Utc::now(),
            },
            cx,
        );
    }

    pub(in crate::workspace) fn is_task_chat_restore(
        &self,
        pane_id: PaneId,
        cx: &gpui::App,
    ) -> bool {
        let Some(chat) = self
            .main_area
            .pane(pane_id)
            .and_then(Pane::agent_chat_content)
        else {
            return false;
        };
        cx.global::<GlobalTasks>().records().any(|task| {
            task.execution
                .as_ref()
                .is_some_and(|execution| matches_session(chat, execution, cx))
        })
    }

    pub(super) fn task_chat_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(message)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("task.chat.open")
                .build(),
            cx,
        );
    }

    pub(in crate::workspace) fn task_chat_identity_available(
        &self,
        agent_id: &str,
        account_id: Option<daruda_store::accounts::AccountId>,
    ) -> bool {
        let Some(agent) = self
            .mirrors
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
        else {
            return false;
        };
        account_id.is_none_or(|id| {
            self.accounts
                .find(id)
                .is_some_and(|account| agent.launch.account_recipe(false) == Some(account.recipe))
        })
    }

    /// Show the chat a task's run belongs to, from the Tasks UI. A failure
    /// raises its toast here.
    pub(in crate::workspace) fn open_task_chat(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.begin_task_chat_open(task_id, window, cx)
            && let Some(message) = e.message()
        {
            self.task_chat_error(message, cx);
        }
    }

    /// Bring up the chat a task's run belongs to: the pane already showing
    /// it, else a new one that loads that exact session.
    pub(crate) fn begin_task_chat_open(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, TaskChatError> {
        let task = cx
            .global::<GlobalTasks>()
            .get(task_id)
            .cloned()
            .ok_or(TaskChatError::NotFound)?;
        let execution = task.execution.ok_or(TaskChatError::NoSession)?;
        let title = task.spec.title;
        let existing = self.main_area.runtimes.iter().find_map(|(lane, rt)| {
            rt.panes.iter().find_map(|pane| {
                let chat = pane.agent_chat_content()?;
                let owned = self
                    .task_chat_execution(pane.id, cx)
                    .is_some_and(|exec| exec.task_id == task_id);
                (owned || matches_session(chat, &execution, cx)).then_some((*lane, pane.id))
            })
        });
        if let Some((lane, pane)) = existing {
            self.reveal_task_chat(lane, pane, window, cx);
            return Ok(pane);
        }
        let session_id = execution.session_id.ok_or(TaskChatError::NoSession)?;
        if !self.task_chat_identity_available(&execution.agent_id, execution.account_id) {
            return Err(TaskChatError::AgentUnavailable);
        }
        let lane = self
            .projects
            .lanes()
            .find(|(_, _, lane)| daruda_core::path::same_path(&lane.path, &execution.cwd))
            .map(|(target, _, _)| target);
        let lane = lane
            .filter(|_| execution.cwd.is_dir())
            .ok_or(TaskChatError::WorktreeMissing)?;
        self.activate_lane(lane, window, cx);
        let mut pane = self.create_agent_chat_pane(
            Some(PaneCwd::Local(execution.cwd)),
            Some(session_id),
            execution.agent_id,
            Some(title),
            window,
            cx,
        );
        if let Some(chat) = pane.agent_chat_content_mut() {
            chat.account = AccountSelection::from_persisted(execution.account_id);
            if execution.source.cli_process().is_some() {
                chat.view.update(cx, |view, _| {
                    view.set_access(daruda_store::tasks::AgentChatAccess::CliSnapshot(
                        ExecutionRef {
                            task_id: task_id.to_string(),
                            execution_id: execution.id,
                        },
                    ));
                });
            }
        }
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut().tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        self.reveal_task_chat(lane, pane_id, window, cx);
        self.mutate_durable(cx, |_, _| {});
        Ok(pane_id)
    }

    fn reveal_task_chat(
        &mut self,
        lane: LaneRef,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_lane(lane, window, cx);
        if let Some(tab) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(pane))
        {
            self.activate_tab(tab, window, cx);
        }
        self.set_focused_pane(pane, window, cx);
        self.focus_pane(pane, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }
}

#[cfg(test)]
#[path = "task_chat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_delivery_tests.rs"]
mod delivery_tests;
