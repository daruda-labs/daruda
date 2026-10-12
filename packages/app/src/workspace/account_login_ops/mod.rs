//! Headless add-account / reauthenticate login:
//! [`Workspace::add_managed_account`] orchestrates the headless add-account
//! login itself — spawning the login process, tracking it in
//! `Workspace::pending_login`, and folding the result back into
//! `Workspace::accounts` via [`Workspace::finish_login`] /
//! [`Workspace::cancel_pending_login`]. [`Workspace::reauthenticate_account`]
//! is the counterpart for an existing [`ManagedAccount`], reusing its config
//! dir instead of minting a fresh one.
//!
//! This is a sibling domain to `account_ops.rs`'s pane-account *switching* —
//! split out because account state (creation, switching, deletion) is one
//! cohesive domain on `Workspace`, mirrored by
//! `settings/sections/accounts.rs` owning the Settings-side half of
//! the same domain (default/delete), but the login flow's spawn/poll/finish
//! machinery is large enough to warrant its own file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Context, Window};

use daruda_agent::accounts::{LoginOutcome, account_config_dir, recipe_for, spawn_login};
use daruda_store::accounts::{AccountId, AccountRecipeId, AccountsState, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::{AddManagedAccount, accounts_global, auth_status_global};
use crate::agent::account::managed_env;
use crate::agent::adapter_cli::{self, AdapterCliRequest};
use crate::agent::launch_resolve::AgentLaunchSpec;
use crate::surface::strings as s;
use crate::workspace::Workspace;

mod attempt;
mod finish;
mod probe;
mod resolve;
#[cfg(test)]
mod tests;

pub(in crate::workspace) use attempt::*;
pub(in crate::workspace) use resolve::*;

/// Timeout for a headless add-account login before it's treated as hung
/// and cancelled. Generous — the flow blocks on the user completing OAuth
/// consent by hand in a system browser window, not on anything this app
/// controls the pace of.
///
/// `pub(in crate::workspace)`: `Workspace::new_with_project`'s startup
/// orphan sweep (`daruda_agent::accounts::sweep_orphan_dirs`) passes this
/// same constant as its `grace` window, so the two can't drift — see that
/// call site's comment for why a login's own timeout is exactly the right
/// sweep grace period.
pub(in crate::workspace) const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Whose credentials a headless login writes into — the key every part of
/// the flow is tracked by: `Workspace::pending_login`, the process-wide slot
/// in [`accounts_global`], and each finish path's staleness guard.
///
/// A system login has no [`AccountId`] at all (there is no `accounts.json`
/// row for the ambient home), which is why this is an enum rather than an
/// `AccountId` with a sentinel value: a fake id would make an unreal account
/// representable everywhere a real one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LoginTarget {
    /// A managed account's own isolated config dir.
    Managed {
        id: AccountId,
        recipe: AccountRecipeId,
    },
    /// The user's ambient home for this domain — what a pane runs under with
    /// [`daruda_store::accounts::AccountSelection::SystemDefault`]. Keyed by
    /// domain alone: there is exactly one such home per domain per machine,
    /// and that is precisely why two windows must not sign into it at once.
    System { recipe: AccountRecipeId },
}

impl LoginTarget {
    /// The auth domain this login signs into. Always present, and never a
    /// second field that could disagree with the variant.
    pub(in crate::workspace) fn recipe(self) -> AccountRecipeId {
        match self {
            LoginTarget::Managed { recipe, .. } | LoginTarget::System { recipe } => recipe,
        }
    }
}

/// Whether cancelling a pending login should remove its target's directory
/// (+ scoped credential-store entry): only for [`LoginFinish::Add`], whose
/// account id names a throwaway dir that never became a kept
/// [`ManagedAccount`].
///
/// [`LoginFinish::Reauth`] names an existing account's real credentials —
/// deleting them on cancel would be permanent data loss for a still-good
/// account (see [`Workspace::reauthenticate_account`]'s doc for why
/// [`Workspace::finish_reauth_failed`] already avoids this on the
/// login-failed path; cancel must match it). [`LoginFinish::System`] is the
/// same hazard one step worse: the directory is the user's own `~/.claude`,
/// which this app never created and must never remove.
pub(in crate::workspace) fn cleanup_dir_on_cancel(finish: LoginFinish) -> bool {
    matches!(finish, LoginFinish::Add)
}

