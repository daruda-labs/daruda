//! Pane-owned CLI execution identity and read-only chat handoff.

use chrono::Utc;
use daruda_store::accounts::AccountRecipeId;
use daruda_store::tasks::{
    CliProcessState, ExecutionRef, Task, TaskExecution, TaskExecutionSource, TaskState,
};
use gpui::{BorrowAppContext as _, Context};
use std::path::Path;

use crate::agent::tasks_global::GlobalTasks;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::LoadIntent;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_agent::pty_tracker::PtyBinding;

impl Workspace {
    pub(super) fn bind_task_cli_execution(
        &mut self,
        task_id: &str,
        pane_id: PaneId,
        cwd: &Path,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self
            .main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| &mut rt.panes)
            .find(|pane| pane.id == pane_id)
        else {
            return;
        };
        let PaneContent::Terminal(terminal) = &mut pane.content else {
            return;
        };
        let account_id = terminal.account.to_persisted().filter(|id| {
            self.accounts
                .find(*id)
                .is_some_and(|a| a.recipe == AccountRecipeId::Claude)
        });
        let agent_id = self
            .agents
            .iter()
            .find(|agent| agent.launch.account_recipe(false) == Some(AccountRecipeId::Claude))
            .map(|agent| agent.id.clone())
            .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id);
        let execution = TaskExecution::begin(
            TaskExecutionSource::ClaudeCli {
                transcript_path: None,
                process: CliProcessState::Discovering,
            },
            agent_id,
            account_id,
            cwd.to_path_buf(),
        );
        terminal.task_run = Some(ExecutionRef {
            task_id: task_id.to_string(),
            execution_id: execution.id.clone(),
        });
        let recorded = cx.update_global::<GlobalTasks, _>(|tasks, _| {
            let Some(task) = tasks.get_mut(task_id) else {
                return false;
            };
            task.execution = Some(execution);
            true
        });
        // A task deleted under its own start has no run to discover.
        if !recorded {
            return;
        }
        if let Some(dir) = self.cli_sessions_dir(account_id) {
            self.claude.pty_tracker.track_task(pane_id, dir);
        }
        self.save_tasks_dirty(cx);
    }

    /// Where a `claude` run under `account_id` writes its per-PID session
    /// files: the account's config home, or the ambient `~/.claude`.
    fn cli_sessions_dir(
        &self,
        account_id: Option<daruda_store::accounts::AccountId>,
    ) -> Option<std::path::PathBuf> {
        let home = match account_id {
            Some(id) => Some(daruda_agent::accounts::account_config_dir(
                &self.data_dir,
                id,
            )),
            None => daruda_agent::accounts::recipe_for(AccountRecipeId::Claude).system_home_dir(),
        }?;
        Some(daruda_agent::pty_link::sessions_dir_in(&home))
    }

    /// Apply one observation to every CLI run it concerns and persist when any
    /// changed. `apply` sees the task's state, its session list and the run,
    /// and answers whether it changed them.
    fn update_cli_runs(
        &mut self,
        cx: &mut Context<Self>,
        concerns: impl Fn(&TaskExecution) -> bool,
        mut apply: impl FnMut(&TaskState, &mut Vec<String>, &mut TaskExecution) -> bool,
    ) {
        let dirty = cx.update_global::<GlobalTasks, _>(|tasks, _| {
            let mut dirty = false;
            for task in &mut tasks.tasks {
                let Task {
                    state,
                    session_ids,
                    execution,
                    updated_at,
                    ..
                } = task;
                let Some(run) = execution
                    .as_mut()
                    .filter(|run| run.source.cli_process().is_some() && concerns(run))
                else {
                    continue;
                };
                if apply(state, session_ids, run) {
                    *updated_at = Utc::now();
                    dirty = true;
                }
            }
            dirty
        });
        if dirty {
            self.save_tasks_dirty(cx);
        }
    }

    /// Re-arm exit tracking for runs a previous launch saw running, and give
    /// up on runs that never bound — their pane did not survive the restart.
    pub(in crate::workspace) fn restore_task_cli_tracking(&mut self, cx: &mut Context<Self>) {
        let runs: Vec<_> = cx
            .global::<GlobalTasks>()
            .tasks
            .iter()
            .filter_map(|task| task.execution.clone())
            .collect();
        for run in runs {
            if let (Some(CliProcessState::Running { pid }), Some(session)) =
                (run.source.cli_process(), &run.session_id)
                && let Some(dir) = self.cli_sessions_dir(run.account_id)
            {
                self.claude
                    .pty_tracker
                    .track_session(session.clone(), *pid, dir);
            }
        }
        self.update_cli_runs(
            cx,
            |_| true,
            |_, _, run| {
                run.source
                    .cli_mut()
                    .is_some_and(|(_, process)| process.orphan_discovery())
            },
        );
    }

    /// Record what a PTY binding proves. In the owning terminal it names the
    /// run's session, once; anywhere, a binding for a run's session means a
    /// process is writing it, which `CliProcessState::observe_running` folds
    /// in — a confirmed exit included, since `claude --resume` keeps the id.
    pub(in crate::workspace) fn record_task_cli_binding(
        &mut self,
        pane_id: PaneId,
        binding: &PtyBinding,
        cx: &mut Context<Self>,
    ) {
        let owner = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| &rt.panes)
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| match &pane.content {
                PaneContent::Terminal(terminal) => terminal.task_run.clone(),
                _ => None,
            });
        if let Some(owner) = owner {
            let session = binding.session_id.clone();
            self.update_cli_runs(
                cx,
                move |run| run.id == owner.execution_id && run.session_id.is_none(),
                |state, session_ids, run| {
                    // The first session in the owning terminal is the run's; a
                    // later `claude` there never takes it over.
                    run.session_id = Some(session.clone());
                    if matches!(state, TaskState::Running { .. }) && !session_ids.contains(&session)
                    {
                        session_ids.push(session.clone());
                    }
                    true
                },
            );
        }
        let transcript = self
            .claude
            .claude_status
            .get(&binding.session_id)
            .and_then(|file| file.transcript_path.clone());
        let session = binding.session_id.clone();
        let pid = binding.claude_pid;
        self.update_cli_runs(
            cx,
            move |run| run.session_id.as_deref() == Some(session.as_str()),
            |_, _, run| {
                let Some((transcript_path, process)) = run.source.cli_mut() else {
                    return false;
                };
                if !process.observe_running(pid) {
                    return false;
                }
                if transcript.is_some() {
                    transcript_path.clone_from(&transcript);
                }
                true
            },
        );
    }

    pub(in crate::workspace) fn record_task_cli_transcript(
        &mut self,
        session_id: &str,
        path: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else { return };
        let session = session_id.to_string();
        self.update_cli_runs(
            cx,
            move |run| run.session_id.as_deref() == Some(session.as_str()),
            |_, _, run| {
                let Some((transcript_path, _)) = run.source.cli_mut() else {
                    return false;
                };
                if transcript_path.as_deref() == Some(path) {
                    return false;
                }
                *transcript_path = Some(path.to_path_buf());
                true
            },
        );
    }

    pub(in crate::workspace) fn record_task_cli_exit(
        &mut self,
        session_id: &str,
        pid: u32,
        cx: &mut Context<Self>,
    ) {
        let session = session_id.to_string();
        let now = Utc::now();
        self.update_cli_runs(
            cx,
            move |run| run.session_id.as_deref() == Some(session.as_str()),
            |_, _, run| {
                run.source
                    .cli_mut()
                    .is_some_and(|(_, process)| process.confirm_exit(pid, now))
            },
        );
    }

    /// Whether anything still shows a process writing `session_id`: a pane
    /// bound to it, or a process the tracker has not seen exit.
    fn cli_session_live(&self, session_id: &str) -> bool {
        self.claude
            .pty_claude_bindings
            .values()
            .any(|binding| binding.session_id == session_id)
            || self.claude.pty_tracker.is_running(session_id)
    }

    /// Continuing is safe only once the run's exit is confirmed and nothing
    /// has since resumed its session.
    fn cli_run_may_continue(&self, run: &ExecutionRef, session: &str, cx: &gpui::App) -> bool {
        matches!(
            self.cli_run_process(run, cx),
            Some(CliProcessState::ExitConfirmed { .. })
        ) && !self.cli_session_live(session)
    }

    /// The process record of the run a pane mirrors; `None` once that run
    /// is gone (task deleted or re-run).
    pub(in crate::workspace) fn cli_run_process(
        &self,
        run: &ExecutionRef,
        cx: &gpui::App,
    ) -> Option<CliProcessState> {
        run.resolve(cx.global::<GlobalTasks>())?
            .source
            .cli_process()
            .cloned()
    }

    pub(in crate::workspace) fn refresh_cli_chat(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.load_cli_chat(pane_id, LoadIntent::Snapshot, cx);
    }

    pub(in crate::workspace) fn continue_cli_chat(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.load_cli_chat(pane_id, LoadIntent::Continue, cx);
    }

    fn load_cli_chat(&mut self, pane_id: PaneId, intent: LoadIntent, cx: &mut Context<Self>) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let v = view.read(cx);
        let Some(run) = v.mirrored_run() else { return };
        if v.status().is_connecting() {
            return;
        }
        let Some(session) = v.session_id().map(str::to_owned) else {
            return;
        };
        if intent == LoadIntent::Continue && !self.cli_run_may_continue(run, &session, cx) {
            self.task_chat_error(s::task::cli_not_exited(), cx);
            return;
        }
        let Some(cwd) = v.cwd().cloned() else { return };
        // A user-asked reload is also a moment to re-check the process.
        self.claude.pty_tracker.poke();
        view.update(cx, |v, cx| {
            v.set_load_intent(intent);
            v.retry_for_reconnect(Some(session.clone()), cx);
        });
        // The CLI status row reads the pane's loading state.
        cx.notify();
        self.connect_agent_chat(pane_id, cwd, Some(session), cx);
    }

    /// Settle a mirror's load at `Connected`, which arrives only after the
    /// complete required `session/load` replay. A continue whose run is still
    /// safe to take over turns interactive; anything else lets go of the
    /// session so the pane stays a snapshot.
    pub(in crate::workspace) fn finish_cli_chat_load(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        let v = view.read(cx);
        let Some(run) = v.mirrored_run() else { return };
        let allow = v.load_intent() == Some(LoadIntent::Continue)
            && v.session_id()
                .is_some_and(|session| self.cli_run_may_continue(run, session, cx));
        view.update(cx, |v, cx| {
            if allow {
                v.make_interactive();
            } else {
                v.detach_handle();
                v.settle_mirror_intent();
            }
            cx.notify();
        });
        self.mutate_durable(cx, |_, _| {});
    }

    /// Open the seeded transcript as the read-only mirror of a CLI task run,
    /// its process still `Running` or already `ExitConfirmed`. The task lives
    /// only in memory; the status row under the chat is what the shot judges.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_cli_snapshot_for_shot(
        &mut self,
        exited: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self
            .active_lane()
            .map(|lane| lane.path.clone())
            .unwrap_or_else(std::env::temp_dir);
        let project = self.active_project().map(|p| p.uuid).unwrap_or_default();
        let mut task = daruda_store::tasks::Task::new(
            project,
            "CLI task".into(),
            "Fix the parser".into(),
            None,
        );
        let process = if exited {
            CliProcessState::ExitConfirmed {
                ended_at: Utc::now(),
            }
        } else {
            CliProcessState::Running {
                pid: std::process::id(),
            }
        };
        let mut execution = TaskExecution::begin(
            TaskExecutionSource::ClaudeCli {
                transcript_path: None,
                process,
            },
            self.agents[0].id.clone(),
            None,
            cwd.clone(),
        );
        execution.session_id = Some("screenshot-cli-session".into());
        task.agent_surface = daruda_store::tasks::TaskAgentSurface::Terminal;
        task.state = TaskState::Running { worktree_path: cwd };
        let access = daruda_store::tasks::AgentChatAccess::CliSnapshot(ExecutionRef {
            task_id: task.id.clone(),
            execution_id: execution.id.clone(),
        });
        task.execution = Some(execution);
        cx.update_global::<GlobalTasks, _>(|tasks, _| {
            tasks.add(task);
        });
        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| {
                v.seed_transcript(
                    crate::workspace::main_area::agent_chat_pane::shot_transcript::sample_transcript(),
                    window,
                    cx,
                );
                v.session_id = Some("screenshot-cli-session".into());
                v.set_access(access);
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
#[path = "task_cli_tests.rs"]
mod tests;
