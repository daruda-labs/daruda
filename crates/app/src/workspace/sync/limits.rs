//! Background-poll tasks for the Anthropic plan-rate API, the public
//! service-status page, and the local JSONL activity aggregation.
//!
//! Three independent loops, one per endpoint, so cadences can differ
//! (`[usage.poll].limits_secs` vs. `status_secs`; `Activity` shares
//! `limits_secs`), a hung fetch on one never blocks the others, and each can
//! be disabled (`= 0`) independently. Each loop snapshots its live-reload-aware
//! cadence, idle-rechecks when disabled, otherwise dispatches the blocking
//! `ureq` fetch onto the background executor and forwards the result to a
//! workspace setter, then sleeps the cadence. Failed fetches are silently
//! dropped (previous snapshot stays); the Usage tab renders placeholders while
//! the cache is still `Default::default()`.
//!
//! Known limitation: the pump is per-`Workspace`, so N project windows fire
//! `2 * N` account-wide requests per tick. Harmless at the default 5-minute
//! cadence; a process-wide singleton would need to outlive Workspace (the
//! Welcome window has none) to fix it.

use std::path::PathBuf;
use std::time::Duration;

use daruda_claude::{ActivityStats, PlanLimits, ServiceStatus, activity, limits, service_status};
use daruda_config::PollConfig;
use gpui::{Context, Task, WeakEntity};

use crate::workspace::Workspace;

/// Re-check cadence while the endpoint is disabled (`secs == 0`). Reuses
/// [`PollConfig::MIN_POLL_SECS`] (60 s): flipping the toggle on takes effect
/// quickly without spinning on `read_with` while idle.
const IDLE_RECHECK: Duration = Duration::from_secs(PollConfig::MIN_POLL_SECS);

/// Spawn the endpoint pumps. Returns the `Task<()>` handles so the
/// caller (Workspace constructor) can keep them alive in a field —
/// dropping any task cancels its loop.
pub(in crate::workspace) fn spawn(cx: &mut Context<Workspace>) -> (Task<()>, Task<()>, Task<()>) {
    (
        spawn_loop(cx, Endpoint::Limits),
        spawn_loop(cx, Endpoint::Status),
        spawn_loop(cx, Endpoint::Activity),
    )
}

#[derive(Clone, Copy)]
enum Endpoint {
    Limits,
    Status,
    /// Local JSONL aggregation (no network). Shares the `limits_secs`
    /// cadence — it is the other half of the Usage tab's data and there
    /// is no reason to refresh it on a different schedule, so it adds no
    /// config knob.
    Activity,
}

fn spawn_loop(cx: &mut Context<Workspace>, kind: Endpoint) -> Task<()> {
    cx.spawn(async move |this: WeakEntity<Workspace>, cx| {
        loop {
            // 1. Snapshot the poll cadence. `read_with` returns
            //    `Err(_)` once the entity is gone — that's our
            //    cue to exit the loop.
            let interval = match this.read_with(cx, |ws, _| match kind {
                Endpoint::Limits | Endpoint::Activity => ws.claude.usage_poll.limits_interval(),
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
                Fetched::Activity(Some(a)) => {
                    if this
                        .update(cx, |ws, cx| ws.set_activity_stats(a, cx))
                        .is_err()
                    {
                        break;
                    }
                }
                Fetched::Limits(Err(_)) | Fetched::Status(Err(_)) | Fetched::Activity(None) => {
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

/// Result envelope so every endpoint can share a single
/// `background_executor().spawn(...)` call site without dispatch
/// branching across thread boundaries. `Activity` carries an `Option`
/// rather than a `Result` because its failures (no home dir, an
/// unreadable projects root) are all handled by the same silent
/// fallback — there is nothing the loop does differently per error.
enum Fetched {
    Limits(Result<PlanLimits, daruda_claude::FetchError>),
    Status(Result<ServiceStatus, daruda_claude::FetchError>),
    Activity(Option<ActivityStats>),
}

fn fetch(kind: Endpoint) -> Fetched {
    match kind {
        Endpoint::Limits => Fetched::Limits(limits::fetch_plan_limits()),
        Endpoint::Status => Fetched::Status(service_status::fetch_service_status()),
        Endpoint::Activity => Fetched::Activity(fetch_activity()),
    }
}

/// Resolve the activity source + cache paths and run one incremental
/// aggregation. Returns `None` when the home directory is unavailable
/// or the aggregation errors (an unreadable projects root, an I/O
/// failure mid-read) — the caller keeps the previous snapshot.
///
/// Blocking (disk I/O over `~/.claude/projects/*/*.jsonl`); only call
/// from the background executor. Shared by the pump and the Usage tab's
/// manual-refresh button (`Workspace::refresh_usage_now`).
pub(in crate::workspace) fn fetch_activity() -> Option<ActivityStats> {
    let (projects_root, cache_path) = activity_paths()?;
    activity::update_activity(&projects_root, &cache_path).ok()
}

/// `(~/.claude/projects, <profile-scoped data dir>/cache/activity.json)`.
/// `projects_root` is Claude Code's own account-wide JSONL logs — never
/// profile-scoped, every profile reads the same real source. `cache_path`
/// is daruda's own derived cache; profile-scoped like every other
/// on-disk file so a debug/test run recomputes its own cache from that
/// same source instead of overwriting the release build's.
fn activity_paths() -> Option<(PathBuf, PathBuf)> {
    let home = dirs::home_dir()?;
    let projects_root = home.join(".claude").join("projects");
    let cache_path = daruda_store::persistence::default_data_dir()
        .join("cache")
        .join("activity.json");
    Some((projects_root, cache_path))
}