/// Which headless login flow is running — the one axis the flows differ on
/// once the shared spawn/poll machinery is done: which finish method the
/// outcome dispatches into (add files a new [`ManagedAccount`]; reauth
/// updates the existing row by id; system has no row to touch), the title of
/// the spawn-failure toast, and whether a cancel may delete the directory
/// ([`cleanup_dir_on_cancel`]).
///
/// The auth domain is not carried here — it comes from the [`LoginTarget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum LoginFinish {
    Add,
    Reauth,
    System,
}

impl LoginFinish {
    /// Dispatch a resolved `outcome` into the flow-specific finish method.
    /// `home_dir` is where the login wrote: a managed account's config dir,
    /// or the ambient home for a system login.
    fn dispatch(
        self,
        ws: &mut Workspace,
        target: LoginTarget,
        attempt: LoginAttempt,
        home_dir: PathBuf,
        outcome: LoginOutcome,
        cx: &mut Context<Workspace>,
    ) {
        let recipe = target.recipe();
        match (self, target) {
            (LoginFinish::Add, LoginTarget::Managed { id, .. }) => {
                ws.finish_login(id, attempt, home_dir, recipe, outcome, cx)
            }
            (LoginFinish::Reauth, LoginTarget::Managed { id, .. }) => {
                ws.finish_reauth(id, attempt, home_dir, recipe, outcome, cx)
            }
            (LoginFinish::System, _) => ws.finish_system_login(attempt, outcome, cx),
            // An Add/Reauth flow is only ever started against a managed
            // target (`add_managed_account` / `reauthenticate_account` both
            // mint one), so this pairing is unreachable rather than merely
            // unhandled.
            (LoginFinish::Add | LoginFinish::Reauth, LoginTarget::System { .. }) => {
                debug_assert!(false, "a managed login flow ran against a system target");
            }
        }
    }

    /// Title for the toast shown when `spawn_login` itself fails to launch
    /// the child (distinct from a launched-then-failed login, which the
    /// finish methods report).
    fn spawn_error_title(self) -> String {
        match self {
            LoginFinish::Add => s::settings::accounts_login_failed(),
            LoginFinish::Reauth | LoginFinish::System => s::settings::accounts_reauth_failed(),
        }
    }
}

// The [`LoginOutcome::Denied`] / [`LoginOutcome::TimedOut`] toast-body
// detail folded into the failure toast's `.message()` is authored,
// user-visible copy — it goes through `s::settings::accounts_login_denied_detail()`
// / `s::settings::accounts_login_timed_out_detail()` (i18n), not a fixed
// English constant here. [`LoginOutcome::Failed`]'s captured-output
// string stays as-is: it's a diagnostic dump, not an authored sentence.

/// The login this window has in flight, if any.
///
/// The field is private because every transition is a method in this module —
/// starting a login, taking one over, finishing it, cancelling it — and a field
/// the other `impl Workspace` files could assign would make that a convention
/// instead of a rule. `Workspace` holds the value; only this module reads it.
#[derive(Default)]
pub(in crate::workspace) struct LoginState {
    pending: PendingLogin,
}

impl LoginState {
    /// Seed an in-flight login. A test hook: reaching this state in production
    /// goes through [`Workspace::add_managed_account`], which needs a real
    /// child process, so a test that only exercises the *finish* path has no
    /// other way in.
    #[cfg(test)]
    pub(in crate::workspace) fn seed_for_test(&mut self, pending: PendingLogin) {
        self.pending = pending;
    }
}

