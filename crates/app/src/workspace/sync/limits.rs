//! Background-poll tasks for each auth domain's plan-rate API, its provider's
//! public service-status page, and the local JSONL activity aggregation.
//!
//! One loop per (endpoint, domain) pair, so cadences can differ
//! (`[usage.poll].limits_secs` vs. `status_secs`; `Activity` shares
//! `limits_secs`), a hung fetch on one never blocks the others, and each can
//! be disabled (`= 0`) independently. Each loop snapshots its live-reload-aware
//! cadence, idle-rechecks when disabled, otherwise dispatches the blocking
//! `ureq` fetch onto the background executor and forwards the result to a
//! workspace setter, then sleeps the cadence. Limit fetches forward their
//! failures too — see [`daruda_claude::UsageOutcome`]; a failed status fetch or
//! a quiet activity tick leaves the previous value in place.
//!
//! Known limitation: the pump is per-`Workspace`, so N project windows fire
//! `2 * DOMAINS * N` account-wide requests per tick. Harmless at the default
//! 5-minute cadence; a process-wide singleton would need to outlive Workspace
//! (the Welcome window has none) to fix it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_claude::{ActivityStats, ProviderUsage, ServiceStatus, activity, service_status, usage};
use daruda_config::PollConfig;
use daruda_store::accounts::{AccountRecipeId, AccountSelection};
use gpui::{Context, Task, WeakEntity};

use crate::workspace::Workspace;
use crate::workspace::claude_session_ops::UsageKey;
use crate::workspace::main_area::pane::FocusedAccount;

/// Re-check cadence while the endpoint is disabled (`secs == 0`). Reuses
/// [`PollConfig::MIN_POLL_SECS`] (60 s): flipping the toggle on takes effect
/// quickly without spinning on `read_with` while idle.
const IDLE_RECHECK: Duration = Duration::from_secs(PollConfig::MIN_POLL_SECS);

/// Spawn the endpoint pumps: limits and status for every auth domain, plus the
/// single activity loop. Returns the `Task<()>` handles so the caller
/// (Workspace constructor) can keep them alive in a field — dropping any task
/// cancels its loop.
pub(in crate::workspace) fn spawn(cx: &mut Context<Workspace>) -> Vec<Task<()>> {
    let mut pumps = Vec::new();
    for recipe in AccountRecipeId::ALL {
        pumps.push(spawn_loop(cx, Endpoint::Limits(recipe)));
        pumps.push(spawn_loop(cx, Endpoint::Status(recipe)));
    }
    pumps.push(spawn_loop(cx, Endpoint::Activity));
    pumps
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    /// One auth domain's plan-rate API, for the account that domain resolves
    /// to.
    Limits(AccountRecipeId),
    /// One provider's public status page. Account-independent but still
    /// domain-specific — each provider hosts its own page.
    Status(AccountRecipeId),
    /// Local JSONL aggregation (no network). Shares the `limits_secs`
    /// cadence — it is the other half of the Usage tab's data and there
    /// is no reason to refresh it on a different schedule, so it adds no
    /// config knob.
    Activity,
}

/// Whether `kind` reads a source scoped to the focused pane's account
/// (`Limits`, `Activity`) rather than an account-independent one
/// (`Status` — the provider's global service health).
fn account_scoped(kind: Endpoint) -> bool {
    match kind {
        Endpoint::Limits(_) | Endpoint::Activity => true,
        Endpoint::Status(_) => false,
    }
}

/// Which account `kind` reads this tick, or `None` when it has nothing to read.
///
/// A `Limits` loop covers one domain, which is usually *not* the focused pane's:
/// the focused pane names an account only for its own domain, so every other
/// domain reports its ambient login — the only account daruda can name for a
/// domain nothing is focused on.
///
/// `Activity` is Claude-only: `daruda_claude::activity` parses Claude Code's own
/// JSONL session logs, a layout no other CLI writes.
fn target_account(
    kind: Endpoint,
    focused: FocusedAccount,
    pane_domain: crate::workspace::main_area::pane::AccountDomain,
) -> Option<(AccountSelection, Option<PathBuf>)> {
    let focused_recipe = focused.recipe(pane_domain);
    match kind {
        Endpoint::Limits(recipe) => {
            let account = usage_account(recipe, &focused, pane_domain);
            let config_dir = (account != AccountSelection::SystemDefault)
                .then(|| focused.into_config_dir())
                .flatten();
            Some((account, config_dir))
        }
        Endpoint::Activity if focused_recipe == AccountRecipeId::Claude => {
            Some((focused.key(), focused.into_config_dir()))
        }
        Endpoint::Activity => None,
        Endpoint::Status(_) => Some((AccountSelection::SystemDefault, None)),
    }
}

/// The account key one domain's usage is cached under. The renderer needs the
/// same answer the pump used, or it would read an empty slot — so both call
/// this rather than each spelling the rule out.
pub(in crate::workspace) fn usage_account(
    recipe: AccountRecipeId,
    focused: &FocusedAccount,
    pane_domain: crate::workspace::main_area::pane::AccountDomain,
) -> AccountSelection {
    if recipe == focused.recipe(pane_domain) {
        focused.key()
    } else {
        AccountSelection::SystemDefault
    }
}

