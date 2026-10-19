//! MCP waits keep their reservation live until the socket acknowledges the full frame.
use crate::runs::{
    RunsHost,
    result_host::RunQuery,
    watch::{WaitAnswer, WaitId, WaitOutcome},
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
            super::convert::tool_result(
                true,
                serde_json::json!({
                    "code": "waiter_busy",
                    "message": "Another omitted-handles daruda_run_wait is already active; \
                                wait for it to return or pass explicit handles.",
                }),
            )
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
        self,
        id: serde_json::Value,
        reply: smol::channel::Sender<super::socket::Outgoing>,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| self.answer(id, reply, cx).await)
            .detach();
    }

    async fn answer(
        mut self,
        id: serde_json::Value,
        reply: smol::channel::Sender<super::socket::Outgoing>,
        cx: &mut gpui::AsyncApp,
    ) {
        read_queries(std::mem::take(&mut self.queries), cx).await;
        let Some(result) = self.settle(cx).await else {
            return;
        };
        let pending = self.pending;
        let text = super::protocol::result_frame(
            id,
            super::convert::tool_result(false, self.payload(result)),
        );
        let (outgoing, receipt) = super::socket::Outgoing::with_receipt(text);
        let delivered = reply.send(outgoing).await.is_ok() && receipt.await.is_ok();
        cx.update(|cx| RunsHost::finish_run_wait(pending.0, delivered, cx));
    }

    /// Wake on the deadline, the disk refresh, or a lifecycle event until the
    /// watch has an answer. `None` once the wait was withdrawn.
    async fn settle(&self, cx: &mut gpui::AsyncApp) -> Option<WaitAnswer> {
        use futures::{FutureExt as _, select_biased};
        let executor = cx.background_executor().clone();
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let deadline = executor.timer(remaining).fuse();
        futures::pin_mut!(deadline);
        let refresh = executor.timer(DISK_REFRESH).fuse();
        futures::pin_mut!(refresh);
        let wait = self.pending.0;
        let mut timed_out = self.timeout == 0;
        loop {
            if !cx.update(|cx| self.pending.is_live(cx)) {
                return None;
            }
            if let Some(answer) = cx.update(|cx| RunsHost::poll_run_wait(wait, timed_out, cx)) {
                return Some(answer);
            }
            select_biased! {
                () = deadline => {
                    timed_out = true;
                    self.refresh(cx).await;
                }
                () = refresh => {
                    self.refresh(cx).await;
                    refresh.set(executor.timer(DISK_REFRESH).fuse());
                }
                _ = self.wake.recv().fuse() => {}
            }
        }
    }

    fn payload(self, result: WaitAnswer) -> serde_json::Value {
        let convert = |runs: Vec<crate::runs::result::RunSnapshot>| -> Vec<wire::RunResult> {
            runs.into_iter().map(super::runs::convert).collect()
        };
        let mut answer = wire::WaitAnswer {
            outcome: match result.outcome {
                WaitOutcome::Settled => wire::WaitOutcome::Settled,
                WaitOutcome::TimedOut => wire::WaitOutcome::TimedOut,
                WaitOutcome::Nothing => wire::WaitOutcome::Nothing,
            },
            ended: convert(result.ended),
            blocked: convert(result.blocked),
            already_reported: convert(result.already_reported),
            not_running: convert(result.not_running),
            pending: convert(result.pending),
            more: result.more,
        };
        answer.not_running.extend(self.ready);
        let mut payload = serde_json::to_value(answer).expect("wait results serialize");
        payload["errors"] = serde_json::json!(self.errors);
        payload["timeout"] = serde_json::json!({
            "requested_secs": self.requested,
            "actual_secs": self.timeout,
            "clamped": self.requested != self.timeout,
        });
        payload
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
