//! The app's side of agent hooks: the route each terminal PTY carries, and
//! keeping daruda's hook entries in every agent home in step with
//! `claude_status.install_hooks`.
//!
//! The ambient homes (`~/.claude`, `~/.codex`) are reconciled at launch and
//! whenever the setting flips, together with existing managed homes. A
//! managed home is also reconciled before a pane runs under it. Installing at every launch is what repairs an
//! entry another tool rewrote the file without.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use daruda_agent::hooks::installer::{self, Change, HookTarget, InstallerError};
use daruda_agent::hooks::route::{self, Harness, TerminalId};
use daruda_store::accounts::AccountRecipeId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::{App, Global};

use crate::window_registry::WindowRegistry;

mod sync_state;
use sync_state::SyncState;

/// A fresh identity for a terminal about to spawn.
pub(crate) fn mint_terminal_id() -> TerminalId {
    TerminalId::parse(&uuid::Uuid::new_v4().simple().to_string())
        .expect("a simple v4 uuid is a valid terminal id")
}

/// The route a terminal PTY carries. Empty when this process cannot name
/// its own executable or status directory; that terminal reports no
/// agent status, and everything else about it works.
pub(crate) fn terminal_env(terminal: &TerminalId) -> Vec<(String, String)> {
    static ROUTE: OnceLock<Option<(PathBuf, PathBuf)>> = OnceLock::new();
    let route = ROUTE.get_or_init(|| {
        Some((
            std::env::current_exe().ok()?,
            daruda_agent::hooks::status_file::default_dir().ok()?,
        ))
    });
    route
        .as_ref()
        .and_then(|(bin, status_dir)| route::terminal_env(terminal, bin, status_dir))
        .unwrap_or_default()
}

/// The harness a managed account's home belongs to.
fn harness_for(recipe: AccountRecipeId) -> Harness {
    match recipe {
        AccountRecipeId::Claude => Harness::Claude,
        AccountRecipeId::Codex => Harness::Codex,
    }
}

fn install_hooks_enabled(cx: &App) -> bool {
    crate::settings_store::SettingsStore::global(cx)
        .user()
        .claude_status
        .install_hooks
}

/// Bring a managed account's home in line with the setting before a pane
/// runs under it. Small local file edits, done inline like the rest of the
/// account preparation they belong to.
pub(crate) fn prepare_account_home(
    recipe: AccountRecipeId,
    home: &Path,
    cx: &App,
) -> Result<Change, InstallerError> {
    reconcile(
        &HookTarget::new(harness_for(recipe), home),
        install_hooks_enabled(cx),
    )
}

fn reconcile(target: &HookTarget, enabled: bool) -> Result<Change, InstallerError> {
    if enabled {
        installer::install(target)
    } else {
        installer::uninstall(target)
    }
}

#[derive(Clone, PartialEq)]
struct Request {
    enabled: bool,
    targets: Vec<HookTarget>,
}

/// One worker applies requests in order and catches up to the latest request.
#[derive(Default)]
pub(crate) struct AgentHooks {
    state: SyncState,
}

impl Global for AgentHooks {}

impl AgentHooks {
    pub(crate) fn sync(cx: &mut App) {
        let request = Request {
            enabled: install_hooks_enabled(cx),
            targets: reconciliation_targets(cx),
        };
        Self::sync_request(request, cx);
    }

    fn sync_request(request: Request, cx: &mut App) {
        let hooks = cx.default_global::<Self>();
        let Some(mut request) = hooks.state.request(request) else {
            return;
        };
        cx.spawn(async move |cx| {
            loop {
                let work = request.clone();
                let outcomes = cx
                    .background_executor()
                    .spawn(async move { reconcile_homes(work.targets, work.enabled) })
                    .await;
                let again = cx.update(|cx| {
                    let succeeded = outcomes.iter().all(|outcome| outcome.result.is_ok());
                    report(outcomes, request.enabled, cx);
                    let hooks = cx.default_global::<Self>();
                    hooks.state.complete(&request, succeeded)
                });
                match again {
                    Some(next) => request = next,
                    None => break,
                }
            }
        })
        .detach();
    }
}

struct Outcome {
    target: HookTarget,
    was_installed: bool,
    result: Result<Change, InstallerError>,
}

fn reconciliation_targets(cx: &App) -> Vec<HookTarget> {
    let mut targets: Vec<_> = Harness::ALL
        .into_iter()
        .filter_map(HookTarget::system)
        .collect();
    if cx.has_global::<crate::workspace::accounts_global::AccountsGlobal>() {
        append_managed_targets(
            &mut targets,
            &daruda_store::persistence::default_data_dir(),
            crate::workspace::accounts_global::snapshot(cx),
        );
    }
    targets
}

