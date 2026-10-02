//! Task execution ownership and reopening its exact conversation.

use daruda_store::accounts::AccountSelection;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneRef, PaneCwd};
use daruda_store::tasks::{ExecutionRef, TaskAgentSurface, TaskExecution};
use gpui::{BorrowAppContext as _, Context, Window};

use crate::agent::tasks_global::GlobalTasks;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{AgentChatContent, Pane, TabEntry};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

fn matches_session(chat: &AgentChatContent, execution: &TaskExecution, cx: &gpui::App) -> bool {
    execution.session_id.is_some()
        && chat.view.read(cx).session_id == execution.session_id
        && chat.agent_id == execution.agent_id
        && chat.account.to_persisted() == execution.account_id
        && chat
            .cwd
            .as_ref()
            .and_then(PaneCwd::as_local)
            .is_some_and(|cwd| daruda_core::path::same_path(cwd, &execution.cwd))
}

impl Workspace {
    pub(in crate::workspace) fn bind_task_chat_execution(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self
            .main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| rt.panes.iter_mut())
            .find(|pane| pane.id == pane_id)
            .and_then(Pane::agent_chat_content_mut)
        else {
            return;
        };
        let Some(cwd) = chat
            .cwd
            .as_ref()
            .and_then(PaneCwd::as_local)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let mut execution = TaskExecution::begin(
            Default::default(),
            chat.agent_id.clone(),
            chat.account.to_persisted(),
            cwd,
        );
        execution.session_id = chat.view.read(cx).session_id.clone();
        chat.task_run = Some(ExecutionRef {
            task_id: task_id.to_string(),
            execution_id: execution.id.clone(),
        });
        cx.update_global::<GlobalTasks, _>(|tasks, _| {
            if let Some(task) = tasks.get_mut(task_id) {
                task.execution = Some(execution);
            }
        });
        self.save_tasks_dirty(cx);
    }

    pub(in crate::workspace) fn task_chat_owner(
        &self,
        pane_id: PaneId,
        cx: &gpui::App,
    ) -> Option<String> {
        let chat = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|pane| pane.id == pane_id)?
            .agent_chat_content()?;
        let run = chat.task_run.as_ref()?;
        let tasks = cx.global::<GlobalTasks>();
        let task = tasks.get(&run.task_id)?;
        let execution = run.resolve(tasks)?;
        (task.agent_surface == TaskAgentSurface::AgentChat
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
                .is_none_or(|id| chat.view.read(cx).session_id.as_ref() == Some(id)))
        .then(|| run.task_id.clone())
    }

    pub(in crate::workspace) fn cancel_task_chat_execution(
        &mut self,
        task_id: &str,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .find_map(|pane| {
                (self.task_chat_owner(pane.id, cx).as_deref() == Some(task_id)).then_some(pane.id)
            });
        let Some(pane_id) = pane_id else {
            return;
        };
        self.cancel_agent_turn(pane_id, cx);
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |view, cx| view.clear_queue(cx));
        }
    }

    pub(in crate::workspace) fn record_task_chat_session(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.task_chat_owner(pane_id, cx) else {
            return;
        };
        let Some(session_id) = self
            .agent_chat_view(pane_id)
            .and_then(|view| view.read(cx).session_id.clone())
        else {
            return;
        };
        let changed = cx.update_global::<GlobalTasks, _>(|tasks, _| {
            let Some(execution) = tasks.get_mut(&id).and_then(|task| task.execution.as_mut())
            else {
                return false;
            };
            if execution.session_id.is_some() {
                return false;
            }
            execution.session_id = Some(session_id);
            true
        });
        if changed {
            self.save_tasks_dirty(cx);
        }
    }

    pub(in crate::workspace) fn is_task_chat_restore(
        &self,
        pane_id: PaneId,
        cx: &gpui::App,
    ) -> bool {
        let Some(chat) = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|pane| pane.id == pane_id)
            .and_then(Pane::agent_chat_content)
        else {
            return false;
        };
        cx.global::<GlobalTasks>().tasks.iter().any(|task| {
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
        let Some(agent) = self.agents.iter().find(|agent| agent.id == agent_id) else {
            return false;
        };
        account_id.is_none_or(|id| {
            self.accounts
                .find(id)
                .is_some_and(|account| agent.launch.account_recipe(false) == Some(account.recipe))
        })
    }

    pub(in crate::workspace) fn open_task_chat(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx.global::<GlobalTasks>().get(task_id).cloned() else {
            return;
        };
        let Some(execution) = task.execution else {
            self.task_chat_error(s::task::chat_missing_session(), cx);
            return;
        };
        let existing = self.main_area.runtimes.iter().find_map(|(lane, rt)| {
            rt.panes.iter().find_map(|pane| {
                let chat = pane.agent_chat_content()?;
                let owned = self.task_chat_owner(pane.id, cx).as_deref() == Some(task_id);
                (owned || matches_session(chat, &execution, cx)).then_some((*lane, pane.id))
            })
        });
        if let Some((lane, pane)) = existing {
            self.reveal_task_chat(lane, pane, window, cx);
            return;
        }
        let Some(session_id) = execution.session_id else {
            self.task_chat_error(s::task::chat_missing_session(), cx);
            return;
        };
        if !self.task_chat_identity_available(&execution.agent_id, execution.account_id) {
            self.task_chat_error(s::task::chat_missing_agent(), cx);
            return;
        }
        let lane = self.projects.iter().find_map(|project| {
            project
                .lanes
                .iter()
                .find(|lane| daruda_core::path::same_path(&lane.path, &execution.cwd))
                .map(|lane| LaneRef {
                    project: project.id,
                    lane: lane.id,
                })
        });
        let Some(lane) = lane.filter(|_| execution.cwd.is_dir()) else {
            self.task_chat_error(s::task::chat_missing_worktree(), cx);
            return;
        };
        self.activate_lane(lane, window, cx);
        let mut pane = self.create_agent_chat_pane(
            Some(PaneCwd::Local(execution.cwd)),
            Some(session_id),
            execution.agent_id,
            Some(task.title),
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
