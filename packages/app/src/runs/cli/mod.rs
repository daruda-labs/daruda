//! The CLI tracker this process runs for every task terminal: which panes it
//! watches, and the tracker events it turns into execution facts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use daruda_agent::pty_tracker::{PtyTracker, PtyTrackerEvent};
use daruda_store::accounts::{AccountId, AccountRecipeId};
use daruda_store::project::WorkspaceUuid;
use daruda_store::tasks::{CliProcessState, ExecutionRef, TaskEvent, TaskExecution, TaskRecord};
use gpui::{App, BorrowAppContext as _};

use super::{GlobalTasks, RunPane, RunsHost};

mod session;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct CliTracker {
    tracker: PtyTracker,
    next_pane: u64,
    panes: HashMap<RunPane, u64>,
    reverse: HashMap<u64, RunPane>,
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

pub(crate) fn sessions_dir(data_dir: &Path, account: Option<AccountId>) -> Option<PathBuf> {
    let home = match account {
        Some(id) => Some(daruda_agent::accounts::account_config_dir(data_dir, id)),
        None => daruda_agent::accounts::recipe_for(AccountRecipeId::Claude).system_home_dir(),
    }?;
    Some(daruda_agent::pty_link::sessions_dir_in(&home))
}

impl RunsHost {
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
                next_pane: 0,
                panes: HashMap::new(),
                reverse: HashMap::new(),
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

    pub(crate) fn register_terminal(pane: RunPane, shell_pid: u32, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| {
            let cli = host.cli.as_mut().expect("installed CLI tracker");
            let key = *cli.panes.entry(pane).or_insert_with(|| {
                cli.next_pane += 1;
                cli.reverse.insert(cli.next_pane, pane);
                cli.next_pane
            });
            cli.tracker.register(key, shell_pid);
        });
    }

    pub(crate) fn unregister_terminal(pane: RunPane, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| {
            let cli = host.cli.as_mut().expect("installed CLI tracker");
            if let Some(key) = cli.panes.remove(&pane) {
                cli.reverse.remove(&key);
                cli.tracker.unregister(key);
            }
        });
    }

    pub(super) fn release_terminals(workspace: WorkspaceUuid, cx: &mut App) {
        let panes: Vec<_> = cx
            .global::<Self>()
            .cli
            .as_ref()
            .into_iter()
            .flat_map(|cli| cli.panes.keys())
            .filter(|pane| pane.workspace == workspace)
            .copied()
            .collect();
        for pane in panes {
            Self::unregister_terminal(pane, cx);
        }
    }

    pub(crate) fn track_cli(pane: RunPane, directory: PathBuf, cx: &App) {
        let cli = cx
            .global::<Self>()
            .cli
            .as_ref()
            .expect("installed CLI tracker");
        if let Some(key) = cli.panes.get(&pane) {
            cli.tracker.track_task(*key, directory);
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

    fn apply_cli_tracker_event(event: PtyTrackerEvent, cx: &mut App) {
        let pane = match &event {
            PtyTrackerEvent::BindingChanged { pane_id, binding } => {
                let pane = cx
                    .global::<Self>()
                    .cli
                    .as_ref()
                    .and_then(|cli| cli.reverse.get(pane_id))
                    .copied();
                if let Some(binding) = binding {
                    Self::record_cli_binding(pane, binding, None, cx);
                }
                pane
            }
            PtyTrackerEvent::SessionProcessExited {
                session_id,
                claude_pid,
            } => {
                Self::record_cli_exit(session_id, *claude_pid, cx);
                None
            }
        };
        if let Some(effects) = cx.global::<Self>().effects.clone() {
            effects.cli_event(pane, event, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn tracked_terminal(pane: RunPane, cx: &App) -> Option<u64> {
        cx.global::<Self>().cli.as_ref()?.panes.get(&pane).copied()
    }
}

#[cfg(test)]
mod tests;
