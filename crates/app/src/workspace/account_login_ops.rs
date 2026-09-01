//! Headless add-account / reauthenticate login (Plan B):
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
//! `settings_window/sections/accounts.rs` owning the Settings-side half of
//! the same domain (default/delete), but the login flow's spawn/poll/finish
//! machinery is large enough to warrant its own file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Context, Window};

use daruda_agent::accounts::{LoginOutcome, account_config_dir, recipe_for, spawn_login};
use daruda_store::accounts::{AccountId, AccountRecipeId, AccountsState, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::{
    AddManagedAccount, PendingLogin, ReauthenticateAccount, ReauthenticateSystem, accounts_global,
    auth_status_global,
};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;

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

/// Budget for one auth-status probe. Far shorter than a login: nothing here
/// waits on a human, only on the CLI starting up and printing a line — and a
/// probe that outlives this is one the user is no longer looking at.
const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(30);

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
            LoginFinish::Add => s::settings_accounts_login_failed(),
            LoginFinish::Reauth | LoginFinish::System => s::settings_accounts_reauth_failed(),
        }
    }
}

// The [`LoginOutcome::Denied`] / [`LoginOutcome::TimedOut`] toast-body
// detail folded into the failure toast's `.message()` is authored,
// user-visible copy — it goes through `s::settings_accounts_login_denied_detail()`
// / `s::settings_accounts_login_timed_out_detail()` (i18n), not a fixed
// English constant here. [`LoginOutcome::Failed`]'s captured-output
// string stays as-is: it's a diagnostic dump, not an authored sentence.

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
    /// agent the session currently runs — see [`resolve_login_command`] for
    /// the resolution order. Both login flows go through this, so the
    /// command spawned and the domain the account row is filed under always
    /// come from one resolution.
    pub(in crate::workspace) fn login_command_for_recipe(
        &self,
        requested: AccountRecipeId,
    ) -> String {
        let active_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        resolve_login_command(&self.agents, &active_id, requested)
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
        !can_start_login(&self.pending_login) || accounts_global::login_busy(cx)
    }

    /// Start a headless add-account login (Plan B, A2 approach) for the
    /// `recipe` auth domain: spawn that domain's login command
    /// ([`Self::login_command_for_recipe`]) with a fresh per-account config
    /// dir and the isolating env (`daruda_config::account_env`), stash a
    /// cancel handle in `pending_login` (drives the status-bar dropdown's
    /// spinner + Cancel row via [`Self::is_login_pending`]), and let the
    /// process run to completion on the background executor.
    /// [`Self::finish_login`] picks the result back up on this `Workspace`
    /// once the wait resolves.
    ///
    /// The command and the domain the new account is filed under come from
    /// that one resolution, so they can't name different domains.
    /// `pub(crate)`: the Settings window's Accounts section also calls this
    /// directly — via `WindowRegistry::first_workspace` — since the headless
    /// login has no other entry point that reaches a specific `Workspace`
    /// from outside `crate::workspace` (see that section's
    /// `start_add_account`).
    pub(crate) fn add_managed_account(
        &mut self,
        recipe: AccountRecipeId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_start_login(&self.pending_login) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        let command = self.login_command_for_recipe(recipe);

        let account_id = AccountId::new();
        let config_dir = account_config_dir(&self.data_dir, account_id);
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_failed())
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
            command,
            LoginFinish::Add,
            cx,
        );
    }

    /// Clear a stale attempt for `target` so the caller's guard can let a fresh
    /// one through — see [`reclick_restarts`] for when that is the right read
    /// of a repeated click. A no-op for anything else, so the busy guard right
    /// after it still refuses a genuinely concurrent login.
    fn restart_stale_login(&mut self, target: LoginTarget, cx: &mut Context<Self>) {
        if reclick_restarts(&self.pending_login, target) {
            self.cancel_pending_login(cx);
        }
    }

    /// Shared spawn/poll/finish machinery behind every headless login flow
    /// ([`Self::add_managed_account`], [`Self::reauthenticate_account`],
    /// [`Self::reauthenticate_system`]). The caller has already run its own
    /// front-guards (busy / command-available / account-exists) and resolved
    /// `home_dir`; this owns everything after: stash `Preparing`, resolve a
    /// managed-node PATH off-thread, spawn the login child, track it as
    /// `InProgress`, and await its `wait()` on the background executor —
    /// dispatching the result through `finish`.
    ///
    /// `home_dir` is where the login's credentials land, and plays two roles
    /// that coincide for a managed account and diverge for a system login: it
    /// is always the probe dir (`has_credentials`), but it is only *injected*
    /// as the domain's config-dir env var for a managed target. A system
    /// login deliberately injects and strips nothing — that is exactly what
    /// `resolve_pane_account` returning `None` means for a `SystemDefault`
    /// pane, so signing in has to run under the same environment the pane
    /// will.
    fn spawn_login_flow(
        &mut self,
        target: LoginTarget,
        home_dir: PathBuf,
        command: String,
        finish: LoginFinish,
        cx: &mut Context<Self>,
    ) {
        let recipe_id = target.recipe();
        let recipe = daruda_agent::accounts::recipe_for(recipe_id);
        let env = match target {
            LoginTarget::Managed { .. } => {
                daruda_config::account_env(recipe.config_dir_env(), &home_dir, recipe.strip_env())
            }
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
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.login.global_busy")
                    .build(),
                cx,
            );
            return;
        }

        // Stash `Preparing` before the async node-resolve gap even starts —
        // it blocks a second concurrent login (`can_start_login`) and gives
        // the status-bar dropdown its spinner immediately, rather than only
        // once a process actually exists. See `PendingLogin::Preparing`'s
        // doc for why this state exists at all (`resolve_node_path_env` is
        // blocking and may download Node.js, so it can't run inline here on
        // the UI thread).
        self.pending_login = PendingLogin::Preparing {
            target,
            attempt,
            finish,
        };
        cx.notify();

        let resolve_command = command.clone();
        cx.spawn(async move |this, cx| {
            let path_pair = cx
                .background_executor()
                .spawn(async move { resolve_node_path_env(&resolve_command) })
                .await;

            let closed_window_cleanup_path = home_dir.clone();
            let updated = this.update(cx, move |ws, cx| {
                // The user may have cancelled during the resolve (or, in
                // principle, a newer call superseded it) — bail without
                // touching anything if this `Preparing` isn't the one that
                // was staged above.
                // Keyed on the attempt, not the target: a taken-over attempt
                // reaches here after its replacement has already staged the
                // same target, and would otherwise spawn a second process.
                let is_current = matches!(
                    &ws.pending_login,
                    PendingLogin::Preparing { attempt: current, .. }
                        if *current == attempt
                );
                if !is_current {
                    return;
                }

                let mut inject = env.inject.clone();
                if let Some(pair) = path_pair {
                    inject.push(pair);
                }

                match spawn_login(&command, &inject, &env.strip, LOGIN_TIMEOUT) {
                    Ok(login_process) => {
                        let handle = login_process.handle();
                        ws.pending_login = PendingLogin::InProgress {
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
                        ws.pending_login = PendingLogin::None;
                        accounts_global::finish_login(cx, attempt);
                        ws.report_error(
                            ErrorReport::new(finish.spawn_error_title())
                                .message(e.to_string())
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
                // Workspace window closed during the node resolve, before
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

    /// Picks up a headless login's result on this `Workspace`, once
    /// [`Self::add_managed_account`]'s background `wait()` resolves.
    ///
    /// Guards against a stale callback first: if `pending_login` no longer
    /// points at `account_id`, this login was already handled —
    /// [`Self::cancel_pending_login`] already cancelled + cleaned it up,
    /// or a newer `add_managed_account` call superseded it — so running
    /// this path anyway would either double-cleanup an already-removed
    /// dir or, worse, clobber a newer `InProgress` this callback knows
    /// nothing about. In that case this is a no-op.
    pub(in crate::workspace) fn finish_login(
        &mut self,
        account_id: AccountId,
        attempt: LoginAttempt,
        config_dir: PathBuf,
        recipe_id: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let ambient_before = match &self.pending_login {
            PendingLogin::InProgress { ambient_before, .. } => ambient_before.clone(),
            _ => None,
        };
        if !self.claim_finished_login(attempt, cx) {
            return;
        }
        self.report_ambient_clobber(ambient_before, cx);

        match outcome {
            LoginOutcome::Success => {
                self.finish_login_success(account_id, config_dir, recipe_id, cx)
            }
            LoginOutcome::Denied => self.finish_login_failed(
                recipe_id,
                config_dir,
                s::settings_accounts_login_denied_detail(),
                cx,
            ),
            LoginOutcome::TimedOut => self.finish_login_failed(
                recipe_id,
                config_dir,
                s::settings_accounts_login_timed_out_detail(),
                cx,
            ),
            LoginOutcome::Failed(detail) => {
                self.finish_login_failed(recipe_id, config_dir, detail, cx)
            }
        }
    }

    /// `LoginOutcome::Success` half of [`Self::finish_login`].
    ///
    /// INVARIANT: filing an account never touches `default_by_recipe` — which
    /// account new panes get is the user's explicit Settings choice.
    ///
    /// INVARIANT: a dedup hit discards the fresh dir rather than adopting it,
    /// so it must not bump `last_authenticated_at` nor report "added" — the
    /// existing credentials are exactly as stale as before. Reauthenticate is
    /// the path that does adopt them.
    fn finish_login_success(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        recipe_id: AccountRecipeId,
        cx: &mut Context<Self>,
    ) {
        let recipe = recipe_for(recipe_id);
        if !recipe.has_credentials(&config_dir) {
            self.finish_login_failed(
                recipe_id,
                config_dir,
                s::settings_accounts_login_no_credentials_detail(),
                cx,
            );
            return;
        }

        let identity = recipe.read_identity(&config_dir);
        let now = now_unix();

        let (state, is_duplicate) =
            match daruda_store::accounts::mutate_accounts_in(&self.data_dir, |state| {
                let duplicate = find_duplicate(
                    state,
                    recipe_id,
                    identity.email.as_deref(),
                    identity.organization.as_deref(),
                );
                match duplicate {
                    Some(_existing_id) => {
                        // The freshly logged-in config dir duplicates an account
                        // already tracked under a different dir — nothing new to
                        // keep, and the existing account's credentials are not
                        // refreshed by this (see this method's doc for why
                        // `last_authenticated_at` is deliberately left untouched).
                        cleanup_account_dir(recipe_id, &config_dir);
                        true
                    }
                    None => {
                        state.accounts.push(ManagedAccount {
                            id: account_id,
                            recipe: recipe_id,
                            email: identity.email,
                            organization: identity.organization,
                            config_dir: config_dir.clone(),
                            created_at: now,
                            last_authenticated_at: now,
                        });
                        false
                    }
                }
            }) {
                Ok((state, is_duplicate)) => (state, is_duplicate),
                Err(e) => {
                    log_io_error(
                        "Failed to save accounts.json after add-account login",
                        "account.add.save_failed",
                        &e,
                    );
                    return;
                }
            };
        // Publish to the app-wide Global (fires `observe_global` on every
        // window, including this one, refreshing each `accounts` mirror);
        // set this window's mirror eagerly too so the code right below sees
        // it without waiting for the deferred callback.
        self.accounts = state.clone();
        accounts_global::replace(cx, state);

        let toast = if is_duplicate {
            s::settings_accounts_login_already_exists()
        } else {
            s::settings_accounts_login_added()
        };
        self.report_error(
            ErrorReport::new(toast)
                .severity(ErrorSeverity::Info)
                .build(),
            cx,
        );
        self.probe_auth_status(
            LoginTarget::Managed {
                id: account_id,
                recipe: recipe_id,
            },
            true,
            cx,
        );
        cx.notify();
    }

    /// Shared failure tail for [`Self::finish_login`] (`Denied`/`TimedOut`/
    /// `Failed`) and [`Self::finish_login_success`]'s own credentials-check
    /// failure: best-effort remove the config dir, surface a toast, notify.
    fn finish_login_failed(
        &mut self,
        recipe: AccountRecipeId,
        config_dir: PathBuf,
        detail: String,
        cx: &mut Context<Self>,
    ) {
        cleanup_account_dir(recipe, &config_dir);
        self.report_error(
            ErrorReport::new(s::settings_accounts_login_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
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
        let (target, attempt, handle, finish) =
            match std::mem::replace(&mut self.pending_login, PendingLogin::None) {
                PendingLogin::None => return,
                PendingLogin::Preparing {
                    target,
                    attempt,
                    finish,
                } => (target, attempt, None, finish),
                PendingLogin::InProgress {
                    target,
                    attempt,
                    handle,
                    finish,
                    ..
                } => (target, attempt, Some(handle), finish),
            };
        if let Some(handle) = handle {
            handle.cancel();
        }
        // Only an add login owns a throwaway dir, and only a managed target
        // has one this app minted — a system cancel has nothing to remove.
        if cleanup_dir_on_cancel(finish)
            && let LoginTarget::Managed { id, recipe } = target
        {
            cleanup_account_dir(recipe, &account_config_dir(&self.data_dir, id));
        }
        accounts_global::finish_login(cx, attempt);
        cx.notify();
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
        let (target, attempt, handle, finish) =
            match std::mem::replace(&mut self.pending_login, PendingLogin::None) {
                PendingLogin::None => return,
                PendingLogin::Preparing {
                    target,
                    attempt,
                    finish,
                } => (target, attempt, None, finish),
                PendingLogin::InProgress {
                    target,
                    attempt,
                    handle,
                    finish,
                    ..
                } => (target, attempt, Some(handle), finish),
            };
        if let Some(handle) = handle {
            handle.cancel();
        }
        if cleanup_dir_on_cancel(finish)
            && let LoginTarget::Managed { id, recipe } = target
        {
            cleanup_account_dir(recipe, &account_config_dir(&self.data_dir, id));
        }
        accounts_global::finish_login(cx, attempt);
    }

    /// Take ownership of a resolved login on this `Workspace`: `true` when
    /// `target` is still the pending one, having cleared both the per-window
    /// state and the process-wide slot; `false` when this callback is stale.
    ///
    /// A stale callback means the login was already handled — cancelled and
    /// cleaned up, or superseded by a newer one — so running its finish path
    /// anyway would either double-clean an already-removed dir or clobber an
    /// `InProgress` it knows nothing about.
    /// Read how each set of credentials daruda can name was signed in, and
    /// cache it for the Settings rows.
    ///
    /// Scoped to domains this user actually has a stake in: one with a managed
    /// account, or one some configured agent signs into. A probe spawns the
    /// domain's CLI through `npx` and may pull down a Node runtime to do it
    /// ([`resolve_node_path_env`]), so probing a domain the user has never
    /// touched would spend a download on a row they do not care about.
    pub(in crate::workspace) fn probe_auth_statuses(&mut self, cx: &mut Context<Self>) {
        let mut targets: Vec<LoginTarget> = AccountRecipeId::all()
            .filter(|recipe| self.has_stake_in_domain(*recipe))
            .map(|recipe| LoginTarget::System { recipe })
            .collect();
        targets.extend(self.accounts.accounts.iter().map(|a| LoginTarget::Managed {
            id: a.id,
            recipe: a.recipe,
        }));
        for target in targets {
            self.probe_auth_status(target, false, cx);
        }
    }

    /// Whether this user has anything in `recipe`'s auth domain: an account
    /// filed under it, or a configured agent that signs into it.
    ///
    /// Deliberately *not* satisfied by the built-in adapter fallback that
    /// [`resolve_login_command`] leans on. That fallback exists so a login can
    /// always be started on request; it must not turn a passive refresh into a
    /// download for a domain nothing here uses.
    fn has_stake_in_domain(&self, recipe: AccountRecipeId) -> bool {
        stake_in_domain(&self.accounts, &self.agents, recipe)
    }

    /// One reading, off the UI thread.
    ///
    /// The probe spawns the agent's CLI (through `npx` on a default install),
    /// so it is resolved here and run on the background executor — the same
    /// division the login flow uses. A domain with no confirmed status command
    /// is skipped rather than guessed at.
    ///
    /// `supersede` marks a caller whose reading is newer than anything already
    /// running — a login that just changed these credentials. Without it an
    /// equivalent request is dropped while one is in flight, so reopening
    /// Settings costs nothing.
    fn probe_auth_status(&mut self, target: LoginTarget, supersede: bool, cx: &mut Context<Self>) {
        let recipe_id = target.recipe();
        let active_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        let Some((command, format)) = resolve_status_command(&self.agents, &active_id, recipe_id)
        else {
            return;
        };
        let Some(ticket) = auth_status_global::begin_probe(cx, target, supersede) else {
            return;
        };
        let recipe = recipe_for(recipe_id);
        let env = match target {
            LoginTarget::Managed { id, .. } => {
                let dir = account_config_dir(&self.data_dir, id);
                daruda_config::account_env(recipe.config_dir_env(), &dir, recipe.strip_env())
            }
            // The ambient home is read exactly as a pane reads it.
            LoginTarget::System { .. } => daruda_config::AccountEnv::ambient(),
        };

        cx.spawn(async move |_this, cx| {
            let probe_command = command.clone();
            let reading = cx
                .background_executor()
                .spawn(async move {
                    let mut inject = env.inject.clone();
                    if let Some(pair) = resolve_node_path_env(&probe_command) {
                        inject.push(pair);
                    }
                    daruda_agent::accounts::auth_status::read_auth_status(
                        &probe_command,
                        format,
                        &inject,
                        &env.strip,
                        AUTH_STATUS_TIMEOUT,
                    )
                })
                .await;
            cx.update(|cx| match reading {
                Some(reading) => auth_status_global::record(cx, ticket, reading),
                // Release the scope, or the next probe reads as a duplicate of
                // one that never answered.
                None => auth_status_global::abandon_probe(cx, ticket),
            });
        })
        .detach();
    }

    /// Report a managed login that replaced the user's own ambient sign-in.
    ///
    /// Nothing is written back: the store is the user's, and a restore built on
    /// an unverified premise could corrupt a working sign-in. Saying so is what
    /// turns a silent replacement into one the user can act on — their
    /// `SystemDefault` panes are now running as the managed account.
    fn report_ambient_clobber(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        let Some(before) = before else {
            return;
        };
        if daruda_agent::accounts::credentials::system_credentials_digest()
            .is_none_or(|after| after == before)
        {
            return;
        }
        self.report_error(
            ErrorReport::new(s::account_ambient_login_replaced())
                .message(s::account_ambient_login_replaced_detail())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("account.login.ambient_clobbered")
                .build(),
            cx,
        );
    }

    fn claim_finished_login(&mut self, attempt: LoginAttempt, cx: &mut Context<Self>) -> bool {
        if !claims_pending(&self.pending_login, attempt) {
            return false;
        }
        self.pending_login = PendingLogin::None;
        accounts_global::finish_login(cx, attempt);
        true
    }

    /// Action handler for [`ReauthenticateAccount`]. Thin shim, mirroring
    /// [`Self::on_add_managed_account`].
    pub(in crate::workspace) fn on_reauthenticate_account(
        &mut self,
        action: &ReauthenticateAccount,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reauthenticate_account(action.0, cx);
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
                ErrorReport::new(s::settings_accounts_reauth_failed())
                    .message(s::account_reauth_missing())
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
        if !can_start_login(&self.pending_login) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
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
        let command = self.login_command_for_recipe(recipe);

        // Defensive only: `config_dir` is the account's existing
        // directory from its original login, so this is a no-op unless
        // something removed it out from under the account row.
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_reauth_failed())
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
            command,
            LoginFinish::Reauth,
            cx,
        );
    }

    /// Action handler for [`ReauthenticateSystem`]. Thin shim, mirroring
    /// [`Self::on_reauthenticate_account`].
    pub(in crate::workspace) fn on_reauthenticate_system(
        &mut self,
        action: &ReauthenticateSystem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reauthenticate_system(action.0, cx);
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
        if !can_start_login(&self.pending_login) || accounts_global::login_busy(cx) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
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
                ErrorReport::new(s::settings_accounts_reauth_failed())
                    .message(s::account_system_home_unknown())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.system_reauth.no_home")
                    .build(),
                cx,
            );
            return;
        };

        let command = self.login_command_for_recipe(recipe);
        self.spawn_login_flow(
            LoginTarget::System { recipe },
            home_dir,
            command,
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
        let Some(selection) = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find(|p| p.id == pane_id)
            .and_then(|p| p.account_selection())
        else {
            return;
        };
        let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
            &self.account_pane_for(pane_id, cx),
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
                ErrorReport::new(s::account_reauth_elsewhere())
                    .message(s::account_reauth_elsewhere_detail())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.not_runnable_here")
                    .build(),
                cx,
            ),
        }
    }

    /// Picks up a system login's result. Unlike the managed flows there is
    /// no row to file or update and no directory to clean up either way —
    /// the credentials landed in the user's own home (or the Keychain) and
    /// that *is* the durable state. All that remains is telling the user,
    /// and letting the panes that were failing on the old credentials try
    /// again.
    fn finish_system_login(
        &mut self,
        attempt: LoginAttempt,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        // Read before the claim clears it — the reconnect sweep below needs to
        // know which credentials this login wrote.
        let PendingLogin::InProgress { target, .. } = self.pending_login else {
            return;
        };
        if !self.claim_finished_login(attempt, cx) {
            return;
        }

        match outcome {
            LoginOutcome::Success => {
                self.report_error(
                    ErrorReport::new(s::settings_accounts_reauth_added())
                        .severity(ErrorSeverity::Info)
                        .build(),
                    cx,
                );
                self.probe_auth_status(target, true, cx);
                self.reconnect_panes_after_login(target, cx);
                cx.notify();
            }
            LoginOutcome::Denied => {
                self.finish_reauth_failed(s::settings_accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => {
                self.finish_reauth_failed(s::settings_accounts_login_timed_out_detail(), cx)
            }
            LoginOutcome::Failed(detail) => self.finish_reauth_failed(detail, cx),
        }
    }

    /// Picks up a reauthenticate-account login's result on this
    /// `Workspace`, once [`Self::reauthenticate_account`]'s background
    /// `wait()` resolves. Mirrors [`Self::finish_login`]'s staleness
    /// guard and dispatch shape.
    fn finish_reauth(
        &mut self,
        account_id: AccountId,
        attempt: LoginAttempt,
        config_dir: PathBuf,
        recipe: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let ambient_before = match &self.pending_login {
            PendingLogin::InProgress { ambient_before, .. } => ambient_before.clone(),
            _ => None,
        };
        if !self.claim_finished_login(attempt, cx) {
            return;
        }
        self.report_ambient_clobber(ambient_before, cx);

        match outcome {
            LoginOutcome::Success => self.finish_reauth_success(account_id, config_dir, recipe, cx),
            LoginOutcome::Denied => {
                self.finish_reauth_failed(s::settings_accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => {
                self.finish_reauth_failed(s::settings_accounts_login_timed_out_detail(), cx)
            }
            LoginOutcome::Failed(detail) => self.finish_reauth_failed(detail, cx),
        }
    }

    /// `LoginOutcome::Success` half of [`Self::finish_reauth`]: confirms
    /// credentials actually landed, re-parses identity, and updates the
    /// row matching the *known* `account_id` directly — bumping
    /// `last_authenticated_at` and refreshing email/organization from
    /// whatever the fresh login reports.
    ///
    /// This deliberately updates by `account_id` rather than going
    /// through [`find_duplicate`] (the add flow's email+org dedup match):
    /// a reauth already knows exactly which account it's reauthenticating,
    /// so keying the update on that id is unambiguous even if the freshly
    /// reparsed identity no longer matches what was on file (e.g. the
    /// organization changed) — `find_duplicate` could otherwise update
    /// the *wrong* row, or file a spurious new account, if the reparsed
    /// identity happens to collide with (or fail to match) a different
    /// existing row.
    fn finish_reauth_success(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        recipe_id: AccountRecipeId,
        cx: &mut Context<Self>,
    ) {
        let recipe = recipe_for(recipe_id);
        if !recipe.has_credentials(&config_dir) {
            self.finish_reauth_failed(s::settings_accounts_reauth_no_credentials_detail(), cx);
            return;
        }

        let identity = recipe.read_identity(&config_dir);
        let now = now_unix();

        let (state, updated) =
            match daruda_store::accounts::mutate_accounts_in(&self.data_dir, |state| {
                let Some(existing) = state.accounts.iter_mut().find(|a| a.id == account_id) else {
                    return false;
                };
                existing.last_authenticated_at = now;
                // Only overwrite identity when the reparse actually yielded a value —
                // a flaky post-success read must not blank a previously-good row.
                if identity.email.is_some() {
                    existing.email = identity.email;
                }
                if identity.organization.is_some() {
                    existing.organization = identity.organization;
                }
                true
            }) {
                Ok((state, updated)) => (state, updated),
                Err(e) => {
                    log_io_error(
                        "Failed to save accounts.json after reauthenticate-account login",
                        "account.reauth.save_failed",
                        &e,
                    );
                    return;
                }
            };
        if !updated {
            // The account was removed (Settings-window delete) while
            // this reauth was in flight — nothing left to update; the
            // delete flow already removed `config_dir` itself. Still
            // republish the freshly-loaded disk state so this window's
            // mirror matches what's on disk (and any other window's).
            self.accounts = state.clone();
            accounts_global::replace(cx, state);
            cx.notify();
            return;
        }
        // Publish to the app-wide Global (fires `observe_global` on every
        // window, including this one, refreshing each `accounts` mirror);
        // set this window's mirror eagerly too so the code right below sees
        // it without waiting for the deferred callback.
        self.accounts = state.clone();
        accounts_global::replace(cx, state);

        self.report_error(
            ErrorReport::new(s::settings_accounts_reauth_added())
                .severity(ErrorSeverity::Info)
                .build(),
            cx,
        );
        self.probe_auth_status(
            LoginTarget::Managed {
                id: account_id,
                recipe: recipe_id,
            },
            true,
            cx,
        );
        // The panes that sent the user here are still sitting on the failure
        // this login just cleared.
        self.reconnect_panes_after_login(
            LoginTarget::Managed {
                id: account_id,
                recipe: recipe_id,
            },
            cx,
        );
        cx.notify();
    }

    /// Shared failure tail for [`Self::finish_reauth`]'s
    /// `Denied`/`TimedOut`/`Failed` outcomes and
    /// [`Self::finish_reauth_success`]'s own credentials-check failure.
    /// Deliberately does **not** call [`cleanup_account_dir`] — see
    /// [`Self::reauthenticate_account`]'s doc for why: `config_dir` here
    /// is the account's real, permanent directory (reused, not freshly
    /// minted for this attempt), so a failed reauth must leave the
    /// existing account and its on-disk credentials exactly as they were
    /// before the attempt.
    fn finish_reauth_failed(&mut self, detail: String, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::settings_accounts_reauth_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
    }
}

/// The login command for `requested`, tried in order: the active agent
/// (keeps the user's own pinned adapter version when it already signs into
/// that domain), the first catalog entry that does, then the built-in
/// adapter for the domain — so a catalog holding no agent for a domain can
/// still add an account there.
///
/// Total: every candidate that can win is a local `Raw` launch (a domain is
/// only derived from one), and the built-in backstop is too, so
/// `login_command` always answers. A domain with no local login flow at all
/// would make this fallible again.
pub(in crate::workspace) fn resolve_login_command(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
) -> String {
    resolve_agent_command(
        agents,
        active_id,
        requested,
        recipe_for(requested).login_args(),
    )
}

/// The command that asks `requested`'s CLI for its auth status, resolved the
/// same way [`resolve_login_command`] resolves the login — so the status read
/// and the login it describes always come from one adapter.
///
/// `None` for a domain with no confirmed status command
/// ([`AccountRecipe::status_probe`](daruda_agent::accounts::AccountRecipe::status_probe)).
pub(in crate::workspace) fn resolve_status_command(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
) -> Option<(
    String,
    daruda_agent::accounts::auth_status::AuthStatusFormat,
)> {
    let probe = recipe_for(requested).status_probe()?;
    Some((
        resolve_agent_command(agents, active_id, requested, probe.args),
        probe.format,
    ))
}

/// Whether `accounts` or `agents` give this user a stake in `recipe`'s domain.
///
/// Pure so the gate that keeps a passive refresh from downloading an adapter is
/// testable without a `Workspace`.
pub(in crate::workspace) fn stake_in_domain(
    accounts: &AccountsState,
    agents: &[daruda_config::AgentDefinition],
    recipe: AccountRecipeId,
) -> bool {
    accounts.accounts.iter().any(|a| a.recipe == recipe)
        || agents
            .iter()
            .any(|a| a.launch.account_recipe(false) == Some(recipe))
}

/// Append `args` to the adapter launch that serves `requested`'s auth domain.
fn resolve_agent_command(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
    args: &str,
) -> String {
    let login_args = args;
    // No lane in scope for this catalog-wide scan — `is_remote: false` is
    // not a guess, it's the honest answer: a bare `Raw` command can't
    // self-report as remote-only (see `account_recipe`'s doc), so this
    // flow's exclusions stay limited to what `Ssh`/`Docker`/`{{cwd}}`
    // already self-describe, exactly as before this axis moved to the lane.
    let signs_into_requested =
        |a: &&daruda_config::AgentDefinition| a.launch.account_recipe(false) == Some(requested);
    let active = agents
        .iter()
        .find(|a| a.id == active_id)
        .filter(signs_into_requested);
    let from_catalog = || agents.iter().find(signs_into_requested);
    let launch = active
        .or_else(from_catalog)
        .map(|a| a.launch.clone())
        .unwrap_or_else(|| builtin_launch_for(requested));
    launch
        .login_command(login_args)
        .expect("a domain-deriving launch is local Raw, which always yields a login command")
}

/// The built-in adapter launch for `recipe` — the fallback when the user's
/// catalog has no agent for that auth domain.
fn builtin_launch_for(recipe: AccountRecipeId) -> daruda_config::AgentLaunch {
    match recipe {
        AccountRecipeId::Claude => daruda_config::AgentDefinition::claude_default().launch,
        AccountRecipeId::Codex => daruda_config::AgentDefinition::codex_default().launch,
    }
}

/// The login a pane's "sign in again" button runs, or `None` when the pane
/// has none to offer.
///
/// Two inputs because neither alone is enough: a managed selection names an
/// account but not its auth domain (the row does), and a `SystemDefault`
/// selection names a domain-less choice whose ambient home is *per domain* —
/// only the pane's own agent says which.
pub(in crate::workspace) fn pane_login_target(
    selection: daruda_store::accounts::AccountSelection,
    domain: crate::workspace::main_area::pane::AccountDomain,
    accounts: &AccountsState,
) -> Option<LoginTarget> {
    use crate::workspace::main_area::pane::AccountDomain;
    use daruda_store::accounts::AccountSelection;
    match selection {
        // The account's own domain, not the pane's: signing in with the
        // pane's domain would write another domain's credentials into this
        // account's dir.
        AccountSelection::Managed(id) => accounts.find(id).map(|account| LoginTarget::Managed {
            id,
            recipe: account.recipe,
        }),
        AccountSelection::SystemDefault => match domain {
            AccountDomain::Exactly(recipe) => Some(LoginTarget::System { recipe }),
            AccountDomain::Any | AccountDomain::Unsupported => None,
        },
    }
}

/// One login attempt, distinct from every other — including a later attempt
/// at the same [`LoginTarget`].
///
/// The target alone used to identify the pending login, which held only while
/// a target could have one attempt at a time. Taking one over
/// ([`reclick_restarts`]) breaks that: the replaced attempt's background wait
/// is still running and resolves against whatever is pending *then*. Keyed on
/// the target it would claim its own replacement's slot, report that login's
/// failure, and leave the attempt the user is waiting on orphaned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) struct LoginAttempt(u64);

/// Mint the next attempt id. Process-wide, because the login slot is: two
/// windows must not mint the same id for different attempts.
pub(in crate::workspace) fn next_login_attempt() -> LoginAttempt {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LoginAttempt(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Whether `attempt`'s finish callback owns what is currently pending.
///
/// `Preparing` is deliberately not claimable: no process exists for it yet, so
/// a finish callback cannot belong to one.
pub(in crate::workspace) fn claims_pending(pending: &PendingLogin, attempt: LoginAttempt) -> bool {
    matches!(
        pending,
        PendingLogin::InProgress { attempt: current, .. } if *current == attempt
    )
}

/// Whether a click on sign-in for `target` should take over the attempt
/// already in flight rather than be refused as a duplicate.
///
/// True only when the pending login is for the *same* target. Clicking sign-in
/// again is the user saying the attempt they started is not going to finish —
/// the browser opened and they closed it, or it never appeared. daruda cannot
/// see that: the login process outlives the browser (its OAuth callback server
/// waits on stdin), so the slot stays taken for the whole [`LOGIN_TIMEOUT`] and
/// every later click reports "already signing in" with the only way out parked
/// in the status bar's account menu, which is not where the user is looking.
///
/// A login for a *different* target is not the user's to discard — the slot is
/// single and that attempt is somebody else's intent, so it is still refused.
pub(in crate::workspace) fn reclick_restarts(pending: &PendingLogin, target: LoginTarget) -> bool {
    match pending {
        PendingLogin::None => false,
        PendingLogin::Preparing {
            target: current, ..
        }
        | PendingLogin::InProgress {
            target: current, ..
        } => *current == target,
    }
}

/// Whether a new headless add-account login is safe to start: `true` iff
/// nothing is already pending. Extracted as a pure predicate off
/// [`Workspace::add_managed_account`]'s concurrent-add guard so it's
/// unit-testable without constructing a `Workspace` — a second concurrent
/// login would overwrite `pending_login`'s [`PendingLogin::InProgress`]
/// handle before anything can cancel the first one, leaking that
/// process + its config dir.
pub(in crate::workspace) fn can_start_login(pending: &PendingLogin) -> bool {
    matches!(pending, PendingLogin::None)
}

/// Best-effort resolve a managed-node runtime for `command`, for the
/// headless login spawn (mirrors the ACP session path's
/// `daruda_acp::node::ensure_node` + `NodeRuntime::wrap_command`, but
/// `spawn_login` can't consume `wrap_command`'s JSON-argv rewrite — it
/// tokenizes `command` itself — so this resolves only the PATH-injection
/// half: on a [`daruda_acp::NodeRuntime::Managed`] runtime, a `("PATH",
/// "<node_dir>/bin:$PATH")` pair for `spawn_login`'s `inject_env`, so the
/// child's `Command::new` lookup of `npx`/`node` finds the managed
/// install even when it isn't on the system PATH).
///
/// Returns `None` when `command` doesn't need node at all, the system
/// already has a usable node ([`daruda_acp::NodeRuntime::System`] — the
/// inherited PATH already covers it), or the resolve itself fails (a
/// download/network error) — in every `None` case `spawn_login` still
/// runs with the inherited PATH, and a genuine "no node" failure surfaces
/// as an ordinary [`daruda_agent::accounts::LoginError`] through the
/// normal failure toast rather than being special-cased here.
///
/// **Blocking** — [`daruda_acp::ensure_node`] may download and extract a
/// node runtime on a first-run machine. Callers must run this on
/// [`gpui::BackgroundExecutor`], never the UI update thread.
pub(in crate::workspace) fn resolve_node_path_env(command: &str) -> Option<(String, String)> {
    if !daruda_acp::node::command_needs_node(command) {
        return None;
    }
    let node_install_dir = daruda_store::persistence::node_install_dir();
    match daruda_acp::ensure_node(&node_install_dir, &mut |_| {}) {
        Ok(daruda_acp::NodeRuntime::Managed { node_dir }) => {
            let existing_path = std::env::var("PATH").unwrap_or_default();
            Some((
                "PATH".to_string(),
                format!("{}:{existing_path}", node_dir.join("bin").display()),
            ))
        }
        Ok(daruda_acp::NodeRuntime::System) | Err(_) => None,
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
/// Mirrors `settings_window::sections::accounts`'s private helper of the
/// same shape — not shared across that module boundary for a 5-line
/// helper.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Log an I/O failure without surfacing a toast — mirrors
/// `settings_window::sections::accounts`'s private helper of the same
/// shape (see its doc for why: this module has `Workspace::report_error`
/// available and still chooses to log rather than toast, since a
/// best-effort cleanup failure has no functional impact worth interrupting
/// the user for).
fn log_io_error(title: &str, dedup: &str, err: &std::io::Error) {
    LogWriter::log(
        ErrorReport::new(title)
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .with_context("error", format!("{err}"))
            .dedup(dedup)
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_start_login_none_is_startable() {
        assert!(can_start_login(&PendingLogin::None));
    }

    #[test]
    fn can_start_login_preparing_is_not_startable() {
        let pending = PendingLogin::Preparing {
            target: LoginTarget::Managed {
                id: AccountId::new(),
                recipe: AccountRecipeId::Claude,
            },
            attempt: next_login_attempt(),
            finish: LoginFinish::Add,
        };
        assert!(!can_start_login(&pending));
    }

    mod attempt_identity {
        use super::*;

        fn in_progress(target: LoginTarget, attempt: LoginAttempt) -> PendingLogin {
            let login = spawn_login("/usr/bin/true", &[], &[], Duration::from_secs(5))
                .expect("spawn a trivial process for the test handle");
            PendingLogin::InProgress {
                target,
                attempt,
                ambient_before: None,
                handle: login.handle(),
                finish: LoginFinish::System,
            }
        }

        /// The race a restart opens. Taking over an attempt leaves its
        /// background wait still running, and it resolves against whatever is
        /// pending *then* — which is the replacement, for the same target. Keyed
        /// on the target alone the stale wait claims the new attempt's slot,
        /// clears it, and reports the failure of a login the user already
        /// abandoned; the attempt they are waiting on is then orphaned and dies
        /// silently.
        #[test]
        fn a_replaced_attempt_does_not_claim_its_replacement() {
            let target = LoginTarget::System {
                recipe: AccountRecipeId::Claude,
            };
            let stale = next_login_attempt();
            let live = next_login_attempt();
            assert_ne!(stale, live, "each attempt is its own");

            let pending = in_progress(target, live);
            assert!(!claims_pending(&pending, stale));
            assert!(claims_pending(&pending, live));
        }

        /// Nothing pending is nothing to claim — the ordinary
        /// already-cancelled case.
        #[test]
        fn nothing_pending_is_claimed_by_nobody() {
            assert!(!claims_pending(&PendingLogin::None, next_login_attempt()));
        }

        /// A `Preparing` attempt has not produced a process yet, so a finish
        /// callback cannot belong to it.
        #[test]
        fn a_preparing_attempt_is_not_a_finished_one() {
            let attempt = next_login_attempt();
            let pending = PendingLogin::Preparing {
                target: LoginTarget::System {
                    recipe: AccountRecipeId::Claude,
                },
                attempt,
                finish: LoginFinish::System,
            };
            assert!(!claims_pending(&pending, attempt));
        }
    }

    mod reclick {
        use super::*;

        fn system(recipe: AccountRecipeId) -> LoginTarget {
            LoginTarget::System { recipe }
        }

        fn preparing(target: LoginTarget) -> PendingLogin {
            PendingLogin::Preparing {
                target,
                attempt: next_login_attempt(),
                finish: LoginFinish::System,
            }
        }

        /// The report this exists for: sign in, a browser opens, the user
        /// closes it without finishing. The login process outlives that — its
        /// callback server waits on stdin — so the slot stays taken for the
        /// whole login timeout. Clicking sign-in again is the user saying that
        /// attempt is not going to finish, not asking for a second one.
        #[test]
        fn clicking_sign_in_again_takes_over_the_attempt_it_repeats() {
            let target = system(AccountRecipeId::Claude);
            assert!(reclick_restarts(&preparing(target), target));
        }

        /// A login for something else is not the user's to discard — the slot
        /// is single, so this still has to be refused.
        #[test]
        fn a_login_for_another_target_is_not_taken_over() {
            assert!(!reclick_restarts(
                &preparing(system(AccountRecipeId::Codex)),
                system(AccountRecipeId::Claude)
            ));
            assert!(!reclick_restarts(
                &preparing(LoginTarget::Managed {
                    id: AccountId::new(),
                    recipe: AccountRecipeId::Claude
                }),
                system(AccountRecipeId::Claude)
            ));
        }

        #[test]
        fn nothing_in_flight_is_nothing_to_take_over() {
            assert!(!reclick_restarts(
                &PendingLogin::None,
                system(AccountRecipeId::Claude)
            ));
        }
    }

    #[test]
    fn a_login_target_always_names_its_auth_domain() {
        for recipe in AccountRecipeId::all() {
            assert_eq!(
                LoginTarget::Managed {
                    id: AccountId::new(),
                    recipe
                }
                .recipe(),
                recipe
            );
            assert_eq!(LoginTarget::System { recipe }.recipe(), recipe);
        }
    }

    /// A system login writes into the user's real home. Deleting it on cancel
    /// would destroy credentials this app never created — the same reason a
    /// reauth's permanent dir is spared, one step more severe.
    #[test]
    fn only_an_add_login_may_delete_its_directory_on_cancel() {
        assert!(cleanup_dir_on_cancel(LoginFinish::Add));
        assert!(!cleanup_dir_on_cancel(LoginFinish::Reauth));
        assert!(!cleanup_dir_on_cancel(LoginFinish::System));
    }

    /// The slot is single and process-wide, so a second attempt — whatever it
    /// signs into — has to be refused while one is live. Before the login was
    /// keyed at all there was nothing to store for an ambient sign-in, which is
    /// exactly the case where two windows would have raced over one `~/.claude`.
    #[gpui::test]
    fn the_login_slot_admits_one_attempt_at_a_time(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            accounts_global::install_if_absent(cx, AccountsState::default());
            let first = next_login_attempt();
            assert!(accounts_global::begin_login(cx, first));
            assert!(
                !accounts_global::begin_login(cx, next_login_attempt()),
                "a second window must not sign in while one attempt is live"
            );
            accounts_global::finish_login(cx, first);
            let second = next_login_attempt();
            assert!(accounts_global::begin_login(cx, second));
            accounts_global::finish_login(cx, second);
        });
    }

    /// A replaced or orphaned attempt resolves late and must not release the
    /// slot its replacement now holds — the replacement signs into the same
    /// place, so anything keyed on that would free it.
    #[gpui::test]
    fn a_stale_attempt_does_not_free_a_newer_one(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            accounts_global::install_if_absent(cx, AccountsState::default());
            let stale = next_login_attempt();
            let live = next_login_attempt();
            assert!(accounts_global::begin_login(cx, live));
            accounts_global::finish_login(cx, stale);
            assert!(
                accounts_global::login_busy(cx),
                "the live attempt still owns the slot"
            );
            accounts_global::finish_login(cx, live);
            assert!(!accounts_global::login_busy(cx));
        });
    }

    #[test]
    fn resolve_node_path_env_none_when_command_does_not_need_node() {
        assert_eq!(resolve_node_path_env("/usr/local/bin/codex-acp"), None);
    }

    #[test]
    fn can_start_login_in_progress_is_not_startable() {
        // A real (near-instant, no-op) child process is the only way to
        // build a `LoginProcessHandle` — it has no public constructor
        // other than `LoginProcess::handle()`.
        let login = spawn_login("/usr/bin/true", &[], &[], Duration::from_secs(5))
            .expect("spawn a trivial process for the test handle");
        let pending = PendingLogin::InProgress {
            target: LoginTarget::Managed {
                id: AccountId::new(),
                recipe: AccountRecipeId::Claude,
            },
            attempt: next_login_attempt(),
            ambient_before: None,
            handle: login.handle(),
            finish: LoginFinish::Add,
        };
        assert!(!can_start_login(&pending));
        // Best-effort: the process has almost certainly already exited on
        // its own; this just avoids relying on that timing.
        if let PendingLogin::InProgress { handle, .. } = &pending {
            handle.cancel();
        }
    }

    fn agent(id: &str, command: &str) -> daruda_config::AgentDefinition {
        daruda_config::AgentDefinition {
            id: id.to_string(),
            name: id.to_string(),
            launch: daruda_config::AgentLaunch::Raw(command.to_string()),
            default_mode: None,
            default_model: None,
            fold_mode: None,
            tail_window: None,
            display_filter: None,
        }
    }

    #[test]
    fn resolve_login_command_prefers_the_active_agent_of_that_recipe() {
        let agents = vec![
            agent(
                "pinned",
                "npx -y @agentclientprotocol/claude-agent-acp@1.2.3",
            ),
            agent(
                "other",
                "npx -y @agentclientprotocol/claude-agent-acp@latest",
            ),
        ];
        let command = resolve_login_command(&agents, "pinned", AccountRecipeId::Claude);
        assert!(command.starts_with("npx -y @agentclientprotocol/claude-agent-acp@1.2.3"));
        assert!(command.ends_with("--cli auth login --claudeai"));
    }

    #[test]
    fn resolve_login_command_scans_the_catalog_when_the_active_agent_is_another_recipe() {
        let agents = vec![
            agent(
                "claude",
                "npx -y @agentclientprotocol/claude-agent-acp@latest",
            ),
            agent("codex", "npx -y @agentclientprotocol/codex-acp@9.9.9"),
        ];
        let command = resolve_login_command(&agents, "claude", AccountRecipeId::Codex);
        assert_eq!(
            command,
            "npx -y @agentclientprotocol/codex-acp@9.9.9 cli login"
        );
    }

    #[test]
    fn resolve_login_command_falls_back_to_the_builtin_for_an_empty_catalog() {
        let command = resolve_login_command(&[], "", AccountRecipeId::Codex);
        let builtin = daruda_config::AgentDefinition::codex_default();
        let daruda_config::AgentLaunch::Raw(raw) = builtin.launch else {
            panic!("the built-in Codex launch is Raw");
        };
        assert_eq!(command, format!("{raw} cli login"));
    }

    #[test]
    fn resolve_login_command_falls_back_when_the_catalog_is_remote_only() {
        // Ssh/Docker/`{{cwd}}` launches derive to no recipe at all, so none
        // of them can serve the request — the built-in must still answer it.
        let agents = vec![
            daruda_config::AgentDefinition {
                id: "remote-ssh".to_string(),
                name: "remote-ssh".to_string(),
                launch: daruda_config::AgentLaunch::Ssh {
                    adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest"
                        .to_string(),
                    host: "box".to_string(),
                },
                default_mode: None,
                default_model: None,
                fold_mode: None,
                tail_window: None,
                display_filter: None,
            },
            daruda_config::AgentDefinition {
                id: "remote-docker".to_string(),
                name: "remote-docker".to_string(),
                launch: daruda_config::AgentLaunch::Docker {
                    adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest"
                        .to_string(),
                    container: "dev".to_string(),
                },
                default_mode: None,
                default_model: None,
                fold_mode: None,
                tail_window: None,
                display_filter: None,
            },
            agent(
                "remote-cwd",
                "npx -y @agentclientprotocol/claude-agent-acp@latest --cwd {{cwd}}",
            ),
        ];
        let command = resolve_login_command(&agents, "remote-ssh", AccountRecipeId::Claude);
        let daruda_config::AgentLaunch::Raw(raw) =
            daruda_config::AgentDefinition::claude_default().launch
        else {
            panic!("the built-in Claude launch is Raw");
        };
        assert_eq!(command, format!("{raw} --cli auth login --claudeai"));
    }

    mod pane_login {
        use super::*;
        use crate::workspace::main_area::pane::AccountDomain;
        use daruda_store::accounts::AccountSelection;

        fn state_with(id: AccountId, recipe: AccountRecipeId) -> AccountsState {
            let mut st = AccountsState::default();
            st.accounts.push(ManagedAccount {
                id,
                recipe,
                email: None,
                organization: None,
                config_dir: "/x".into(),
                created_at: 0,
                last_authenticated_at: 0,
            });
            st
        }

        /// A managed pane signs back into its own account, in that account's
        /// own domain — read from the row rather than from the pane's agent,
        /// so a pane pointed at another domain's account can't reauthenticate
        /// the wrong credentials.
        #[test]
        fn a_managed_pane_signs_back_into_its_own_account() {
            let id = AccountId::new();
            let st = state_with(id, AccountRecipeId::Codex);
            assert_eq!(
                pane_login_target(
                    AccountSelection::Managed(id),
                    AccountDomain::Exactly(AccountRecipeId::Claude),
                    &st
                ),
                Some(LoginTarget::Managed {
                    id,
                    recipe: AccountRecipeId::Codex
                })
            );
        }

        /// The ambient home is per-domain and the selection names no domain,
        /// so the pane's own agent has to supply it.
        #[test]
        fn a_system_pane_signs_into_its_agents_domain() {
            for recipe in AccountRecipeId::all() {
                assert_eq!(
                    pane_login_target(
                        AccountSelection::SystemDefault,
                        AccountDomain::Exactly(recipe),
                        &AccountsState::default()
                    ),
                    Some(LoginTarget::System { recipe })
                );
            }
        }

        /// An agent with no local auth domain (remote, JSON stdio, or one this
        /// build does not recognize) has no login to run — offering a button
        /// would spawn a command for a domain that was never resolved.
        #[test]
        fn a_pane_with_no_auth_domain_offers_no_login() {
            assert_eq!(
                pane_login_target(
                    AccountSelection::SystemDefault,
                    AccountDomain::Unsupported,
                    &AccountsState::default()
                ),
                None
            );
        }

        /// `Any` is the terminal case: no agent, so no single ambient home to
        /// sign into.
        #[test]
        fn an_unscoped_pane_offers_no_system_login() {
            assert_eq!(
                pane_login_target(
                    AccountSelection::SystemDefault,
                    AccountDomain::Any,
                    &AccountsState::default()
                ),
                None
            );
        }

        /// The row can be deleted from the Settings window while a pane still
        /// points at it; there is nothing left to sign into.
        #[test]
        fn a_deleted_account_offers_no_login() {
            assert_eq!(
                pane_login_target(
                    AccountSelection::Managed(AccountId::new()),
                    AccountDomain::Exactly(AccountRecipeId::Claude),
                    &AccountsState::default()
                ),
                None
            );
        }
    }

    mod billing {
        use super::*;

        /// The two ways the Claude agent advertises signing in, restated here
        /// rather than shared with `daruda_acp`'s own fixture: this asserts a
        /// contract *between* the crates, so it has to describe the agent
        /// independently of the code that parses it.
        ///
        /// Only the fields the assertion reads are kept — the id that decides
        /// the billing kind, and the runnable command whose trailing flags
        /// select the flow.
        fn advertised_claude_logins() -> Vec<daruda_acp::LoginMethod> {
            let response: agent_client_protocol::schema::v1::InitializeResponse =
                serde_json::from_value(serde_json::json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {},
                    "authMethods": [
                        {
                            "id": "claude-ai-login",
                            "name": "Claude Subscription",
                            "type": "terminal",
                            "_meta": {"terminal-auth": {
                                "command": "/opt/node/bin/node",
                                "args": ["/cache/claude-agent-acp", "--cli", "auth",
                                         "login", "--claudeai"]
                            }}
                        },
                        {
                            "id": "console-login",
                            "name": "Anthropic Console",
                            "type": "terminal",
                            "_meta": {"terminal-auth": {
                                "command": "/opt/node/bin/node",
                                "args": ["/cache/claude-agent-acp", "--cli", "auth",
                                         "login", "--console"]
                            }}
                        }
                    ]
                }))
                .expect("the advertised payload parses");
            daruda_acp::parse_login_methods(&response.auth_methods)
        }

        /// The flags that pick a flow: everything after the adapter path the
        /// agent resolved for us.
        fn login_flags(method: &daruda_acp::LoginMethod) -> String {
            method
                .command
                .as_ref()
                .expect("the advertised method carries a runnable command")
                .args[1..]
                .join(" ")
        }

        /// daruda has to sign in on the flow that spends the plan the user
        /// already pays for, and the guard that says which one that is comes
        /// from the agent's own classification rather than a flag spelled out
        /// here. Pointing the login command at the other one would move a user
        /// onto per-token billing, and they would find out on an invoice.
        #[test]
        fn the_claude_login_runs_the_subscription_flow_not_the_metered_one() {
            let methods = advertised_claude_logins();
            let mut safe = methods.iter().filter(|m| m.kind.is_safe_default());
            let subscription = safe.next().expect("one flow bills against the plan");
            assert!(
                safe.next().is_none(),
                "more than one flow claims to be free — which one daruda runs is no longer decidable"
            );
            let metered = methods
                .iter()
                .find(|m| !m.kind.is_safe_default())
                .expect("the agent also advertises a metered flow");

            let command = resolve_login_command(&[], "", AccountRecipeId::Claude);
            assert!(
                command.ends_with(&login_flags(subscription)),
                "the login command must run the subscription flow: {command}"
            );
            assert!(
                !command.contains(&login_flags(metered)),
                "the login command runs the per-token billed flow: {command}"
            );
        }
    }

    mod status_probe {
        use super::*;
        use daruda_agent::accounts::auth_status::AuthStatusFormat;

        fn account(recipe: AccountRecipeId) -> AccountsState {
            let mut st = AccountsState::default();
            st.accounts.push(ManagedAccount {
                id: AccountId::new(),
                recipe,
                email: None,
                organization: None,
                config_dir: "/x".into(),
                created_at: 0,
                last_authenticated_at: 0,
            });
            st
        }

        /// The status read and the login it describes must come from one
        /// adapter, so this shares `resolve_login_command`'s resolution — and
        /// pairs it with the format that adapter's CLI actually prints.
        #[test]
        fn the_status_command_reuses_the_agent_that_serves_the_domain() {
            let agents = vec![agent(
                "pinned",
                "npx -y @agentclientprotocol/claude-agent-acp@1.2.3",
            )];
            let (command, format) =
                resolve_status_command(&agents, "pinned", AccountRecipeId::Claude)
                    .expect("claude reports status");
            assert!(command.starts_with("npx -y @agentclientprotocol/claude-agent-acp@1.2.3"));
            assert!(command.ends_with("--cli auth status --json"));
            assert_eq!(format, AuthStatusFormat::Json);
        }

        /// A domain whose CLI prints a sentence has to be read as one — JSON
        /// parsing of prose yields no method, which is indistinguishable from
        /// being signed out.
        #[test]
        fn each_domain_gets_the_format_its_cli_prints() {
            let (command, format) = resolve_status_command(&[], "", AccountRecipeId::Codex)
                .expect("codex reports status");
            assert!(command.ends_with("cli login status"));
            assert_eq!(format, AuthStatusFormat::Prose);
        }

        /// Nothing here has a stake in a domain with no account and no agent,
        /// and a probe would spawn `npx` — possibly downloading the adapter and
        /// a Node runtime — for a row the user does not care about.
        #[test]
        fn an_untouched_domain_is_not_probed() {
            for recipe in AccountRecipeId::all() {
                assert!(
                    !stake_in_domain(&AccountsState::default(), &[], recipe),
                    "{recipe:?}"
                );
            }
        }

        #[test]
        fn an_account_in_a_domain_is_a_stake_in_it() {
            assert!(stake_in_domain(
                &account(AccountRecipeId::Codex),
                &[],
                AccountRecipeId::Codex
            ));
            assert!(!stake_in_domain(
                &account(AccountRecipeId::Codex),
                &[],
                AccountRecipeId::Claude
            ));
        }

        /// A configured agent counts even with no account yet — that is the
        /// domain whose ambient home the user is signing in to.
        #[test]
        fn a_configured_agent_is_a_stake_in_its_domain() {
            let agents = vec![agent(
                "claude",
                "npx -y @agentclientprotocol/claude-agent-acp@latest",
            )];
            assert!(stake_in_domain(
                &AccountsState::default(),
                &agents,
                AccountRecipeId::Claude
            ));
            assert!(!stake_in_domain(
                &AccountsState::default(),
                &agents,
                AccountRecipeId::Codex
            ));
        }

        /// The built-in fallback keeps a *requested* login startable, but must
        /// not make a passive refresh look like a stake.
        #[test]
        fn the_builtin_fallback_is_not_a_stake() {
            assert!(!stake_in_domain(
                &AccountsState::default(),
                &[],
                AccountRecipeId::Claude
            ));
            // …while the command itself still resolves, via that fallback.
            assert!(resolve_status_command(&[], "", AccountRecipeId::Claude).is_some());
        }
    }

    #[test]
    fn find_duplicate_matches_email_and_org() {
        let mut st = AccountsState::default();
        let id = AccountId::new();
        st.accounts.push(ManagedAccount {
            id,
            recipe: AccountRecipeId::Claude,
            email: Some("a@x.com".into()),
            organization: Some("Org".into()),
            config_dir: "/x".into(),
            created_at: 0,
            last_authenticated_at: 0,
        });
        let claude = AccountRecipeId::Claude;
        assert_eq!(
            find_duplicate(&st, claude, Some("a@x.com"), Some("Org")),
            Some(id)
        );
        // Same email, different org — a distinct account.
        assert_eq!(
            find_duplicate(&st, claude, Some("a@x.com"), Some("Other")),
            None
        );
        assert_eq!(
            find_duplicate(&st, claude, Some("b@x.com"), Some("Org")),
            None
        );
        // Same identity in another auth domain is separate credentials.
        assert_eq!(
            find_duplicate(&st, AccountRecipeId::Codex, Some("a@x.com"), Some("Org")),
            None
        );
    }
}