fn spawn_loop(cx: &mut Context<Workspace>, kind: Endpoint) -> Task<()> {
    cx.spawn(async move |this: WeakEntity<Workspace>, cx| {
        loop {
            // 1. Snapshot the poll cadence. `read_with` returns
            //    `Err(_)` once the entity is gone — that's our
            //    cue to exit the loop.
            let interval = match this.read_with(cx, |ws, _| match kind {
                Endpoint::Limits(_) | Endpoint::Activity => ws.claude.usage_poll.limits_interval(),
                Endpoint::Status(_) => ws.claude.usage_poll.status_interval(),
            }) {
                Ok(opt) => opt,
                Err(_) => break,
            };

            // 2. Disabled — idle-check and try again later.
            let Some(dur) = interval else {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            };

            // 3. Resolve which account this tick reads — fresh each time (on
            //    the UI thread, like the cadence read above) so a focus switch
            //    is picked up on the *next* tick.
            let target = if account_scoped(kind) {
                let resolved = this.read_with(cx, |ws, cx| {
                    let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
                        &ws.focused_account_pane(cx),
                    );
                    (ws.focused_account(), domain)
                });
                let (focused, pane_domain) = match resolved {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                target_account(kind, focused, pane_domain)
            } else {
                // Unused for `Status` (`set_service_status` is keyed by domain,
                // not account); `SystemDefault` just satisfies the type.
                Some((AccountSelection::SystemDefault, None))
            };
            let Some((account_key, config_dir)) = target else {
                // Nothing for this endpoint to read this tick.
                cx.background_executor().timer(dur).await;
                continue;
            };

            // 4. Run the blocking fetch off the GPUI thread, then
            //    forward into the workspace setter.
            let fetched = cx
                .background_executor()
                .spawn(async move { fetch(kind, config_dir.as_deref()) })
                .await;

            let forwarded = match fetched {
                // Failures are forwarded, not dropped: `NoToken` is how the
                // cache learns the domain is signed out, and any other error
                // is how it learns to mark its numbers stale.
                Fetched::Limits(recipe, result) => this.update(cx, |ws, cx| {
                    ws.advance_usage(
                        UsageKey {
                            recipe,
                            account: account_key,
                        },
                        result,
                        cx,
                    )
                }),
                Fetched::Status(recipe, Ok(s)) => {
                    this.update(cx, |ws, cx| ws.set_service_status(recipe, s, cx))
                }
                Fetched::Activity(Some(a)) => {
                    this.update(cx, |ws, cx| ws.set_activity_stats(account_key, a, cx))
                }
                // A failed status fetch or a quiet activity tick leaves the
                // previous value in place; the next tick retries, and logging
                // would be churn the user already sees as unchanged chrome.
                Fetched::Status(_, Err(_)) | Fetched::Activity(None) => Ok(()),
            };
            if forwarded.is_err() {
                break;
            }

            // 5. Sleep and loop.
            cx.background_executor().timer(dur).await;
        }
    })
}

/// Everything one manual refresh gathered, ready to apply on the UI thread.
pub(in crate::workspace) struct RefreshRound {
    /// One entry per auth domain, keyed the way the pump keys it. Failures ride
    /// along — which failure it was is what tells a signed-out domain from a
    /// broken refresh.
    pub limits: Vec<(UsageKey, Result<ProviderUsage, daruda_claude::FetchError>)>,
    pub status: Vec<(
        AccountRecipeId,
        Result<ServiceStatus, daruda_claude::FetchError>,
    )>,
    /// `None` when the focused domain has no local activity log to read, or the
    /// read found nothing.
    pub activity: Option<(AccountSelection, ActivityStats)>,
}

/// One round of every usage source across every auth domain — the Usage tab's
/// ⟳ button, which refreshes the whole dashboard rather than one section of it.
/// Routes through the same [`target_account`] the pumps use, so the two can't
/// disagree about which account a domain reads or whether activity applies.
///
/// Blocking (HTTP + disk); call from the background executor.
pub(in crate::workspace) fn refresh_round(
    focused: FocusedAccount,
    pane_domain: crate::workspace::main_area::pane::AccountDomain,
) -> RefreshRound {
    let mut limits = Vec::new();
    let mut status = Vec::new();
    for recipe in AccountRecipeId::ALL {
        let account = usage_account(recipe, &focused, pane_domain);
        let config_dir = target_account(Endpoint::Limits(recipe), focused.clone(), pane_domain)
            .and_then(|(_, dir)| dir);
        limits.push((
            UsageKey { recipe, account },
            fetch_limits(recipe, config_dir.as_deref()),
        ));
        status.push((recipe, fetch_status(recipe)));
    }
    let activity = target_account(Endpoint::Activity, focused, pane_domain).and_then(
        |(account, config_dir)| {
            fetch_activity_in(config_dir.as_deref()).map(|stats| (account, stats))
        },
    );
    RefreshRound {
        limits,
        status,
        activity,
    }
}

