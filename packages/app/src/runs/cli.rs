use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use daruda_agent::pty_tracker::{PtyBinding, PtyTracker, PtyTrackerEvent};
use daruda_store::accounts::{AccountId, AccountRecipeId};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::project::WorkspaceUuid;
use daruda_store::tasks::{
    CliProcessState, ExecutionRef, Lifecycle, SessionEndReason, TaskEvent, TaskExecution,
    TaskRecord,
};
use gpui::{App, BorrowAppContext as _};

use super::{GlobalTasks, RunPane, RunsHost};

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

    pub(crate) fn record_cli_binding(
        pane: Option<RunPane>,
        binding: &PtyBinding,
        transcript: Option<&Path>,
        cx: &mut App,
    ) {
        let owner = pane
            .and_then(|pane| Self::pane_execution(pane, cx))
            .cloned();
        let session = &binding.session_id;
        let tasks = cx.global::<GlobalTasks>();
        let bound = owner.as_ref().and_then(|owner| {
            cli_runs(tasks, |run| {
                run.id == owner.execution_id && run.session_id.is_none()
            })
            .find(|(task, _, _)| task.id() == owner.task_id)
        });
        let mut events = Vec::new();
        if let Some((task, _, exec)) = &bound {
            events.push(TaskEvent::ExecutionSessionBound {
                exec: exec.clone(),
                session_id: session.clone(),
                at: Utc::now(),
            });
            if matches!(task.lifecycle, Lifecycle::Running { .. })
                && !task.session_ids.contains(session)
            {
                events.push(TaskEvent::SessionAttached {
                    exec: exec.clone(),
                    session_id: session.clone(),
                    at: Utc::now(),
                });
            }
        }
        let bound_exec = bound.map(|(_, _, exec)| exec);
        for (_, run, exec) in cli_runs(tasks, |_| true).filter(|(_, run, exec)| {
            run.session_id.as_ref() == Some(session) || bound_exec.as_ref() == Some(exec)
        }) {
            let mut process = run.source.cli_process().cloned().unwrap_or_default();
            if process.observe_running(binding.claude_pid) {
                events.push(TaskEvent::CliProcessObserved {
                    exec: exec.clone(),
                    process,
                    at: Utc::now(),
                });
            }
            if let Some(path) = transcript {
                events.push(TaskEvent::CliTranscriptFound {
                    exec,
                    transcript_path: path.to_path_buf(),
                    at: Utc::now(),
                });
            }
        }
        for event in events {
            Self::propose_fact(event, cx);
        }
    }

    pub(crate) fn record_cli_transcript(session: &str, path: &Path, cx: &mut App) {
        let session = session.to_owned();
        let path = path.to_path_buf();
        Self::after_projection(
            move |cx| {
                let events: Vec<_> = cli_runs(cx.global::<GlobalTasks>(), |run| {
                    run.session_id.as_deref() == Some(&session)
                })
                .filter(|(_, run, _)| {
                    !matches!(&run.source, daruda_store::tasks::TaskExecutionSource::ClaudeCli { transcript_path: Some(current), .. } if current == &path)
                })
                .map(|(_, _, exec)| TaskEvent::CliTranscriptFound {
                    exec,
                    transcript_path: path.clone(),
                    at: Utc::now(),
                })
                .collect();
                for event in events {
                    Self::propose_fact(event, cx);
                }
            },
            cx,
        );
    }

    pub(crate) fn record_cli_exit(session: &str, pid: u32, cx: &mut App) {
        let session = session.to_owned();
        Self::after_projection(
            move |cx| Self::record_cli_exit_projected(&session, pid, cx),
            cx,
        );
    }

    fn record_cli_exit_projected(session: &str, pid: u32, cx: &mut App) {
        let now = Utc::now();
        let events: Vec<_> = cli_runs(cx.global::<GlobalTasks>(), |run| {
            run.session_id.as_deref() == Some(session)
        })
        .filter_map(|(_, run, exec)| {
            let mut process = run.source.cli_process().cloned().unwrap_or_default();
            process.confirm_exit(pid, now).then_some((
                exec.clone(),
                TaskEvent::CliProcessObserved {
                    exec,
                    process,
                    at: now,
                },
            ))
        })
        .collect();
        for (exec, event) in events {
            Self::propose_fact(event, cx);
            Self::reconcile_lost_cli(exec, cx);
        }
    }

    pub(crate) fn attach_cli_session(session: &str, cx: &mut App) {
        let session = session.to_owned();
        Self::after_projection(
            move |cx| {
                let events: Vec<_> = cli_runs(cx.global::<GlobalTasks>(), |run| {
                    run.session_id.as_deref() == Some(&session)
                })
                .filter(|(task, _, _)| {
                    matches!(task.lifecycle, Lifecycle::Running { .. })
                        && !task.session_ids.contains(&session)
                })
                .map(|(_, _, exec)| TaskEvent::SessionAttached {
                    exec,
                    session_id: session.clone(),
                    at: Utc::now(),
                })
                .collect();
                for event in events {
                    Self::propose_fact(event, cx);
                }
            },
            cx,
        );
    }

    pub(crate) fn settle_cli_session(session: &str, reason: SessionEndReason, cx: &mut App) {
        // The session ended now; the transcript read below must not extend it.
        let at = Utc::now();
        let session = session.to_owned();
        Self::after_projection(
            move |cx| {
                let runs: Vec<_> = cli_runs(cx.global::<GlobalTasks>(), |_| true)
                    .filter(|(task, run, exec)| {
                        matches!(task.lifecycle, Lifecycle::Running { .. })
                            && !cx.global::<Self>().delivery.settling.contains(exec)
                            && (run.session_id.as_deref() == Some(&session)
                                || task.session_ids.contains(&session))
                    })
                    .map(|(_, run, exec)| {
                        (
                            exec,
                            match &run.source {
                                daruda_store::tasks::TaskExecutionSource::ClaudeCli {
                                    transcript_path,
                                    ..
                                } => transcript_path.clone(),
                                _ => None,
                            },
                        )
                    })
                    .collect();
                for (exec, path) in runs {
                    cx.update_global::<Self, _>(|host, _| {
                        host.delivery.settling.insert(exec.clone());
                    });
                    let Some(path) = path.filter(|_| reason != SessionEndReason::Error) else {
                        Self::finish_cli_result(exec, reason, None, at, cx);
                        continue;
                    };
                    let reading = cx
                        .background_executor()
                        .spawn(async move { super::cli_result::read(&path) });
                    cx.spawn(async move |cx| {
                        let result = reading.await;
                        cx.update(|cx| {
                            Self::after_projection(
                                move |cx| {
                                    Self::finish_cli_result(exec, reason, result, at, cx);
                                },
                                cx,
                            )
                        });
                    })
                    .detach();
                }
            },
            cx,
        );
    }

    fn finish_cli_result(
        exec: ExecutionRef,
        reason: SessionEndReason,
        result: Option<String>,
        at: DateTime<Utc>,
        cx: &mut App,
    ) {
        let current = cx.global::<GlobalTasks>().get(&exec.task_id)
            .is_some_and(|task| matches!(&task.lifecycle, Lifecycle::Running { exec: current, .. } if current == &exec));
        let proposal = current.then(|| {
            Self::propose(
                TaskEvent::Settled {
                    exec: exec.clone(),
                    outcome: daruda_store::tasks::machine::SettleOutcome::Completed {
                        end_reason: reason,
                    },
                    result,
                    at,
                },
                cx,
            )
        });
        cx.spawn(async move |cx| {
            if let Some(proposal) = proposal
                && let Err(error) = proposal.await
            {
                let severity = match &error {
                    super::TaskCommitError::Rejected(_) => ErrorSeverity::Info,
                    super::TaskCommitError::ReadOnly(_) | super::TaskCommitError::Unavailable => {
                        ErrorSeverity::Warning
                    }
                    super::TaskCommitError::Failed(_) => ErrorSeverity::Error,
                };
                LogWriter::log(
                    ErrorReport::new("A task result was not recorded")
                        .severity(severity)
                        .from_error(&error)
                        .at(file!(), line!())
                        .build(),
                );
            }
            // `propose` resolves only after the durable event is projected.
            cx.update(|cx| {
                cx.update_global::<Self, _>(|host, _| {
                    host.delivery.settling.remove(&exec);
                });
                Self::reconcile_lost_cli(exec, cx);
            });
        })
        .detach();
    }

    fn reconcile_lost_cli(exec: ExecutionRef, cx: &mut App) {
        Self::after_projection(
            move |cx| {
                if cx.global::<Self>().delivery.settling.contains(&exec) {
                    return;
                }
                if cx
                    .global::<Self>()
                    .delivery
                    .bindings
                    .values()
                    .any(|bound| bound == &exec)
                {
                    return;
                }
                let lost = cx
                    .global::<GlobalTasks>()
                    .get(&exec.task_id)
                    .is_some_and(|task| {
                        task.current_exec() == Some(&exec)
                            && matches!(task.lifecycle, Lifecycle::Running { .. })
                            && task
                                .execution
                                .as_ref()
                                .and_then(|run| run.source.cli_process())
                                .is_some_and(|process| {
                                    matches!(
                                        process,
                                        CliProcessState::Unknown
                                            | CliProcessState::ExitConfirmed { .. }
                                    )
                                })
                    });
                if lost {
                    Self::propose_fact(
                        TaskEvent::Reconciled {
                            exec,
                            at: Utc::now(),
                        },
                        cx,
                    );
                }
            },
            cx,
        );
    }

    #[cfg(test)]
    pub(crate) fn tracked_terminal(pane: RunPane, cx: &App) -> Option<u64> {
        cx.global::<Self>().cli.as_ref()?.panes.get(&pane).copied()
    }
}

