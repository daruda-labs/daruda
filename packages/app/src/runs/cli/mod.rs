//! Process-owned terminal routes and known CLI process exit observations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use daruda_agent::hooks::route::TerminalId;
use daruda_agent::pty_tracker::{ProcessExited, PtyTracker};
use daruda_store::accounts::{AccountId, AccountRecipeId};
use daruda_store::project::WorkspaceUuid;
use daruda_store::tasks::{CliProcessState, ExecutionRef, TaskEvent, TaskExecution, TaskRecord};
use gpui::{App, BorrowAppContext as _};

use super::{GlobalTasks, RunPane, RunsHost};

pub(in crate::runs) mod resume;
mod session;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct CliTracker {
    tracker: PtyTracker,
    /// The live terminal of each registered pane, by the id its agent hooks
    /// report. One entry per pane: a respawn replaces the old id.
    terminals: HashMap<TerminalId, RunPane>,
    _pump: gpui::Task<()>,
}

fn cli_runs<'a>(
    tasks: &'a GlobalTasks,
    concerns: impl Fn(&TaskExecution) -> bool + 'a,
) -> impl Iterator<Item = (&'a TaskRecord, &'a TaskExecution, ExecutionRef)> + 'a {
    tasks.records().filter_map(move |task| {
        let run = task
            .execution
            .as_ref()
            .filter(|run| run.source.cli_process().is_some() && concerns(run))?;
        let exec = task
            .current_exec()
            .filter(|exec| exec.execution_id == run.id)?;
        Some((task, run, exec.clone()))
    })
}

fn sessions_dir(data_dir: &Path, account: Option<AccountId>) -> Option<PathBuf> {
    let home = match account {
        Some(id) => Some(daruda_agent::accounts::account_config_dir(data_dir, id)),
        None => daruda_agent::accounts::recipe_for(AccountRecipeId::Claude).system_home_dir(),
    }?;
    Some(daruda_agent::pty_link::sessions_dir_in(&home))
}

impl RunsHost {
    pub(crate) fn claim_cli_resume(
        session: String,
        owner: RunPane,
        cx: &mut App,
    ) -> Option<super::CliResumeLease> {
        cx.update_global::<Self, _>(|host, _| host.cli_resumes.claim(session, owner))
    }

    pub(super) fn install_cli_tracker(cx: &mut App) {
        let (tracker, receiver) = PtyTracker::spawn();
        let pump = cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                while let Ok(event) = receiver.try_recv() {
                    let mut event = Some(event);
                    loop {
                        let pending = cx.update(|cx| {
                            if Self::settled(cx) {
                                if let Some(event) = event.take() {
                                    Self::apply_cli_tracker_event(event, cx);
                                }
                                None
                            } else {
                                Some(Self::barrier(cx))
                            }
                        });
                        let Some(pending) = pending else { break };
                        pending.await;
                    }
                }
            }
        });
        cx.update_global::<Self, _>(|host, _| {
            host.cli = Some(CliTracker {
                tracker,
                terminals: HashMap::new(),
                _pump: pump,
            })
        });
        Self::restore_cli_tracking(&daruda_store::persistence::default_data_dir(), cx);
    }

    pub(crate) fn cli_tracker(cx: &App) -> &PtyTracker {
        &cx.global::<Self>()
            .cli
            .as_ref()
            .expect("installed CLI tracker")
            .tracker
    }

    pub(crate) fn register_terminal(pane: RunPane, terminal: TerminalId, cx: &mut App) {
        if Self::terminal_id(pane, cx).is_some_and(|current| current != &terminal) {
            Self::release_execution(
                pane,
                daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
                cx,
            );
        }
        cx.update_global::<Self, _>(|host, _| {
            let cli = host.cli.as_mut().expect("installed CLI tracker");
            cli.terminals.retain(|_, owner| *owner != pane);
            cli.terminals.insert(terminal, pane);
        });
    }

    pub(crate) fn unregister_terminal(pane: RunPane, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| {
            let cli = host.cli.as_mut().expect("installed CLI tracker");
            cli.terminals.retain(|_, owner| *owner != pane);
        });
    }

    pub(crate) fn terminal_id(pane: RunPane, cx: &App) -> Option<&TerminalId> {
        cx.global::<Self>()
            .cli
            .as_ref()?
            .terminals
            .iter()
            .find_map(|(terminal, owner)| (*owner == pane).then_some(terminal))
    }

    /// A replaced view can be released after its successor registered in
    /// the same pane. Only the view still owning the terminal may detach it.
    pub(crate) fn release_terminal(pane: RunPane, terminal: &TerminalId, cx: &mut App) {
        if Self::terminal_pane(terminal, cx) != Some(pane) {
            return;
        }
        Self::release_execution(
            pane,
            daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
            cx,
        );
        Self::unregister_terminal(pane, cx);
    }

    /// The pane whose live terminal is `terminal`; `None` once that shell
    /// was replaced or its pane closed.
    pub(crate) fn terminal_pane(terminal: &TerminalId, cx: &App) -> Option<RunPane> {
        cx.global::<Self>()
            .cli
            .as_ref()?
            .terminals
            .get(terminal)
            .copied()
    }

    pub(super) fn release_terminals(workspace: WorkspaceUuid, cx: &mut App) {
        let panes: Vec<_> = cx
            .global::<Self>()
            .cli
            .as_ref()
            .into_iter()
            .flat_map(|cli| cli.terminals.values())
            .filter(|pane| pane.workspace == workspace)
            .copied()
            .collect();
        for pane in panes {
            Self::unregister_terminal(pane, cx);
        }
    }

    pub(crate) fn restore_cli_tracking(data_dir: &Path, cx: &mut App) {
        let runs: Vec<_> = cli_runs(cx.global::<GlobalTasks>(), |_| true)
            .map(|(_, run, exec)| (run.clone(), exec))
            .collect();
        for (run, exec) in runs {
            match (run.source.cli_process(), &run.session_id) {
                (Some(CliProcessState::Running { pid }), Some(session)) => {
                    if let Some(dir) = sessions_dir(data_dir, run.account_id) {
                        Self::cli_tracker(cx).track_session(session.clone(), *pid, dir);
                    }
                }
                (Some(CliProcessState::Discovering | CliProcessState::Running { .. }), _) => {
                    Self::propose_fact(
                        TaskEvent::CliProcessObserved {
                            exec: exec.clone(),
                            process: CliProcessState::Unknown,
                            at: Utc::now(),
                        },
                        cx,
                    )
                }
                _ => {}
            }
            Self::reconcile_lost_cli(exec, cx);
        }
    }

    fn apply_cli_tracker_event(event: ProcessExited, cx: &mut App) {
        if let Some(effects) = cx.global::<Self>().effects.clone() {
            effects.cli_event(event, cx);
        } else {
            Self::record_cli_exit(&event.session_id, event.pid, cx);
        }
    }
}

#[cfg(test)]
mod tests;
