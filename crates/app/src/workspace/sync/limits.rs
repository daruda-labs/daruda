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

use std::path::{Path, PathBuf};
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

            // 3. Limits/Activity are keyed by the focused pane's account —
            //    resolved fresh each tick (on the UI thread, like the
            //    cadence read above) so a focus switch refetches the
            //    newly-focused account on the *next* tick. Status is
            //    account-independent (Anthropic's global service health).
            let (account_key, config_dir) = match kind {
                Endpoint::Limits | Endpoint::Activity => {
                    match this.read_with(cx, |ws, _| ws.focused_account_key()) {
                        Ok(resolved) => resolved,
                        Err(_) => break,
                    }
                }
                // Status is account-independent — the key is unused here
                // (`set_service_status` takes none); `SystemDefault` just
                // satisfies the tuple's type.
                Endpoint::Status => (
                    daruda_store::accounts::AccountSelection::SystemDefault,
                    None,
                ),
            };

            // 4. Run the blocking fetch off the GPUI thread, then
            //    forward into the workspace setter.
            let fetched = cx
                .background_executor()
                .spawn(async move { fetch(kind, config_dir.as_deref()) })
                .await;

            match fetched {
                Fetched::Limits(Ok(l)) => {
                    if this
                        .update(cx, |ws, cx| ws.set_plan_limits(account_key, l, cx))
                        .is_err()
                    {
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
                        .update(cx, |ws, cx| ws.set_activity_stats(account_key, a, cx))
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

            // 5. Sleep and loop.
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

/// Dispatch one fetch. `config_dir` is the focused account's resolved
/// `CLAUDE_CONFIG_DIR` for `Limits`/`Activity` (`None` = system default);
/// unused for `Status`, which is account-independent.
fn fetch(kind: Endpoint, config_dir: Option<&Path>) -> Fetched {
    match kind {
        Endpoint::Limits => Fetched::Limits(match config_dir {
            Some(dir) => limits::fetch_plan_limits_for(dir),
            None => limits::fetch_plan_limits(),
        }),
        Endpoint::Status => Fetched::Status(service_status::fetch_service_status()),
        Endpoint::Activity => Fetched::Activity(match config_dir {
            Some(dir) => fetch_activity_for(dir),
            None => fetch_activity(),
        }),
    }
}

/// Resolve the system-default activity source + cache paths and run one
/// incremental aggregation. Returns `None` when the home directory is
/// unavailable or the aggregation errors (an unreadable projects root,
/// an I/O failure mid-read) — the caller keeps the previous snapshot.
///
/// Blocking (disk I/O over `~/.claude/projects/*/*.jsonl`); only call
/// from the background executor. Shared by the pump and the Usage tab's
/// manual-refresh button (`Workspace::refresh_usage_now`).
pub(in crate::workspace) fn fetch_activity() -> Option<ActivityStats> {
    let (projects_root, cache_path) = activity_paths()?;
    activity::update_activity(&projects_root, &cache_path).ok()
}

/// Like [`fetch_activity`], but aggregates a managed account's own
/// config-dir-scoped JSONL logs (`activity_paths_for`) instead of the
/// system-default `~/.claude/projects`.
pub(in crate::workspace) fn fetch_activity_for(config_dir: &Path) -> Option<ActivityStats> {
    let (projects_root, cache_path) = activity_paths_for(config_dir)?;
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

/// `(<config_dir>/projects, <profile-scoped data dir>/cache/activity-<key>.json)`.
/// Sibling of [`activity_paths`] for a managed account's isolated
/// `CLAUDE_CONFIG_DIR`: `projects_root` is that account's own JSONL logs
/// (under its own config dir, not the system default `~/.claude`), and
/// `cache_path` is keyed by the config dir's final path component — the
/// account UUID, since `account_config_dir` is `accounts_root/<uuid>` — so
/// each account's cache stays distinct from the system default and from
/// every other account's cache. Falls back to the system-default cache
/// file name when the config dir has no final component (e.g. `/`).
fn activity_paths_for(config_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let projects_root = config_dir.join("projects");
    let cache_file_name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(|key| format!("activity-{key}.json"))
        .unwrap_or_else(|| "activity.json".to_string());
    let cache_path = daruda_store::persistence::default_data_dir()
        .join("cache")
        .join(cache_file_name);
    Some((projects_root, cache_path))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::activity_paths_for;

    #[test]
    fn activity_paths_for_scopes_projects_root_and_cache_to_config_dir() {
        let account_id = "alice-1234";
        let config_dir = Path::new("/data/claude-accounts").join(account_id);

        let (projects_root, cache_path) = activity_paths_for(&config_dir)
            .expect("activity_paths_for should always resolve for an explicit config_dir");

        assert_eq!(projects_root, config_dir.join("projects"));

        let cache_file_name = cache_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("cache path should have a file name");
        assert!(
            cache_file_name.contains(account_id),
            "cache file name {cache_file_name:?} should embed the account key {account_id:?}"
        );
        assert_ne!(
            cache_file_name, "activity.json",
            "account-scoped cache must not collide with the system-default cache file name"
        );
    }
}