/// State of an in-flight headless add-account login. At most one at a time; a
/// second `AddManagedAccount` while `InProgress` is expected to be blocked by the
/// UI (a disabled "+ Add account" affordance while a login is running),
/// not by this enum itself.
///
/// `InProgress` carries the cancel [`LoginProcessHandle`] and the
/// [`LoginTarget`] the login writes into — but not the
/// config dir, which is a pure function of data already on `Workspace`
/// (`daruda_agent::accounts::account_config_dir(&self.data_dir, id)` for a
/// managed target; the domain's own `system_home_dir()` for a system one).
/// The target carries the auth domain because a cancel has to clean that dir
/// up through the right one, and only the spawning flow knows which it
/// launched.
///
/// `finish` distinguishes an add-account login (whose account id names a
/// throwaway config dir that only becomes real on success) from a
/// reauthenticate login (whose id names an *existing*
/// [`daruda_agent::accounts::ManagedAccount`]'s real, permanent config dir
/// and Keychain item) and from a system login (which writes into the user's
/// own home). `Workspace::cancel_pending_login` reads it to decide whether
/// cancelling may delete that directory — for the latter two it must not, or
/// cancelling would destroy credentials this app did not create.
///
/// `Preparing` covers the window before a login process even exists: the
/// adapter preparation ([`adapter_cli::prepare`]) is blocking and may install
/// Node.js or the adapter, so it runs on the background executor rather than
/// the UI thread — this variant is what `can_start_login` blocks a second
/// concurrent login on, and what `cancel_pending_login` can still cancel (no
/// process yet, so it interrupts the preparation; an interrupted install
/// leaves only its own staging behind), during that async gap before
/// `spawn_login`
/// produces a real [`LoginProcessHandle`] and the state advances to
/// `InProgress`.
#[derive(Debug, Clone, Default)]
pub(in crate::workspace) enum PendingLogin {
    #[default]
    None,
    Preparing {
        target: LoginTarget,
        /// Which attempt this is — see `account_login_ops::LoginAttempt`. The
        /// target cannot stand in: a taken-over login is replaced by another
        /// attempt at the same target.
        attempt: LoginAttempt,
        finish: LoginFinish,
        /// Interrupts the adapter install this attempt waits on.
        cancellation: daruda_acp::preparation::PreparationCancellation,
    },
    InProgress {
        target: LoginTarget,
        attempt: LoginAttempt,
        /// Digest of the ambient credential entry as it stood when this
        /// attempt started, for the clobber check in
        /// this module. `None` when there was nothing to read.
        ambient_before: Option<String>,
        // Read by `Workspace::cancel_pending_login` (`handle.cancel()`),
        // wired to the status-bar dropdown's Cancel row.
        handle: daruda_agent::accounts::LoginProcessHandle,
        finish: LoginFinish,
    },
}

impl Workspace {
    /// Action handler for [`AddManagedAccount`]. Thin shim, mirroring
    /// [`Self::on_switch_pane_account`].
    pub(in crate::workspace) fn on_add_managed_account(
        &mut self,
        action: &AddManagedAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_managed_account(action.0, window, cx);
    }

    /// The login command that signs into `requested`, regardless of which
    /// agent the session currently runs — see [`resolve_login_cli`] for
    /// the resolution order. Both login flows go through this, so the
    /// command spawned and the domain the account row is filed under always
    /// come from one resolution.
    pub(in crate::workspace) fn login_cli_for_recipe(
        &self,
        requested: AccountRecipeId,
    ) -> AdapterCliRequest {
        let active_id = self.open_agent_id(None);
        resolve_login_cli(&self.mirrors.agents, &active_id, requested)
    }

    /// Whether a headless login is in flight **anywhere in the process** —
    /// drives the status-bar dropdown's spinner + Cancel row in place of
    /// "+ Add account" (see [`PendingLogin`]).
    ///
    /// Deliberately not limited to this window. The login slot is
    /// process-wide, so a window that does not own the attempt still refuses
    /// to start one; showing it no way out would leave the user hunting for
    /// the window that happens to hold it. The Cancel row cancels across
    /// windows to match.
    pub(in crate::workspace) fn is_login_pending(&self, cx: &App) -> bool {
        !can_start_login(&self.login.pending) || accounts_global::login_busy(cx)
    }