fn append_managed_targets(
    targets: &mut Vec<HookTarget>,
    data: &Path,
    accounts: daruda_store::accounts::AccountsState,
) {
    for account in accounts.accounts {
        let home = daruda_agent::accounts::account_config_dir(data, account.id);
        if home.is_dir() {
            let target = HookTarget::new(harness_for(account.recipe), home);
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
}

fn reconcile_homes(targets: Vec<HookTarget>, enabled: bool) -> Vec<Outcome> {
    targets
        .into_iter()
        .map(|target| Outcome {
            was_installed: installer::is_installed(&target),
            result: reconcile(&target, enabled),
            target,
        })
        .collect()
}

/// Tell the user about every failure, and once when daruda first adds its
/// entries: both change what the user sees in another tool's settings. A
/// launch with no window yet open only logs.
fn report(outcomes: Vec<Outcome>, enabled: bool, cx: &mut App) {
    use crate::surface::strings::agent_hooks as s;
    let mut first_install = false;
    let mut reports = Vec::new();
    for outcome in outcomes {
        match outcome.result {
            Ok(Change::Written) if enabled && !outcome.was_installed => first_install = true,
            Ok(_) => {}
            Err(error) => reports.push(
                ErrorReport::new(s::install_failed())
                    .severity(ErrorSeverity::Warning)
                    .from_error(&error)
                    .at(file!(), line!())
                    .with_context(
                        "file",
                        daruda_store::observability::system_info::redact_home(
                            outcome.target.hooks_file(),
                        ),
                    )
                    .dedup(format!(
                        "agent_hooks.install.{}",
                        outcome.target.harness.as_str()
                    ))
                    .build(),
            ),
        }
    }
    if reports.is_empty() && first_install {
        reports.push(
            ErrorReport::new(s::installed_notice())
                .severity(ErrorSeverity::Info)
                .dedup("agent_hooks.installed")
                .build(),
        );
    }
    let mut pending = Some(reports);
    WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
        for report in pending.take().into_iter().flatten() {
            ws.report_error(report, cx);
        }
    });
    for report in pending.into_iter().flatten() {
        LogWriter::log(report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_are_valid_and_distinct() {
        let a = mint_terminal_id();
        let b = mint_terminal_id();
        assert_ne!(a, b);
        assert!(TerminalId::parse(a.as_str()).is_some());
    }

    #[test]
    fn a_terminal_carries_the_full_route() {
        let id = mint_terminal_id();
        let env = terminal_env(&id);
        let names: Vec<_> = env.iter().map(|(name, _)| name.as_str()).collect();
        let expected: Vec<_> = daruda_core::process_env::TERMINAL_HOOK_ROUTE
            .iter()
            .map(|key| key.name())
            .collect();
        assert_eq!(names, expected);
        assert_eq!(env[0].1, id.as_str());
    }

    #[test]
    fn every_account_recipe_has_a_harness() {
        for recipe in AccountRecipeId::all() {
            let _ = harness_for(recipe);
        }
        assert_eq!(harness_for(AccountRecipeId::Codex), Harness::Codex);
    }

    #[gpui::test]
    fn managed_homes_follow_the_hook_setting(cx: &mut gpui::TestAppContext) {
        use gpui::BorrowAppContext as _;
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings_store::SettingsStore::init(cx);
            for recipe in AccountRecipeId::all() {
                let harness = harness_for(recipe);
                let home = dir.path().join(harness.as_str());
                let target = HookTarget::new(harness, &home);
                assert_eq!(
                    prepare_account_home(recipe, &home, cx).unwrap(),
                    Change::Written
                );
                assert!(installer::is_installed(&target));
                assert_eq!(
                    prepare_account_home(recipe, &home, cx).unwrap(),
                    Change::Unchanged
                );
            }
            cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
                let mut config = daruda_config::Config::default();
                config.claude_status.install_hooks = false;
                store.set_user_for_testing(config);
            });
            for recipe in AccountRecipeId::all() {
                let harness = harness_for(recipe);
                let home = dir.path().join(harness.as_str());
                prepare_account_home(recipe, &home, cx).unwrap();
                assert!(!installer::is_installed(&HookTarget::new(harness, home)));
            }
        });
    }
    #[gpui::test]
    fn queued_setting_changes_finish_at_the_latest_value(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let target = HookTarget::new(Harness::Claude, dir.path());
        cx.update(|cx| {
            AgentHooks::sync_request(
                Request {
                    enabled: true,
                    targets: vec![target.clone()],
                },
                cx,
            );
            AgentHooks::sync_request(
                Request {
                    enabled: false,
                    targets: vec![target.clone()],
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(!installer::is_installed(&target));
        cx.update(|cx| {
            assert!(matches!(&cx.global::<AgentHooks>().state,
                SyncState::Applied(request) if !request.enabled));
            AgentHooks::sync_request(
                Request {
                    enabled: true,
                    targets: vec![target.clone()],
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(installer::is_installed(&target));
        cx.update(|cx| {
            AgentHooks::sync_request(
                Request {
                    enabled: false,
                    targets: vec![target.clone()],
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert!(!installer::is_installed(&target));
    }

    #[test]
    fn disabling_includes_existing_managed_homes() {
        use daruda_store::accounts::{AccountId, AccountsState, ManagedAccount};
        let data = tempfile::tempdir().unwrap();
        let mut accounts = AccountsState::default();
        for recipe in AccountRecipeId::all() {
            let id = AccountId::new();
            let home = daruda_agent::accounts::account_config_dir(data.path(), id);
            installer::install(&HookTarget::new(harness_for(recipe), &home)).unwrap();
            accounts.accounts.push(ManagedAccount {
                id,
                recipe,
                email: None,
                organization: None,
                config_dir: home,
                created_at: 0,
                last_authenticated_at: 0,
            });
        }
        let mut targets = Vec::new();
        append_managed_targets(&mut targets, data.path(), accounts);
        assert_eq!(targets.len(), AccountRecipeId::count());
        assert!(
            reconcile_homes(targets.clone(), false)
                .iter()
                .all(|outcome| outcome.result.is_ok())
        );
        assert!(
            targets
                .iter()
                .all(|target| !installer::is_installed(target))
        );
    }
}
