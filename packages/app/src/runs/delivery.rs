use std::collections::{HashMap, HashSet};

use daruda_store::project::WorkspaceUuid;
use daruda_store::tasks::machine::{SettleOutcome, StartFailure, UndeliveredReason};
use daruda_store::tasks::{ExecutionRef, Lifecycle, TaskEvent};
use gpui::{App, BorrowAppContext as _};

use super::{GlobalTasks, RunsHost};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RunPane {
    pub(crate) workspace: WorkspaceUuid,
    pub(crate) pane: u64,
}

#[derive(Default)]
pub(super) struct Delivery {
    pub(super) bindings: HashMap<RunPane, ExecutionRef>,
    pub(super) settling: HashSet<ExecutionRef>,
    pending_writes: Vec<(ExecutionRef, smol::channel::Receiver<()>)>,
}

#[derive(Debug)]
pub(crate) enum TerminalDispatchError {
    Commit(super::TaskCommitError),
    Write(String),
}

impl std::fmt::Display for TerminalDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Commit(error) => error.fmt(formatter),
            Self::Write(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for TerminalDispatchError {}

#[derive(Clone, Copy)]
pub(crate) enum DeliveryFailure {
    Start(StartFailure),
    Dropped(UndeliveredReason),
}

impl RunsHost {
    pub(crate) fn bind_execution(pane: RunPane, exec: ExecutionRef, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| {
            host.delivery.bindings.insert(pane, exec);
        });
    }

    pub(crate) fn pane_execution(pane: RunPane, cx: &App) -> Option<&ExecutionRef> {
        cx.global::<Self>().delivery.bindings.get(&pane)
    }

    pub(crate) fn unbind_execution(pane: RunPane, cx: &mut App) -> Option<ExecutionRef> {
        cx.update_global::<Self, _>(|host, _| host.delivery.bindings.remove(&pane))
    }

    pub(crate) fn release_execution(pane: RunPane, reason: UndeliveredReason, cx: &mut App) {
        if let Some(exec) = Self::unbind_execution(pane, cx) {
            Self::delivery_failed(exec, DeliveryFailure::Dropped(reason), cx);
        }
    }

    pub(crate) fn release_workspace(workspace: WorkspaceUuid, cx: &mut App) {
        let panes: Vec<_> = cx
            .global::<Self>()
            .delivery
            .bindings
            .keys()
            .filter(|pane| pane.workspace == workspace)
            .copied()
            .collect();
        for pane in panes {
            Self::release_execution(pane, UndeliveredReason::PaneClosed, cx);
        }
        Self::release_terminals(workspace, cx);
    }

    pub(crate) fn prompt_sent(exec: ExecutionRef, session_id: Option<String>, cx: &mut App) {
        Self::propose_fact(
            TaskEvent::Dispatched {
                exec,
                session_id,
                at: chrono::Utc::now(),
            },
            cx,
        );
    }

    pub(crate) fn prompt_dropped(exec: ExecutionRef, reason: UndeliveredReason, cx: &mut App) {
        Self::propose_fact(
            TaskEvent::Undelivered {
                exec,
                reason,
                at: chrono::Utc::now(),
            },
            cx,
        );
    }

    pub(crate) fn delivery_failed(exec: ExecutionRef, failure: DeliveryFailure, cx: &mut App) {
        let pending = cx
            .global::<Self>()
            .delivery
            .pending_writes
            .iter()
            .find(|(writing, _)| writing == &exec)
            .map(|(_, pending)| pending.clone());
        if let Some(pending) = pending {
            cx.spawn(async move |cx| {
                // SILENT-OK: closure signals the write result has been projected.
                let _ = pending.recv().await;
                cx.update(|cx| {
                    Self::after_projection(move |cx| apply_failure(exec, failure, cx), cx)
                });
            })
            .detach();
            return;
        }
        let running = cx
            .global::<GlobalTasks>()
            .get(&exec.task_id)
            .is_some_and(|task| matches!(task.lifecycle, Lifecycle::Running { .. }));
        if running && Self::settled(cx) {
            apply_failure(exec, failure, cx);
        } else {
            cx.defer(move |cx| {
                Self::after_projection(move |cx| apply_failure(exec, failure, cx), cx)
            });
        }
    }

    pub(crate) fn terminal_dispatch(
        exec: ExecutionRef,
        written: std::sync::mpsc::Receiver<Result<(), daruda_terminal::pty::PtyError>>,
        cx: &mut App,
    ) -> gpui::Task<Result<u64, TerminalDispatchError>> {
        let (done, pending) = smol::channel::bounded(1);
        cx.update_global::<Self, _>(|host, _| {
            host.delivery.pending_writes.push((exec.clone(), pending))
        });
        let write = cx.background_executor().spawn(async move {
            written
                .recv()
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()))
        });
        let (answer, answered) = smol::channel::bounded(1);
        cx.spawn(async move |cx| {
            let result = match write.await {
                Ok(()) => cx
                    .update(|cx| {
                        Self::propose(
                            TaskEvent::Dispatched {
                                exec: exec.clone(),
                                session_id: None,
                                at: chrono::Utc::now(),
                            },
                            cx,
                        )
                    })
                    .await
                    .map_err(TerminalDispatchError::Commit),
                Err(detail) => {
                    let result = cx
                        .update(|cx| {
                            Self::propose(
                                TaskEvent::Undelivered {
                                    exec: exec.clone(),
                                    reason: UndeliveredReason::PtyWriteFailed,
                                    at: chrono::Utc::now(),
                                },
                                cx,
                            )
                        })
                        .await;
                    match result {
                        Ok(_) => Err(TerminalDispatchError::Write(detail)),
                        Err(error) => Err(TerminalDispatchError::Commit(error)),
                    }
                }
            };
            cx.update_global::<Self, _>(|host, _| {
                host.delivery
                    .pending_writes
                    .retain(|(writing, _)| writing != &exec)
            });
            drop(done);
            // SILENT-OK: the start's caller stopped listening; the fact is recorded.
            let _ = answer.try_send(result);
        })
        .detach();
        cx.spawn(async move |_| {
            answered
                .recv()
                .await
                .unwrap_or_else(|error| Err(TerminalDispatchError::Write(error.to_string())))
        })
    }

    pub(crate) fn settle_execution(
        exec: ExecutionRef,
        outcome: SettleOutcome,
        result: Option<String>,
        cx: &mut App,
    ) {
        let running = cx
            .global::<GlobalTasks>()
            .get(&exec.task_id)
            .is_some_and(|task| matches!(task.lifecycle, Lifecycle::Running { .. }));
        let operation = move |cx: &mut App| {
            if cx.global::<GlobalTasks>().get(&exec.task_id).is_some_and(|task| matches!(&task.lifecycle, Lifecycle::Running { exec: current, .. } if current == &exec)) {
                Self::propose_fact(TaskEvent::Settled { result, exec, outcome, at: chrono::Utc::now() }, cx);
            }
        };
        if running {
            Self::after_projection(operation, cx);
        } else {
            cx.defer(move |cx| Self::after_projection(operation, cx));
        }
    }

    pub(super) fn after_projection(operation: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        if Self::settled(cx) {
            operation(cx);
            return;
        }
        cx.spawn(async move |cx| {
            let mut operation = Some(operation);
            loop {
                let barrier = cx.update(|cx| {
                    if Self::settled(cx) {
                        if let Some(operation) = operation.take() {
                            operation(cx);
                        }
                        None
                    } else {
                        Some(Self::barrier(cx))
                    }
                });
                let Some(barrier) = barrier else { break };
                barrier.await;
            }
        })
        .detach();
    }
}

