//! MCP waits keep their reservation live until the socket acknowledges the full frame.
use crate::runs::{
    RunsHost,
    result_host::RunQuery,
    watch::{WaitId, WaitOutcome},
};
use daruda_control_types::runs as wire;
use gpui::App;
use std::time::{Duration, Instant};

const TIMEOUT_MAX: u64 = 110;
const DISK_REFRESH: Duration = Duration::from_secs(2);
#[derive(Clone, Copy)]
pub(super) struct PendingRunWait(pub(super) WaitId);
impl PendingRunWait {
    pub(super) fn is_live(self, cx: &App) -> bool {
        RunsHost::run_wait_is_live(self.0, cx)
    }
    pub(super) fn withdraw(self, cx: &mut App) {
        RunsHost::finish_run_wait(self.0, false, cx);
    }
}
pub(super) struct PreparedWait {
    pub pending: PendingRunWait,
    queries: Vec<(RunQuery, u64)>,
    keys: Option<Vec<crate::runs::result::RunKey>>,
    ready: Vec<wire::RunResult>,
    errors: Vec<serde_json::Value>,
    wake: smol::channel::Receiver<()>,
    requested: u64,
    timeout: u64,
    deadline: Instant,
}
pub(super) fn prepare(
    handles: Option<Vec<String>>,
    timeout: u64,
    cx: &mut App,
) -> Result<PreparedWait, serde_json::Value> {
    let explicit = handles.is_some();
    let (keys, ready, errors) = super::runs::resolve(handles, cx);
    // Register before staging or reading disk so concurrent lifecycle events wake this wait.
    let (id, wake) =
        RunsHost::open_run_wait(explicit.then(|| keys.clone()), cx).map_err(|_busy| {
            super::convert::tool_result(true, serde_json::json!({"code":"waiter_busy"}))
        })?;
    let queries = stage(&keys, cx);
    let actual = timeout.min(TIMEOUT_MAX);
    Ok(PreparedWait {
        pending: PendingRunWait(id),
        queries,
        keys: explicit.then_some(keys),
        ready,
        errors,
        wake,
        requested: timeout,
        timeout: actual,
        deadline: Instant::now() + Duration::from_secs(actual),
    })
}
impl PreparedWait {
    async fn refresh(&self, cx: &mut gpui::AsyncApp) {
        if !cx.update(|cx| self.pending.is_live(cx)) {
            return;
        }
        let queries = cx.update(|cx| {
            let keys = self.keys.clone().unwrap_or_else(|| RunsHost::run_scope(cx));
            stage(&keys, cx)
                .into_iter()
                .filter_map(|(query, revision)| match query {
                    RunQuery::Disk(_) => Some((query, revision)),
                    RunQuery::Ready(snapshot) => {
                        if matches!(
                            snapshot.state,
                            crate::runs::result::RunState::Running { .. }
                        ) {
                            RunsHost::seed_run_wait_if_current(snapshot, revision, cx);
                        }
                        None
                    }
                })
                .collect()
        });
        read_queries(queries, cx).await;
    }
    pub(super) fn send(
        mut self,
        id: serde_json::Value,
        reply: smol::channel::Sender<super::socket::Outgoing>,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| {
            read_queries(std::mem::take(&mut self.queries), cx).await;
            let timer = cx.background_executor().timer(self.deadline.saturating_duration_since(Instant::now()));
            futures::pin_mut!(timer);
            let refresh = cx.background_executor().timer(DISK_REFRESH);
            futures::pin_mut!(refresh);
            let mut timed_out = self.timeout == 0;
            let result = loop {
                if !cx.update(|cx| self.pending.is_live(cx)) { return; }
                if let Some(answer) = cx.update(|cx| RunsHost::poll_run_wait(self.pending.0, timed_out, cx)) { break answer; }
                use futures::{FutureExt as _, select_biased};
                select_biased! {
                    _ = timer.as_mut().fuse() => { timed_out = true; self.refresh(cx).await; },
                    _ = refresh.as_mut().fuse() => {
                        self.refresh(cx).await;
                        refresh.set(cx.background_executor().timer(DISK_REFRESH));
                    },
                    _ = self.wake.recv().fuse() => {},
                }
            };
            let mut answer = wire::WaitAnswer {
                outcome: match result.outcome { WaitOutcome::Settled => wire::WaitOutcome::Settled, WaitOutcome::TimedOut => wire::WaitOutcome::TimedOut, WaitOutcome::Nothing => wire::WaitOutcome::Nothing },
                ended: result.ended.into_iter().map(super::runs::convert).collect(),
                blocked: result.blocked.into_iter().map(super::runs::convert).collect(),
                already_reported: result.already_reported.into_iter().map(super::runs::convert).collect(),
                not_running: result.not_running.into_iter().map(super::runs::convert).collect(),
                pending: result.pending.into_iter().map(super::runs::convert).collect(),
                more: result.more,
            };
            answer.not_running.extend(self.ready);
            let mut payload = serde_json::to_value(answer).expect("wait results serialize");
            payload["errors"] = serde_json::json!(self.errors);
            payload["timeout"] = serde_json::json!({"requested_secs":self.requested, "actual_secs":self.timeout, "clamped":self.requested != self.timeout});
            let text = super::protocol::result_frame(id, super::convert::tool_result(false, payload));
            let (outgoing, receipt) = super::socket::Outgoing::with_receipt(text);
            let delivered = reply.send(outgoing).await.is_ok() && receipt.await.is_ok();
            cx.update(|cx| RunsHost::finish_run_wait(self.pending.0, delivered, cx));
        }).detach();
    }
}
#[cfg(test)]
mod tests;

fn stage(keys: &[crate::runs::result::RunKey], cx: &mut App) -> Vec<(RunQuery, u64)> {
    keys.iter()
        .map(|key| {
            let query = RunsHost::run_query(key.clone(), cx);
            (query, RunsHost::run_revision(key, cx))
        })
        .collect()
}
async fn read_queries(queries: Vec<(RunQuery, u64)>, cx: &mut gpui::AsyncApp) {
    let snapshots = cx
        .background_executor()
        .spawn(async move {
            queries
                .into_iter()
                .map(|(query, revision)| (query.read_observation(), revision))
                .collect::<Vec<_>>()
        })
        .await;
    cx.update(|cx| {
        for (observation, revision) in snapshots {
            RunsHost::seed_run_observation_if_current(observation, revision, cx);
        }
    });
}
