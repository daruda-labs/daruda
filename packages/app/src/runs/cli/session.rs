//! Execution facts a CLI session reports: its binding, transcript, exit and
//! end, each proposed to the task log once the projection is current.

use std::path::Path;

use chrono::{DateTime, Utc};
use daruda_agent::pty_tracker::PtyBinding;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::{CliProcessState, ExecutionRef, Lifecycle, SessionEndReason, TaskEvent};
use gpui::{App, BorrowAppContext as _};

use super::cli_runs;
use crate::runs::{GlobalTasks, RunPane, RunsHost};

impl RunsHost {
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
                        .spawn(async move { crate::runs::cli_result::read(&path) });
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
                    crate::runs::TaskCommitError::Rejected(_) => ErrorSeverity::Info,
                    crate::runs::TaskCommitError::ReadOnly(_)
                    | crate::runs::TaskCommitError::Unavailable => ErrorSeverity::Warning,
                    crate::runs::TaskCommitError::Failed(_) => ErrorSeverity::Error,
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

    pub(super) fn reconcile_lost_cli(exec: ExecutionRef, cx: &mut App) {
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
}