fn apply_failure(exec: ExecutionRef, failure: DeliveryFailure, cx: &mut App) {
    let Some(task) = cx
        .global::<GlobalTasks>()
        .get(&exec.task_id)
        .filter(|task| task.current_exec() == Some(&exec))
    else {
        return;
    };
    let at = chrono::Utc::now();
    let event = match (&task.lifecycle, failure) {
        (
            Lifecycle::Preparing { .. } | Lifecycle::Dispatching { .. },
            DeliveryFailure::Start(cause),
        ) => TaskEvent::DispatchFailed { exec, cause, at },
        (Lifecycle::Preparing { .. }, DeliveryFailure::Dropped(_)) => TaskEvent::DispatchFailed {
            exec,
            cause: StartFailure::PaneUnavailable,
            at,
        },
        (Lifecycle::Dispatching { .. }, DeliveryFailure::Dropped(reason)) => {
            TaskEvent::Undelivered { exec, reason, at }
        }
        (Lifecycle::Running { .. }, _) => TaskEvent::Settled {
            result: None,
            exec,
            outcome: SettleOutcome::Errored,
            at,
        },
        _ => return,
    };
    RunsHost::propose_fact(event, cx);
}

#[cfg(test)]
mod tests {
    use super::super::seed::{self, SeedEnd, SeedRun};
    use super::*;
    use daruda_store::tasks::CliProcessState;
    use gpui::TestAppContext;