fn fetch_limits(
    recipe: AccountRecipeId,
    config_dir: Option<&Path>,
) -> Result<ProviderUsage, daruda_claude::FetchError> {
    usage::source_for(recipe).fetch(config_dir)
}

fn fetch_status(recipe: AccountRecipeId) -> Result<ServiceStatus, daruda_claude::FetchError> {
    service_status::fetch_service_status(usage::source_for(recipe).status_url())
}

fn fetch_activity_in(config_dir: Option<&Path>) -> Option<ActivityStats> {
    match config_dir {
        Some(dir) => fetch_activity_for(dir),
        None => fetch_activity(),
    }
}

/// Result envelope so every endpoint can share a single
/// `background_executor().spawn(...)` call site without dispatch
/// branching across thread boundaries. `Activity` carries an `Option`
/// rather than a `Result` because its failures (no home dir, an
/// unreadable projects root) are all handled by the same silent
/// fallback — there is nothing the loop does differently per error.
enum Fetched {
    /// Carries its domain so the consumer keys the cache without re-reading
    /// `Endpoint` — pairing the two would make an exhaustive match impossible.
    Limits(
        AccountRecipeId,
        Result<ProviderUsage, daruda_claude::FetchError>,
    ),
    Status(
        AccountRecipeId,
        Result<ServiceStatus, daruda_claude::FetchError>,
    ),
    Activity(Option<ActivityStats>),
}

/// Dispatch one fetch. `config_dir` is the focused account's resolved
/// `CLAUDE_CONFIG_DIR` for `Limits`/`Activity` (`None` = system default);
/// unused for `Status`, which is account-independent.
fn fetch(kind: Endpoint, config_dir: Option<&Path>) -> Fetched {
    match kind {
        Endpoint::Limits(recipe) => Fetched::Limits(recipe, fetch_limits(recipe, config_dir)),
        Endpoint::Status(recipe) => Fetched::Status(recipe, fetch_status(recipe)),
        Endpoint::Activity => Fetched::Activity(fetch_activity_in(config_dir)),
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

    use daruda_store::accounts::{AccountId, AccountRecipeId, AccountSelection};

    use crate::workspace::main_area::pane::AccountDomain;

    use super::{Endpoint, FocusedAccount, account_scoped, activity_paths_for, target_account};

    #[test]
    fn only_limits_and_activity_are_account_scoped() {
        assert!(account_scoped(Endpoint::Limits(AccountRecipeId::Claude)));
        assert!(account_scoped(Endpoint::Activity));
        assert!(!account_scoped(Endpoint::Status(AccountRecipeId::Claude)));
    }

    fn managed(recipe: AccountRecipeId) -> FocusedAccount {
        FocusedAccount::Managed {
            id: AccountId::new(),
            recipe,
            config_dir: std::path::PathBuf::from("/data/accounts/some-uuid"),
        }
    }

    /// A `Limits` loop covers one domain, and only the loop matching the
    /// focused pane's domain may read that pane's account. The other domain's
    /// loop reads its ambient login — otherwise it would fetch one provider's
    /// endpoint with the other provider's config dir.
    #[test]
    fn a_limits_loop_reads_the_focused_account_only_in_its_own_domain() {
        let focused = managed(AccountRecipeId::Claude);
        let domain = AccountDomain::Exactly(AccountRecipeId::Claude);

        let (key, dir) = target_account(
            Endpoint::Limits(AccountRecipeId::Claude),
            focused.clone(),
            domain,
        )
        .expect("the focused domain always has an account to read");
        assert_eq!(key, focused.key());
        assert!(dir.is_some(), "a managed account brings its own config dir");

        let (key, dir) = target_account(Endpoint::Limits(AccountRecipeId::Codex), focused, domain)
            .expect("another domain still reads its ambient login");
        assert_eq!(key, AccountSelection::SystemDefault);
        assert_eq!(dir, None);
    }

    /// `daruda_claude::activity` parses Claude Code's own JSONL layout, which
    /// no other CLI writes — pointing it at another domain's config dir would
    /// only ever find nothing.
    #[test]
    fn activity_is_skipped_entirely_outside_the_claude_domain() {
        assert!(
            target_account(
                Endpoint::Activity,
                managed(AccountRecipeId::Codex),
                AccountDomain::Exactly(AccountRecipeId::Codex),
            )
            .is_none()
        );
        assert!(
            target_account(
                Endpoint::Activity,
                managed(AccountRecipeId::Claude),
                AccountDomain::Exactly(AccountRecipeId::Claude),
            )
            .is_some()
        );
    }

    /// A pane with no managed account still belongs to a domain, so its
    /// ambient login is what each loop reads.
    #[test]
    fn a_system_default_pane_reads_the_ambient_login_in_every_domain() {
        for recipe in AccountRecipeId::ALL {
            let (key, dir) = target_account(
                Endpoint::Limits(recipe),
                FocusedAccount::SystemDefault,
                AccountDomain::Any,
            )
            .expect("the ambient login is always readable");
            assert_eq!(key, AccountSelection::SystemDefault);
            assert_eq!(dir, None);
        }
    }

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
