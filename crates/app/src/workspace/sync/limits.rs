//! Background-poll tasks for the Anthropic plan-rate API and the
//! public service-status page. Two independent loops, one per
//! endpoint, so:
//!
//! - the cadence can differ (`[usage.poll].limits_secs` vs.
//!   `status_secs`),
//! - a hung fetch on one endpoint never blocks the other,
//! - either can be disabled (`= 0`) without affecting the other.
//!
//! Each loop:
//! 1. snapshots the current poll cadence from `Workspace::usage_poll`
//!    (live-reload aware — picks up `config.toml` edits next tick),
//! 2. if the cadence is `0` / disabled, sleeps `IDLE_RECHECK` and
//!    re-checks (so flipping the toggle on takes effect within ~1
//!    minute, not "next launch"),
//! 3. otherwise dispatches the synchronous `ureq` fetch onto the
//!    background executor (so the Metal thread never blocks) and
//!    forwards the result to a workspace setter via
//!    `entity.update`,
//! 4. sleeps the cadence interval and loops.
//!
//! Failed fetches don't crash the loop — `Err(_)` is silently
//! dropped, leaving the previous snapshot in place. The Usage tab
//! falls back to placeholder rendering when the cached data is at
//! `Default::default()` (no successful fetch yet).
//!
//! ## Known limitation: per-Workspace duplication
//!
//! Both endpoints return account-wide data (plan limits and public
//! service status are not workspace-scoped), but the pump is owned
//! by `Workspace`, so opening N project windows fires `2 * N`
//! requests every tick. With the default 5-minute cadence and a
//! handful of workspaces this is harmless, but a future refactor
//! should hoist the pump to a process-wide singleton (likely in
//! `App`-level state) and have workspaces subscribe to a shared
//! cache. Tracked as a follow-up because the singleton refactor
//! also needs to handle "no workspaces open yet" — Welcome window
//! exists without a Workspace, so the pump owner has to outlive
//! Workspace lifetime.

use std::time::Duration;

use daruda_claude::{PlanLimits, ServiceStatus, limits, service_status};
use daruda_config::PollConfig;
use gpui::{Context, Task, WeakEntity};

use crate::workspace::Workspace;

/// Re-check cadence used while the endpoint is disabled (`secs ==
/// 0`). Reuses [`PollConfig::MIN_POLL_SECS`] (60 s) — same
/// reasoning: short enough that flipping the toggle on takes
/// effect quickly, long enough that we don't spin on `read_with`
/// while idle. Sharing the constant keeps the two "minimum
/// cadence" knobs in one place.
const IDLE_RECHECK: Duration = Duration::from_secs(PollConfig::MIN_POLL_SECS);

/// Spawn both endpoint pumps. Returns the two `Task<()>` handles
/// so the caller (Workspace constructor) can keep them alive in a
/// field — dropping either task cancels its loop.
pub(in crate::workspace) fn spawn(cx: &mut Context<Workspace>) -> (Task<()>, Task<()>) {
    (
        spawn_loop(cx, Endpoint::Limits),
        spawn_loop(cx, Endpoint::Status),
    )
}

#[derive(Clone, Copy)]
enum Endpoint {
    Limits,
    Status,
}

fn spawn_loop(cx: &mut Context<Workspace>, kind: Endpoint) -> Task<()> {
    cx.spawn(async move |this: WeakEntity<Workspace>, cx| {
        loop {
            // 1. Snapshot the poll cadence. `read_with` returns
            //    `Err(_)` once the entity is gone — that's our
            //    cue to exit the loop.
            let interval = match this.read_with(cx, |ws, _| match kind {
                Endpoint::Limits => ws.claude.usage_poll.limits_interval(),
                Endpoint::Status => ws.claude.usage_poll.status_interval(),
            }) {
                Ok(opt) => opt,
                Err(_) => break,
            };

            // 2. Disabled — idle-check and try again later.
            let Some(dur) = interval else {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

            // 3. Run the blocking fetch off the GPUI thread, then
            //    forward into the workspace setter.
            let fetched = cx
                .background_executor()
                .spawn(async move { fetch(kind) })
                .await;

            match fetched {
                Fetched::Limits(Ok(l)) => {
                    if this.update(cx, |ws, cx| ws.set_plan_limits(l, cx)).is_err() {
                        break;
                    }
                }
                Fetched::Status(Ok(s)) => {
                    if this
                        .update(cx, |ws, cx| ws.set_service_status(s, cx))
                        .is_err()
                    {
                        break;
                    }
                }
                Fetched::Limits(Err(_)) | Fetched::Status(Err(_)) => {
                    // Silent fallback — the renderer treats a
                    // never-updated `Default::default()` snapshot
                    // as "data unavailable" and shows placeholder
                    // chrome. Logging would be churn (the next
                    // tick will retry), and the user already sees
                    // the placeholder UI.
                }
            }

            // 4. Sleep and loop.
            cx.background_executor().timer(dur).await;
        }
    })
}

/// Result envelope so both endpoints can share a single
/// `background_executor().spawn(...)` call site without dispatch
/// branching across thread boundaries.
enum Fetched {
    Limits(Result<PlanLimits, daruda_claude::FetchError>),
    Status(Result<ServiceStatus, daruda_claude::FetchError>),
}

fn fetch(kind: Endpoint) -> Fetched {
    match kind {
        Endpoint::Limits => Fetched::Limits(limits::fetch_plan_limits()),
        Endpoint::Status => Fetched::Status(service_status::fetch_service_status()),
    }
}