    /// Start a headless add-account login for the `recipe` auth domain:
    /// spawn that domain's login command
    /// ([`Self::login_cli_for_recipe`]) with a fresh per-account config
    /// dir and the isolating env (`daruda_config::account_env`), stash a
    /// cancel handle in `pending_login` (drives the status-bar dropdown's
    /// spinner + Cancel row via [`Self::is_login_pending`]), and let the
    /// process run to completion on the background executor.
    /// [`Self::finish_login`] picks the result back up on this `Workspace`
    /// once the wait resolves.
    ///
    /// The command and the domain the new account is filed under come from
    /// that one resolution, so they can't name different domains.
    pub(in crate::workspace) fn add_managed_account(
        &mut self,
        recipe: AccountRecipeId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_start_login(&self.login.pending) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings::accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        let request = self.login_cli_for_recipe(recipe);

        let account_id = AccountId::new();
        let config_dir = account_config_dir(&self.data_dir, account_id);
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings::accounts_login_failed())
                    .message(format!(
                        "could not create the account config directory: {e}"
                    ))
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            return;
        }

        self.spawn_login_flow(
            LoginTarget::Managed {
                id: account_id,
                recipe,
            },
            config_dir,
            request,
            LoginFinish::Add,
            cx,
        );
    }

    /// Clear a stale attempt for `target` so the caller's guard can let a fresh
    /// one through — see [`reclick_restarts`] for when that is the right read
    /// of a repeated click. A no-op for anything else, so the busy guard right
    /// after it still refuses a genuinely concurrent login.
    fn restart_stale_login(&mut self, target: LoginTarget, cx: &mut Context<Self>) {
        if reclick_restarts(&self.login.pending, target) {
            self.cancel_pending_login(cx);
        }
    }

    /// Shared spawn/poll/finish machinery behind every headless login flow
    /// ([`Self::add_managed_account`], [`Self::reauthenticate_account`],
    /// [`Self::reauthenticate_system`]). The caller has already run its own
    /// front-guards (busy / command-available / account-exists) and resolved
    /// `home_dir`; this owns everything after: stash `Preparing`, prepare the
    /// adapter off-thread, spawn the login child, track it as
    /// `InProgress`, and await its `wait()` on the background executor —
    /// dispatching the result through `finish`.
    ///
    /// `home_dir` is where the login's credentials land, and plays two roles
    /// that coincide for a managed account and diverge for a system login: it
    /// is always the probe dir (`has_credentials`), but it is only *injected*
    /// as the domain's config-dir env var for a managed target. A system
    /// login adds no account environment — only the agent's own, as a
    /// `SystemDefault` pane runs — so it signs in where that pane will read.
    fn spawn_login_flow(
        &mut self,
        target: LoginTarget,
        home_dir: PathBuf,
        request: AdapterCliRequest,
        finish: LoginFinish,
        cx: &mut Context<Self>,
    ) {
        let recipe_id = target.recipe();
        let env = match target {
            LoginTarget::Managed { recipe, .. } => managed_env(recipe, &home_dir),
            LoginTarget::System { .. } => daruda_config::AccountEnv::ambient(),
        };

        let attempt = next_login_attempt();
        // Snapshot the ambient entry before anything spawns. A managed login
        // must not disturb the sign-in the user did themselves; the CLI has
        // been seen to write that entry even when pointed at a config dir, so
        // this is what makes such a replacement visible instead of silent.
        // Only for a managed target — a system login writes it on purpose.
        let ambient_before = match target {
            LoginTarget::Managed { .. } => {
                daruda_agent::accounts::credentials::system_credentials_digest()
            }
            LoginTarget::System { .. } => None,
        };
        if !accounts_global::begin_login(cx, attempt) {
            if cleanup_dir_on_cancel(finish) {
                cleanup_account_dir(recipe_id, &home_dir);
            }
            self.report_error(
                ErrorReport::new(s::settings::accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.login.global_busy")
                    .build(),
                cx,
            );
            return;
        }

        // Stash `Preparing` before the async preparation gap even starts —
        // it blocks a second concurrent login (`can_start_login`) and gives
        // the status-bar dropdown its spinner immediately, rather than only
        // once a process actually exists. See `PendingLogin::Preparing`'s
        // doc for why this state exists at all (`adapter_cli::prepare` is
        // blocking and may install the adapter, so it can't run inline here
        // on the UI thread).
        let cancellation = daruda_acp::preparation::PreparationCancellation::default();
        self.login.pending = PendingLogin::Preparing {
            target,
            attempt,
            finish,
            cancellation: cancellation.clone(),
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(
                    async move { adapter_cli::prepare(&request, &cancellation)?.for_account(&env) },
                )
                .await;

            let closed_window_cleanup_path = home_dir.clone();
            let updated = this.update(cx, move |ws, cx| {
                // The user may have cancelled during preparation (or, in
                // principle, a newer call superseded it) — bail without
                // touching anything if this `Preparing` isn't the one that
                // was staged above.
                // Keyed on the attempt, not the target: a taken-over attempt
                // reaches here after its replacement has already staged the
                // same target, and would otherwise spawn a second process.
                let is_current = matches!(
                    &ws.login.pending,
                    PendingLogin::Preparing { attempt: current, .. }
                        if *current == attempt
                );
                if !is_current {
                    return;
                }

                let spawned = prepared.and_then(|cli| {
                    spawn_login(&cli.command, LOGIN_TIMEOUT)
                        .map(|login_process| (cli, login_process))
                        .map_err(|e| e.to_string())
                });
                match spawned {
                    Ok((cli, login_process)) => {
                        let handle = login_process.handle();
                        ws.login.pending = PendingLogin::InProgress {
                            target,
                            attempt,
                            ambient_before,
                            handle,
                            finish,
                        };
                        cx.notify();

                        let wait_home_dir = home_dir.clone();
                        cx.spawn(async move |this, cx| {
                            let probe_dir = home_dir.clone();
                            let outcome = cx
                                .background_executor()
                                .spawn(async move {
                                    // Held until the CLI exits, so the cache
                                    // cannot evict the installation it runs.
                                    let _installation = cli;
                                    // The domain decides what "finished"
                                    // means; the probe it may need is bound
                                    // here, where the home dir is known.
                                    let landed =
                                        move || recipe_for(recipe_id).has_credentials(&probe_dir);
                                    let policy = recipe_for(recipe_id)
                                        .login_completion()
                                        .with_probe(&landed);
                                    login_process.wait(policy)
                                })
                                .await;
                            let cleanup_path = wait_home_dir.clone();
                            if this
                                .update(cx, |ws, cx| {
                                    finish.dispatch(ws, target, attempt, wait_home_dir, outcome, cx)
                                })
                                .is_err()
                            {
                                cx.update_global::<accounts_global::AccountsGlobal, _>(
                                    |global, _| {
                                        accounts_global::clear_login_marker(global, attempt);
                                    },
                                );
                                // Workspace window closed mid-login: the
                                // finish path's own cleanup never ran. Run
                                // it here only for the add flow — a reauth's
                                // dir is the account's permanent one and a
                                // system login's is the user's own home; both
                                // must survive (see `LoginFinish`).
                                if cleanup_dir_on_cancel(finish) {
                                    cleanup_account_dir(recipe_id, &cleanup_path);
                                }
                            }
                        })
                        .detach();
                    }
                    Err(e) => {
                        if cleanup_dir_on_cancel(finish) {
                            cleanup_account_dir(recipe_id, &home_dir);
                        }
                        ws.login.pending = PendingLogin::None;
                        accounts_global::finish_login(cx, attempt);
                        ws.report_error(
                            ErrorReport::new(finish.spawn_error_title())
                                .message(e)
                                .severity(ErrorSeverity::Error)
                                .at(file!(), line!())
                                .build(),
                            cx,
                        );
                        cx.notify();
                    }
                }
            });

            if updated.is_err() {
                cx.update_global::<accounts_global::AccountsGlobal, _>(|global, _| {
                    accounts_global::clear_login_marker(global, attempt);
                });
            }
            if updated.is_err() && cleanup_dir_on_cancel(finish) {
                // Workspace window closed during preparation, before
                // `spawn_login` ever ran: nothing was spawned to leak a
                // process, but the add flow's throwaway config dir still
                // needs the same cleanup the wait-path's closed-window
                // fallback does (a reauth's / system login's dir must not —
                // no-op).
                cleanup_account_dir(recipe_id, &closed_window_cleanup_path);
            }
        })
        .detach();
    }

    /// Cancel the in-flight headless login. Covers both pending states — in
    /// [`PendingLogin::Preparing`] no process exists yet, so there is nothing
    /// to kill. A no-op when nothing is pending.
    ///
    /// INVARIANT: only [`LoginMode::Add`]'s config dir may be deleted
    /// ([`cleanup_dir_on_cancel`]) — a Reauth cancel must leave the account's
    /// real credentials intact, matching [`Self::finish_reauth_failed`].
    ///
    /// INVARIANT: clearing `pending_login` here is what makes the eventual
    /// `finish_login` / `finish_reauth` re-entry a no-op via its staleness
    /// guard, so a cancelled login never double-cleans or toasts.
    pub(in crate::workspace) fn cancel_pending_login(&mut self, cx: &mut Context<Self>) {
        if self.tear_down_pending_login(cx) {
            cx.notify();
        }
    }

    /// Release a login this window owned as the window goes away.
    ///
    /// Its callbacks all re-enter this entity, so once it is dropped nothing
    /// can finish the attempt — but the child process keeps running and the
    /// process-wide slot stays taken until the login times out. Every other
    /// window would refuse to start a login for minutes, with a Cancel row
    /// that reaches a workspace no longer there.
    ///
    /// Same teardown as [`Self::cancel_pending_login`], minus the repaint: the
    /// window being released has nothing left to paint.
    pub(in crate::workspace) fn release_pending_login_on_close(&mut self, cx: &mut App) {
        self.tear_down_pending_login(cx);
    }

    /// The teardown both exits share: take the pending attempt, kill its
    /// process, remove an add login's throwaway dir, free the process-wide
    /// slot. `false` when nothing was pending.
    fn tear_down_pending_login(&mut self, cx: &mut App) -> bool {
        let pending = std::mem::replace(&mut self.login.pending, PendingLogin::None);
        let Some((target, attempt, finish)) = stop_pending(pending) else {
            return false;
        };
        if cleanup_dir_on_cancel(finish)
            && let LoginTarget::Managed { id, recipe } = target
        {
            cleanup_account_dir(recipe, &account_config_dir(&self.data_dir, id));
        }
        accounts_global::finish_login(cx, attempt);
        true
    }

    /// Re-run a headless login for an **existing** managed account — the
    /// counterpart to [`Self::add_managed_account`], reusing the account's
    /// `config_dir` and `AccountId` instead of minting a fresh pair. Guards
    /// identically (login already pending, account gone, no login command).
    ///
    /// INVARIANT: a failed reauth must leave the account untouched, so
    /// [`Self::finish_reauth_failed`] does **not** run the add flow's
    /// throwaway-dir [`cleanup_account_dir`] — that would delete a good
    /// account's credentials over one failed attempt.
    pub(in crate::workspace) fn reauthenticate_account(
        &mut self,
        account_id: AccountId,
        cx: &mut Context<Self>,
    ) {
        let Some(account) = self.accounts.find(account_id) else {
            self.report_error(
                ErrorReport::new(s::settings::accounts_reauth_failed())
                    .message(s::settings::accounts_reauth_missing())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.account_missing")
                    .build(),
                cx,
            );
            return;
        };
        let config_dir = account.config_dir.clone();
        let recipe = account.recipe;

        self.restart_stale_login(
            LoginTarget::Managed {
                id: account_id,
                recipe,
            },
            cx,
        );
        if !can_start_login(&self.login.pending) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings::accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        // Scoped to the account's own domain: a login for another domain
        // run against this config dir would overwrite these credentials
        // with a different domain's.
        let request = self.login_cli_for_recipe(recipe);

        // Defensive only: `config_dir` is the account's existing
        // directory from its original login, so this is a no-op unless
        // something removed it out from under the account row.
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings::accounts_reauth_failed())
                    .message(format!(
                        "could not access the account config directory: {e}"
                    ))
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            return;
        }

        self.spawn_login_flow(
            LoginTarget::Managed {
                id: account_id,
                recipe,
            },
            config_dir,
            request,
            LoginFinish::Reauth,
            cx,
        );
    }

    /// Sign in to the **ambient** home for `recipe` — the credentials a pane
    /// with no managed account runs under
    /// ([`daruda_store::accounts::AccountSelection::SystemDefault`], the
    /// default). Until this existed the app had no in-app way to recover an
    /// expired system login at all: the Settings reauthenticate button only
    /// reached managed accounts, so the one selection most users are on was
    /// the one with no path back.
    ///
    /// Runs the domain's ordinary login command with **no** env override —
    /// see [`Self::spawn_login_flow`]'s `home_dir` doc for why injecting a
    /// config dir here would sign into a place the pane never reads.
    ///
    /// INVARIANT: nothing on disk is created or removed by this flow. There
    /// is no `accounts.json` row for the ambient home, and its directory is
    /// the user's own — [`cleanup_dir_on_cancel`] refuses it.
    pub(in crate::workspace) fn reauthenticate_system(
        &mut self,
        recipe: AccountRecipeId,
        cx: &mut Context<Self>,
    ) {
        self.restart_stale_login(LoginTarget::System { recipe }, cx);
        if !can_start_login(&self.login.pending) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings::accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.system_reauth.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        // The probe dir a credentials-landing domain needs, and the dir the
        // login writes into. Absent only with no home directory and no
        // override — there is nothing to sign into then, so refuse rather
        // than run the command against an invented path.
        let Some(home_dir) = recipe_for(recipe).system_home_dir() else {
            self.report_error(
                ErrorReport::new(s::settings::accounts_reauth_failed())
                    .message(s::settings::accounts_system_home_unknown())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.system_reauth.no_home")
                    .build(),
                cx,
            );
            return;
        };

        let request = self.login_cli_for_recipe(recipe);
        self.spawn_login_flow(
            LoginTarget::System { recipe },
            home_dir,
            request,
            LoginFinish::System,
            cx,
        );
    }

    /// Sign in again for the pane that failed — the action behind an
    /// agent-chat failure banner's re-login button
    /// ([`daruda_acp::Remedy::Reauthenticate`]).
    ///
    /// Resolves which credentials that pane actually runs on
    /// ([`pane_login_target`]) and routes to the matching flow, so the same
    /// button recovers a managed account and the ambient home alike. A pane
    /// whose agent has no local auth domain resolves to nothing; the banner
    /// does not offer the button there, and this stays a no-op if it is ever
    /// reached anyway.
    pub(in crate::workspace) fn reauthenticate_pane_account(
        &mut self,
        pane_id: crate::workspace::main_area::pane_tree::PaneId,
        cx: &mut Context<Self>,
    ) {
        // Slot-aware, so a retry aimed at the orchestrator's chat resolves
        // whether or not its tab happens to be in a runtime right now.
        let selection = self.agent_chat_account_selection(pane_id);
        let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
            &self.account_pane_for(pane_id),
        );
        match pane_login_target(selection, domain, &self.accounts) {
            Some(LoginTarget::Managed { id, .. }) => self.reauthenticate_account(id, cx),
            Some(LoginTarget::System { recipe }) => self.reauthenticate_system(recipe, cx),
            // The banner decides whether signing in would *help* from the
            // failure's classification, which it can; it cannot know whether
            // daruda can run one, which depends on where the agent lives. A
            // remote agent signs in on its own host, so there is nothing to
            // spawn here — and a button that silently does nothing is worse
            // than the dead end it was meant to replace.
            None => self.report_error(
                ErrorReport::new(s::settings::accounts_reauth_elsewhere())
                    .message(s::settings::accounts_reauth_elsewhere_detail())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.not_runnable_here")
                    .build(),
                cx,
            ),
        }
    }
}

