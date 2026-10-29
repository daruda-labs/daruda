//! Serialize refreshes and keep progress reads local to the selected run.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Scope {
    Detail,
    History,
}

#[derive(Default)]
pub(super) enum ReadState {
    #[default]
    Idle,
    Reading {
        pending: Option<Scope>,
    },
}

impl ReadState {
    fn begin(&mut self, scope: Scope) -> bool {
        match self {
            Self::Idle => {
                *self = Self::Reading { pending: None };
                true
            }
            Self::Reading { pending } => {
                *pending = Some(pending.map_or(scope, |old| old.max(scope)));
                false
            }
        }
    }

    fn finish(&mut self) -> Option<Scope> {
        match std::mem::take(self) {
            Self::Idle => None,
            Self::Reading { pending } => pending,
        }
    }
}

impl Workspace {
    pub(in crate::workspace) fn read_run_execution(
        &mut self,
        id: FlowDetailId,
        dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.request_execution_read(id, dir, Scope::History, cx);
    }

    pub(in crate::workspace) fn refresh_execution_progress(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        if let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        {
            self.request_execution_read(id, run.dir.clone(), Scope::Detail, cx);
        }
    }

    fn request_execution_read(
        &mut self,
        id: FlowDetailId,
        dir: PathBuf,
        scope: Scope,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            lane,
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        if !run.execution.read.begin(scope) {
            return;
        }
        let lane = *lane;
        let sources: std::collections::HashMap<_, _> = run
            .execution
            .rail
            .iter()
            .map(|row| (row.entry.dir.clone(), row.clone()))
            .collect();
        let lock = self
            .lane_for(lane)
            .and_then(|lane| daruda_flow::lock::CanonicalTree::resolve(&lane.path).ok())
            .map(|tree| daruda_flow::lock::lock_dir_for(&self.lock_root, &tree));
        let read_dir = dir.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                let result = RunHistory::read(&read_dir).map_err(|error| error.to_string());
                let status = daruda_flow::marker::run_status(
                    &read_dir,
                    lock.as_deref(),
                    &crate::workspace::flow_request::process_is_alive,
                );
                let rail = (scope == Scope::History).then(|| {
                    read_dir
                        .parent()
                        .map(|parent| {
                            crate::workspace::flow_history::FlowHistory::read(
                                parent,
                                lock.as_deref(),
                            )
                            .runs()
                            .iter()
                            .map(|entry| history_entry(entry, sources.get(&entry.dir)))
                            .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                });
                (result, status, rail)
            },
            move |ws, (result, status, rail), cx| {
                let Some(FlowDetail {
                    body: FlowDetailBody::Run(run),
                    ..
                }) = ws.pages.flows.detail.as_mut().filter(|d| d.id == id)
                else {
                    return;
                };
                let pending = run.execution.read.finish();
                if let Some(rail) = rail {
                    run.execution.rail = rail;
                }
                if let Some(row) = run
                    .execution
                    .rail
                    .iter_mut()
                    .find(|row| row.entry.dir == dir)
                {
                    row.entry.status = status;
                    if let Ok(history) = &result {
                        row.source = history.source.clone();
                        row.started_at = history.started_at;
                        row.elapsed = history.elapsed;
                    }
                }
                ws.land_run_execution(id, result, cx);
                if let Some(scope) = pending {
                    ws.request_execution_read(id, dir, scope, cx);
                }
            },
        )
        .detach();
    }
}

fn history_entry(
    entry: &crate::workspace::flow_history::FlowRunEntry,
    cached: Option<&HistoryEntry>,
) -> HistoryEntry {
    use daruda_flow::marker::RunStatus;
    if let Some(cached) = cached
        && cached.elapsed.is_some()
        && cached.entry.status == entry.status
        && matches!(
            entry.status,
            RunStatus::Done | RunStatus::Failed | RunStatus::Canceled
        )
    {
        return HistoryEntry {
            entry: entry.clone(),
            ..cached.clone()
        };
    }
    let inspected = daruda_flow::run_log::reader::inspect(&entry.dir)
        .ok()
        .flatten();
    HistoryEntry {
        entry: entry.clone(),
        source: inspected
            .as_ref()
            .and_then(|i| i.state.source().map(std::path::Path::to_path_buf)),
        started_at: inspected.as_ref().and_then(|i| i.started_at),
        elapsed: inspected.as_ref().and_then(|i| i.elapsed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_bursts_queue_one_read_and_preserve_history_requests() {
        let mut state = ReadState::default();
        assert!(state.begin(Scope::Detail));
        for _ in 0..20 {
            assert!(!state.begin(Scope::Detail));
        }
        assert!(!state.begin(Scope::History));
        assert!(!state.begin(Scope::Detail));
        assert!(state.finish() == Some(Scope::History));
        assert!(state.begin(Scope::History));
        assert!(state.finish().is_none());
    }
    #[test]
    fn resumed_history_does_not_reuse_the_stalled_duration() {
        use crate::workspace::pages::flows::execution_fixture::{ExecutionShot, seed};
        use daruda_flow::marker::RunStatus;
        let root = tempfile::tempdir().unwrap();
        let dir = seed(root.path(), ExecutionShot::Completed);
        let entry = crate::workspace::flow_history::FlowRunEntry {
            dir,
            started: "".into(),
            report: None,
            status: RunStatus::Done,
        };
        let fresh = history_entry(&entry, None);
        let mut stalled = fresh.clone();
        stalled.entry.status = RunStatus::Stalled;
        stalled.elapsed = Some(std::time::Duration::from_secs(999));
        for status in [RunStatus::Stalled, RunStatus::Running, RunStatus::Done] {
            let current = crate::workspace::flow_history::FlowRunEntry {
                status,
                ..entry.clone()
            };
            let refreshed = history_entry(&current, Some(&stalled));
            assert_eq!(refreshed.elapsed, fresh.elapsed);
            assert_eq!(refreshed.entry.status, status);
        }
        // Terminal results are immutable; preserve the fast path after refresh.
        std::fs::remove_dir_all(&entry.dir).unwrap();
        assert_eq!(history_entry(&entry, Some(&fresh)).elapsed, fresh.elapsed);
    }
}
