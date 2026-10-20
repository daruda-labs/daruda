//! Foreground snapshots and background disk inspection for result queries.
use super::result::{self, RunKey, RunSnapshot, RunState, WaitingOn};
use super::{GlobalTasks, RunsHost};
use gpui::{App, BorrowAppContext as _};

impl RunsHost {
    /// The host maps pane activity to run-owned facts; only its exact live chat execution may wait.
    pub(crate) fn observe_task_waiting(key: RunKey, waiting_on: Option<WaitingOn>, cx: &mut App) {
        let RunKey::Task(exec) = &key else {
            return;
        };
        let Some(record) = cx
            .try_global::<GlobalTasks>()
            .and_then(|tasks| tasks.get(&exec.task_id))
        else {
            return;
        };
        if record.current_exec() != Some(exec)
            || !matches!(
                record.lifecycle,
                daruda_store::tasks::Lifecycle::Running { .. }
            )
            || !record.execution.as_ref().is_some_and(|run| {
                matches!(
                    run.source,
                    daruda_store::tasks::TaskExecutionSource::AgentChat
                )
            })
        {
            return;
        }
        let Some(mut snapshot) = result::task_snapshot(record, exec) else {
            return;
        };
        if let RunState::Running {
            waiting_on: current,
            ..
        } = &mut snapshot.state
        {
            *current = waiting_on;
        }
        cx.update_global::<Self, _>(|host, _| host.watch.observe_waiting(snapshot));
    }

    pub(crate) fn register_run(key: RunKey, cx: &mut App) {
        Self::ensure_installed(cx);
        cx.update_global::<Self, _>(|host, _| host.watch.register(key));
    }

    pub(crate) fn register_run_solicited(key: RunKey, solicited: bool, cx: &mut App) {
        if !solicited {
            Self::register_run(key, cx);
            return;
        }
        Self::ensure_installed(cx);
        cx.update_global::<Self, _>(|host, _| host.watch.register_solicited(key, solicited));
    }
    pub(crate) fn run_scope(cx: &App) -> Vec<RunKey> {
        cx.try_global::<Self>()
            .map(|host| host.watch.scope())
            .unwrap_or_default()
    }
    pub(crate) fn latest_task_key(task: &str, cx: &App) -> Option<RunKey> {
        let record = cx.try_global::<GlobalTasks>()?.get(task)?;
        record
            .current_exec()
            .or_else(|| record.ended_executions.last().map(|end| &end.exec))
            .cloned()
            .map(RunKey::Task)
    }
    /// Stage process-owned facts before the caller performs disk reads off-thread.
    pub(crate) fn run_query(key: RunKey, cx: &mut App) -> RunQuery {
        if let Some(host) = cx.try_global::<Self>() {
            if let RunKey::Flow(id) = &key
                && let Some(run) = host.flows.by_id(id)
            {
                return RunQuery::Ready(run.result());
            }
            if let RunKey::Task(exec) = &key
                && let Some(mut snapshot) = cx
                    .try_global::<GlobalTasks>()
                    .and_then(|tasks| tasks.get(&exec.task_id))
                    .and_then(|record| result::task_snapshot(record, exec))
            {
                if let RunState::Running { waiting_on, .. } = &mut snapshot.state {
                    *waiting_on = host.watch.waiting_on(&key).cloned();
                }
                return RunQuery::Ready(snapshot);
            }
            if let RunKey::Flow(id) = &key
                && let Some(location) = host.watch.location(id)
            {
                return RunQuery::Disk(DiskQuery {
                    location: location.clone(),
                    fallback: host.watch.get(&key).cloned(),
                    local_end: host.watch.local_end(&key),
                });
            }
        }
        if let RunKey::Flow(id) = &key {
            let effects = cx
                .try_global::<Self>()
                .and_then(|host| host.effects.clone());
            if let Some(dir) = effects.and_then(|effects| effects.locate_flow_run(id, cx)) {
                Self::ensure_installed(cx);
                cx.update_global::<Self, _>(|host, _| host.watch.locate(dir.clone()));
                return RunQuery::Disk(DiskQuery {
                    location: dir,
                    fallback: None,
                    local_end: None,
                });
            }
        }
        if let Some(snapshot) = cx
            .try_global::<Self>()
            .and_then(|host| host.watch.get(&key))
        {
            return RunQuery::Ready(snapshot.clone());
        }
        let state = match &key {
            RunKey::Task(_) => RunState::Gone,
            RunKey::Flow(_) => RunState::Unknown,
        };
        RunQuery::Ready(RunSnapshot {
            key,
            what: String::new(),
            why: None,
            state,
        })
    }
}

pub(crate) struct DiskQuery {
    location: super::result::FlowRunLocation,
    fallback: Option<RunSnapshot>,
    local_end: Option<super::watch::LocalEnd>,
}

pub(crate) enum RunQuery {
    Ready(RunSnapshot),
    Disk(DiskQuery),
}
pub(crate) struct QueryObservation {
    pub snapshot: RunSnapshot,
    pub disk_episode: Option<u64>,
}
impl RunQuery {
    pub(crate) fn read(self) -> RunSnapshot {
        self.read_observation().snapshot
    }
    pub(crate) fn read_observation(self) -> QueryObservation {
        match self {
            Self::Ready(snapshot) => QueryObservation {
                snapshot: super::result_io::enrich(snapshot),
                disk_episode: None,
            },
            Self::Disk(query) => {
                let read = super::result_io::flow_observation(&query.location);
                if let Some(fallback) = query.fallback {
                    let local_wins = query.local_end.is_some_and(|authority| {
                        read.resumed_at
                            .is_none_or(|resumed| !authority.superseded_by(resumed))
                    });
                    if local_wins
                        || (matches!(read.snapshot.state, RunState::Gone)
                            && matches!(fallback.state, RunState::Ended { .. }))
                    {
                        return QueryObservation {
                            snapshot: super::result_io::enrich(fallback),
                            disk_episode: None,
                        };
                    }
                }
                QueryObservation {
                    snapshot: read.snapshot,
                    disk_episode: read.resumed_at,
                }
            }
        }
    }
}
#[cfg(test)]
mod tests;