/// Matches an existing managed account in the *same auth domain* with the
/// *same* email *and* the same organization — a duplicate login for an
/// account already tracked. The domain is part of the key because one
/// person's Claude and Codex logins are separate credentials living in
/// separate homes; keying on identity alone would discard the second one's
/// fresh config dir as a duplicate of the first.
///
/// Same email under a *different* organization (e.g. the same person under
/// two orgs) is a distinct account, since plan/usage is scoped per
/// organization. Two accounts that both have no captured email/org
/// (`None`/`None`) count as duplicates under this literal-equality rule —
/// an edge case that shouldn't arise once a login always captures identity.
pub(in crate::workspace) fn find_duplicate(
    state: &AccountsState,
    recipe: AccountRecipeId,
    email: Option<&str>,
    org: Option<&str>,
) -> Option<AccountId> {
    state
        .accounts
        .iter()
        .find(|a| {
            a.recipe == recipe && a.email.as_deref() == email && a.organization.as_deref() == org
        })
        .map(|a| a.id)
}

/// Best-effort remove `config_dir` — and whatever OS credential store entry
/// `recipe` scopes to it — after a login attempt didn't produce a (kept)
/// managed account: spawn failed, a `Denied`/`TimedOut`/`Failed` outcome, a
/// duplicate dedup, a cancel, or a closed-window race. Each auth domain owns
/// the removal (Claude also drops a Keychain item; Codex must unlink rather
/// than follow the symlinks in its home), so this routes through
/// [`AccountRecipe::cleanup`](daruda_agent::accounts::AccountRecipe::cleanup)
/// instead of hand-rolling the sequence.
fn cleanup_account_dir(recipe: AccountRecipeId, config_dir: &Path) {
    recipe_for(recipe).cleanup(config_dir);
}

/// Current Unix time in seconds, saturating to `0` on a clock error.
/// Mirrors `settings::sections::accounts`'s private helper of the
/// same shape — not shared across that module boundary for a 5-line
/// helper.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