#[cfg(test)]
mod tests {
    mod wait_tests;
    use super::*;
    use crate::runs::seed::{self, SeedEnd, SeedRun};
    use gpui::TestAppContext;

    #[gpui::test]
    fn cli_result_read_protects_the_execution_from_exit_reconciliation(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.jsonl");
        std::fs::write(
            &path,
            super::super::cli_result::tests::assistant("completed CLI answer", false),
        )
        .unwrap();
        let store = daruda_store::tasks::TaskStore::in_directory(
            dir.path().join("task-log"),
            None,
            "test",
            super::super::no_hook(),
        )
        .unwrap();
        let (id, settled_by) = cx.update(|cx| {
            RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
            let run = SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("s");
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            seed::commit_now(
                vec![TaskEvent::CliTranscriptFound {
                    exec: run.exec(&id),
                    transcript_path: path.clone(),
                    at: Utc::now(),
                }],
                cx,
            );
            RunsHost::settle_cli_session("s", SessionEndReason::Stop, cx);
            let settled_by = Utc::now();
            assert!(
                cx.global::<RunsHost>()
                    .delivery
                    .settling
                    .contains(&run.exec(&id))
            );
            RunsHost::record_cli_exit("s", 42, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
                Lifecycle::Running { .. }
            ));
            // The transcript read resolves later; its latency is not run time.
            std::thread::sleep(std::time::Duration::from_millis(20));
            (id, settled_by)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let task = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert!(matches!(task.lifecycle, Lifecycle::Done { .. }));
            assert_eq!(
                task.ended_executions.last().unwrap().result.as_deref(),
                Some("completed CLI answer")
            );
            assert!(task.ended_executions.last().unwrap().finished_at <= settled_by);
            assert!(cx.global::<RunsHost>().delivery.settling.is_empty());
            assert_eq!(store.current().unwrap().1.get(&id), Some(task));
        });
    }

    #[gpui::test]
    fn a_cli_result_read_cannot_settle_a_restarted_execution(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("answer.jsonl");
        std::fs::write(
            &path,
            super::super::cli_result::tests::assistant("old answer", false),
        )
        .unwrap();
        let (id, replacement) = cx.update(|cx| {
            RunsHost::inline_commits_for_test(cx);
            let run = SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("old");
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            RunsHost::record_cli_transcript("old", &path, cx);
            RunsHost::settle_cli_session("old", SessionEndReason::Stop, cx);
            seed::commit_now(
                vec![
                    TaskEvent::CancelRequested {
                        exec: run.exec(&id),
                        at: Utc::now(),
                    },
                    TaskEvent::Reopened {
                        id: id.clone(),
                        run_in: Default::default(),
                        at: Utc::now(),
                    },
                ],
                cx,
            );
            let replacement = SeedRun::cli("/cli", CliProcessState::Running { pid: 43 });
            seed::commit_now(seed::run(&id, &replacement, SeedEnd::Running), cx);
            (id, replacement)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let record = cx.global::<GlobalTasks>().get(&id).unwrap();
            assert!(matches!(&record.lifecycle, Lifecycle::Running { exec, .. } if exec == &replacement.exec(&id)));
            assert_eq!(record.ended_executions.len(), 1);
            assert_eq!(record.ended_executions[0].result, None);
            assert!(cx.global::<RunsHost>().delivery.settling.is_empty());
        });
    }

    #[gpui::test]
    fn an_exit_after_a_pending_binding_is_recorded_durably(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let store = daruda_store::tasks::TaskStore::in_directory(
            directory.path(),
            None,
            "test",
            super::super::no_hook(),
        )
        .unwrap();
        let id = cx.update(|cx| {
            RunsHost::install(RunsHost::open(Ok(store.clone())), cx);
            let run = SeedRun::cli("/cli", CliProcessState::Discovering);
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            let pane = RunPane {
                workspace: WorkspaceUuid::new(),
                pane: 1,
            };
            RunsHost::bind_execution(pane, run.exec(&id), cx);
            RunsHost::record_cli_binding(
                Some(pane),
                &PtyBinding {
                    claude_pid: 42,
                    session_id: "session".into(),
                },
                None,
                cx,
            );
            RunsHost::record_cli_exit("session", 42, cx);
            id
        });
        cx.run_until_parked();
        let durable = store.current().unwrap().1;
        let process = durable
            .get(&id)
            .unwrap()
            .execution
            .as_ref()
            .unwrap()
            .source
            .cli_process();
        assert!(
            matches!(process, Some(CliProcessState::ExitConfirmed { .. })),
            "{process:?}"
        );
        cx.update(|cx| assert_eq!(cx.global::<GlobalTasks>().get(&id), durable.get(&id)));
    }

    #[gpui::test]
    fn the_tracker_keeps_both_windows_panes_distinct(cx: &mut TestAppContext) {
        cx.update(|cx| {
            RunsHost::ensure_installed(cx);
            let first = RunPane {
                workspace: WorkspaceUuid::new(),
                pane: 1,
            };
            let second = RunPane {
                workspace: WorkspaceUuid::new(),
                pane: 1,
            };
            RunsHost::register_terminal(first, 4242, cx);
            RunsHost::register_terminal(second, 4243, cx);
            assert_ne!(
                RunsHost::tracked_terminal(first, cx),
                RunsHost::tracked_terminal(second, cx)
            );
            RunsHost::release_workspace(first.workspace, cx);
            assert!(RunsHost::tracked_terminal(first, cx).is_none());
            assert!(RunsHost::tracked_terminal(second, cx).is_some());
        });
    }

    #[gpui::test]
    fn a_restored_cli_without_a_live_process_does_not_stay_running(cx: &mut TestAppContext) {
        cx.update(|cx| {
            RunsHost::inline_commits_for_test(cx);
            let run =
                SeedRun::cli("/cli", CliProcessState::Running { pid: 42 }).session("restored");
            let id = seed::task(
                seed::spec(Default::default(), "CLI"),
                Some((&run, SeedEnd::Running)),
                cx,
            );
            RunsHost::record_cli_exit("restored", 42, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>().get(&id).unwrap().lifecycle,
                Lifecycle::Interrupted {
                    cause: daruda_store::tasks::machine::InterruptCause::AppRestart,
                    ..
                }
            ));
            let seq = RunsHost::sequence_for_test(cx);
            RunsHost::record_cli_exit("restored", 42, cx);
            assert_eq!(RunsHost::sequence_for_test(cx), seq);
        });
    }
}
