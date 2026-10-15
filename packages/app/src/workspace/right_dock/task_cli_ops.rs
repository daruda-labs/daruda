//! CLI pane identity and read-only chat handoff; RunsHost owns tracking.

#[cfg(any(test, feature = "screenshot"))]
use chrono::Utc;
use daruda_store::accounts::AccountRecipeId;
use daruda_store::tasks::{CliProcessState, ExecutionRef};
use gpui::Context;
use std::path::Path;

use super::task_start::PaneIdentity;
use crate::runs::{GlobalTasks, RunsHost};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;
#[cfg(test)]
use daruda_agent::pty_tracker::PtyBinding;
use daruda_chat::view::LoadIntent;

impl Workspace {
    /// Make terminal `pane_id` the one that runs `exec`, and read the
    /// identity the run records. `None` when the pane is not a terminal.
    pub(super) fn bind_task_cli_execution(
        &mut self,
        exec: &ExecutionRef,
        pane_id: PaneId,
        cwd: &Path,
        cx: &mut Context<Self>,
    ) -> Option<PaneIdentity> {
        let pane = self
            .main_area
            .runtimes
            .values_mut()
            .flat_map(|rt| &mut rt.panes)
            .find(|pane| pane.id == pane_id)?;
        let PaneContent::Terminal(terminal) = &mut pane.content else {
            return None;
        };
        let account_id = terminal.account.to_persisted().filter(|id| {
            self.accounts
                .find(*id)
                .is_some_and(|a| a.recipe == AccountRecipeId::Claude)
        });
        let agent_id = self
            .mirrors
            .agents
            .iter()
            .find(|agent| agent.launch.account_recipe(false) == Some(AccountRecipeId::Claude))
            .map(|agent| agent.id.clone())
            .unwrap_or_else(|| daruda_config::AgentDefinition::claude_default().id);
        RunsHost::bind_execution(self.run_pane(pane_id), exec.clone(), cx);
        Some(PaneIdentity {
            agent_id,
            account_id,
            cwd: cwd.to_path_buf(),
        })
    }

    /// Watch for the CLI in this terminal, using the process-wide tracker.
    pub(super) fn track_task_cli_execution(
        &mut self,
        pane_id: PaneId,
        account_id: Option<daruda_store::accounts::AccountId>,
        cx: &mut Context<Self>,
    ) {
        if let Some(dir) = crate::runs::cli_sessions_dir(&self.data_dir, account_id) {
            RunsHost::track_cli(self.run_pane(pane_id), dir, cx);
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn restore_task_cli_tracking(&mut self, cx: &mut Context<Self>) {
        RunsHost::restore_cli_tracking(&self.data_dir, cx);
    }

    #[cfg(test)]
    pub(in crate::workspace) fn record_task_cli_binding(
        &mut self,
        pane_id: PaneId,
        binding: &PtyBinding,
        cx: &mut Context<Self>,
    ) {
        let transcript = self
            .claude_sessions
            .status
            .get(&binding.session_id)
            .and_then(|file| file.transcript_path.clone());
        RunsHost::record_cli_binding(
            Some(self.run_pane(pane_id)),
            binding,
            transcript.as_deref(),
            cx,
        );
    }

    pub(in crate::workspace) fn record_task_cli_transcript(
        &mut self,
        session_id: &str,
        path: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = path {
            RunsHost::record_cli_transcript(session_id, path, cx);
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn record_task_cli_exit(
        &mut self,
        session_id: &str,
        pid: u32,
        cx: &mut Context<Self>,
    ) {
        RunsHost::record_cli_exit(session_id, pid, cx);
    }

    /// Whether anything still shows a process writing `session_id`: a pane
    /// bound to it, or a process the tracker has not seen exit.
    fn cli_session_live(&self, session_id: &str, cx: &gpui::App) -> bool {
        self.claude_sessions
            .pty_bindings
            .values()
            .any(|binding| binding.session_id == session_id)
            || RunsHost::cli_tracker(cx).is_running(session_id)
    }

    /// Continuing is safe only once the run's exit is confirmed and nothing
    /// has since resumed its session.
    fn cli_run_may_continue(&self, run: &ExecutionRef, session: &str, cx: &gpui::App) -> bool {
        matches!(
            self.cli_run_process(run, cx),
            Some(CliProcessState::ExitConfirmed { .. })
        ) && !self.cli_session_live(session, cx)
    }

    /// The process record of the run a pane mirrors; `None` once that run
    /// is gone (task deleted or re-run).
    pub(in crate::workspace) fn cli_run_process(
        &self,
        run: &ExecutionRef,
        cx: &gpui::App,
    ) -> Option<CliProcessState> {
        run.resolve(cx.global::<GlobalTasks>().book())?
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
        RunsHost::cli_tracker(cx).poke();
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
        let mut spec = daruda_store::tasks::TaskSpec::new(
            project,
            "CLI task".into(),
            "Fix the parser".into(),
            None,
            Utc::now(),
        );
        spec.agent_surface = daruda_store::tasks::TaskAgentSurface::Terminal;
        let process = if exited {
            CliProcessState::ExitConfirmed {
                ended_at: Utc::now(),
            }
        } else {
            CliProcessState::Running {
                pid: std::process::id(),
            }
        };
        let run = crate::runs::seed::SeedRun::cli(cwd, process)
            .agent(self.mirrors.agents[0].id.clone())
            .session("screenshot-cli-session");
        let access = daruda_store::tasks::AgentChatAccess::CliSnapshot(run.exec(&spec.id));
        let mut events = crate::runs::seed::created(spec.clone());
        events.extend(crate::runs::seed::run(
            &spec.id,
            &run,
            crate::runs::seed::SeedEnd::Running,
        ));
        crate::runs::seed::commit_now(events, cx);
        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::sample_transcript(),
                    window,
                    cx,
                );
                v.set_session_id_for_shot("screenshot-cli-session");
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