    fn preparing(cx: &mut App) -> (ExecutionRef, RunPane) {
        RunsHost::ensure_installed(cx);
        let run = SeedRun::cli("/cli", CliProcessState::Discovering);
        let id = seed::task(
            seed::spec(Default::default(), "PTY"),
            Some((&run, SeedEnd::Dispatching)),
            cx,
        );
        let pane = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        let exec = run.exec(&id);
        RunsHost::bind_execution(pane, exec.clone(), cx);
        (exec, pane)
    }

    #[gpui::test]
    async fn terminal_dispatch_waits_for_the_actual_pty_write(cx: &mut TestAppContext) {
        let (written, receipt) = std::sync::mpsc::channel();
        let (exec, dispatched) = cx.update(|cx| {
            let (exec, _) = preparing(cx);
            let dispatched = RunsHost::terminal_dispatch(exec.clone(), receipt, cx);
            assert!(matches!(
                cx.global::<GlobalTasks>()
                    .get(&exec.task_id)
                    .unwrap()
                    .lifecycle,
                Lifecycle::Dispatching { .. }
            ));
            (exec, dispatched)
        });
        written.send(Ok(())).unwrap();
        dispatched.await.unwrap();
        cx.update(|cx| {
            assert!(matches!(
                cx.global::<GlobalTasks>()
                    .get(&exec.task_id)
                    .unwrap()
                    .lifecycle,
                Lifecycle::Running { .. }
            ))
        });
    }

    #[gpui::test]
    async fn a_failed_pty_write_is_undelivered_not_running(cx: &mut TestAppContext) {
        let (written, receipt) = std::sync::mpsc::channel();
        let (exec, dispatched) = cx.update(|cx| {
            let (exec, _) = preparing(cx);
            (exec.clone(), RunsHost::terminal_dispatch(exec, receipt, cx))
        });
        written
            .send(Err(daruda_terminal::pty::PtyError::Io(
                "broken pipe".into(),
            )))
            .unwrap();
        assert!(matches!(
            dispatched.await,
            Err(TerminalDispatchError::Write(_))
        ));
        cx.update(|cx| {
            assert!(matches!(
                cx.global::<GlobalTasks>()
                    .get(&exec.task_id)
                    .unwrap()
                    .lifecycle,
                Lifecycle::Undelivered {
                    reason: UndeliveredReason::PtyWriteFailed,
                    ..
                }
            ))
        });
    }

    #[gpui::test]
    async fn closing_a_terminal_waits_for_its_pending_delivery_fact(cx: &mut TestAppContext) {
        let (written, receipt) = std::sync::mpsc::channel();
        let (exec, dispatched) = cx.update(|cx| {
            let (exec, pane) = preparing(cx);
            let dispatched = RunsHost::terminal_dispatch(exec.clone(), receipt, cx);
            RunsHost::release_execution(pane, UndeliveredReason::PaneClosed, cx);
            (exec, dispatched)
        });
        written.send(Ok(())).unwrap();
        dispatched.await.unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(matches!(
                cx.global::<GlobalTasks>()
                    .get(&exec.task_id)
                    .unwrap()
                    .lifecycle,
                Lifecycle::Error {
                    cause: daruda_store::tasks::machine::ErrorCause::SessionFailed,
                    ..
                }
            ))
        });
    }
}
