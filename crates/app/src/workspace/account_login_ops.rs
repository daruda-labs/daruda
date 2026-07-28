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

use gpui::{Context, Window};

use daruda_claude::accounts::{LoginOutcome, account_config_dir, recipe_for, spawn_login};
use daruda_store::accounts::{AccountId, AccountRecipeId, AccountsState, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::{AddManagedAccount, PendingLogin, ReauthenticateAccount, accounts_global};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;

/// Timeout for a headless add-account login before it's treated as hung
/// and cancelled. Generous — the flow blocks on the user completing OAuth
/// consent by hand in a system browser window, not on anything this app
/// controls the pace of.
///
/// `pub(in crate::workspace)`: `Workspace::new_with_project`'s startup
/// orphan sweep (`daruda_claude::accounts::sweep_orphan_dirs`) passes this
/// same constant as its `grace` window, so the two can't drift — see that
/// call site's comment for why a login's own timeout is exactly the right
/// sweep grace period.
pub(in crate::workspace) const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Which of the two headless login flows a [`PendingLogin::InProgress`] is
/// tracking. Carried alongside `account_id` so
/// [`Workspace::cancel_pending_login`] can tell apart an add-account
/// login's throwaway config dir (safe to delete on cancel) from a
/// reauthenticate login's real, permanent one (must survive a cancel) —
/// see [`cleanup_dir_on_cancel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum LoginMode {
    Add,
    Reauth,
}

/// Whether cancelling a pending login should remove its `account_id`'s
/// config dir (+ scoped Keychain item): only for [`LoginMode::Add`], whose
/// `account_id` names a throwaway dir that never became a kept
/// [`ManagedAccount`]. For [`LoginMode::Reauth`] the same `account_id`
/// names an existing account's real credentials — deleting them on cancel
/// would be permanent data loss for a still-good account (see
/// [`Workspace::reauthenticate_account`]'s doc for why
/// [`Workspace::finish_reauth_failed`] already avoids this on the
/// login-failed path; cancel must match it).
pub(in crate::workspace) fn cleanup_dir_on_cancel(mode: LoginMode) -> bool {
    matches!(mode, LoginMode::Add)
}

/// How [`Workspace::spawn_login_flow`] finishes a resolved login — the one
/// axis on which the add and reauth flows differ once the shared
/// spawn/poll machinery is done: which finish method the outcome dispatches
/// into (add files a new [`ManagedAccount`]; reauth updates the existing row
/// by id) and the title of the spawn-failure toast. The [`LoginMode`] (which
/// drives cleanup-on-cancel / cleanup-on-spawn-failure) is *derived* from
/// the variant ([`Self::mode`]) rather than passed alongside, so the two
/// can't disagree. The auth domain is not carried here either — it comes
/// from the agent actually running the login, threaded through
/// [`Workspace::spawn_login_flow`].
#[derive(Debug, Clone, Copy)]
pub(in crate::workspace) enum LoginFinish {
    Add,
    Reauth,
}

impl LoginFinish {
    fn mode(self) -> LoginMode {
        match self {
            LoginFinish::Add => LoginMode::Add,
            LoginFinish::Reauth => LoginMode::Reauth,
        }
    }

    /// Dispatch a resolved `outcome` into the flow-specific finish method.
    /// `recipe` is the auth domain the login command signed into — the add
    /// flow files the new account under it; reauth's row already has one.
    fn dispatch(
        self,
        ws: &mut Workspace,
        account_id: AccountId,
        config_dir: PathBuf,
        recipe: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Workspace>,
    ) {
        match self {
            LoginFinish::Add => ws.finish_login(account_id, config_dir, recipe, outcome, cx),
            LoginFinish::Reauth => ws.finish_reauth(account_id, config_dir, recipe, outcome, cx),
        }
    }

    /// Title for the toast shown when `spawn_login` itself fails to launch
    /// the child (distinct from a launched-then-failed login, which the
    /// finish methods report).
    fn spawn_error_title(self) -> String {
        match self {
            LoginFinish::Add => s::settings_accounts_login_failed(),
            LoginFinish::Reauth => s::settings_accounts_reauth_failed(),
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
    ) -> Option<String> {
        let active_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        resolve_login_command(&self.agents, &active_id, requested)
    }

    /// Whether a headless add-account login is currently in flight —
    /// drives the status-bar dropdown's spinner + Cancel row in place of
    /// "+ Add account" (see [`PendingLogin`]).
    pub(in crate::workspace) fn is_login_pending(&self) -> bool {
        !can_start_login(&self.pending_login)
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
    /// that one resolution, so they can't name different domains. Rejects
    /// up front (toast, no process spawned) only when no login command
    /// resolves at all.
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
        if !can_start_login(&self.pending_login) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        let Some(command) = self.login_command_for_recipe(recipe) else {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_unavailable())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.login_unavailable")
                    .build(),
                cx,
            );
            return;
        };

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
            account_id,
            config_dir,
            command,
            recipe,
            LoginFinish::Add,
            cx,
        );
    }

    /// Shared spawn/poll/finish machinery behind both headless login flows
    /// ([`Self::add_managed_account`] + [`Self::reauthenticate_account`]).
    /// The caller has already run its own front-guards (busy /
    /// command-available / account-exists) and provisioned `config_dir`;
    /// this owns everything after: stash `Preparing`, resolve a managed-node
    /// PATH off-thread, spawn the login child, track it as `InProgress`, and
    /// await its `wait()` on the background executor — dispatching the
    /// result through `finish`.
    ///
    /// `finish` ([`LoginFinish`]) is the single axis the two flows differ
    /// on: it picks the finish method and spawn-failure toast, and its
    /// derived [`LoginMode`] decides whether a spawn failure or closed-window
    /// race removes `config_dir` (add's dir is throwaway → yes; reauth's is
    /// the account's permanent dir → no, via [`cleanup_dir_on_cancel`]).
    fn spawn_login_flow(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        command: String,
        recipe_id: AccountRecipeId,
        finish: LoginFinish,
        cx: &mut Context<Self>,
    ) {
        let recipe = daruda_claude::accounts::recipe_for(recipe_id);
        let env =
            daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env());
        let mode = finish.mode();

        // Stash `Preparing` before the async node-resolve gap even starts —
        // it blocks a second concurrent login (`can_start_login`) and gives
        // the status-bar dropdown its spinner immediately, rather than only
        // once a process actually exists. See `PendingLogin::Preparing`'s
        // doc for why this state exists at all (`resolve_node_path_env` is
        // blocking and may download Node.js, so it can't run inline here on
        // the UI thread).
        self.pending_login = PendingLogin::Preparing {
            account_id,
            recipe: recipe_id,
            mode,
        };
        cx.notify();

        let resolve_command = command.clone();
        cx.spawn(async move |this, cx| {
            let path_pair = cx
                .background_executor()
                .spawn(async move { resolve_node_path_env(&resolve_command) })
                .await;

            let closed_window_cleanup_path = config_dir.clone();
            let updated = this.update(cx, move |ws, cx| {
                // The user may have cancelled during the resolve (or, in
                // principle, a newer call superseded it) — bail without
                // touching anything if this `Preparing` isn't the one that
                // was staged above.
                let is_current = matches!(
                    &ws.pending_login,
                    PendingLogin::Preparing { account_id: current, .. }
                        if *current == account_id
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
                            account_id,
                            recipe: recipe_id,
                            handle,
                            mode,
                        };
                        cx.notify();

                        let wait_config_dir = config_dir.clone();
                        cx.spawn(async move |this, cx| {
                            let probe_dir = config_dir.clone();
                            let outcome = cx
                                .background_executor()
                                .spawn(async move {
                                    // The domain decides what "finished"
                                    // means; the probe it may need is bound
                                    // here, where the config dir is known.
                                    let landed =
                                        move || recipe_for(recipe_id).has_credentials(&probe_dir);
                                    let policy = recipe_for(recipe_id)
                                        .login_completion()
                                        .with_probe(&landed);
                                    login_process.wait(policy)
                                })
                                .await;
                            let cleanup_path = wait_config_dir.clone();
                            if this
                                .update(cx, |ws, cx| {
                                    finish.dispatch(
                                        ws,
                                        account_id,
                                        wait_config_dir,
                                        recipe_id,
                                        outcome,
                                        cx,
                                    )
                                })
                                .is_err()
                            {
                                // Workspace window closed mid-login: the
                                // finish path's own cleanup never ran. Run
                                // it here only for the add flow — a reauth's
                                // `config_dir` is the account's permanent
                                // one and must survive (see `LoginFinish`).
                                if cleanup_dir_on_cancel(mode) {
                                    cleanup_account_dir(recipe_id, &cleanup_path);
                                }
                            }
                        })
                        .detach();
                    }
                    Err(e) => {
                        if cleanup_dir_on_cancel(mode) {
                            cleanup_account_dir(recipe_id, &config_dir);
                        }
                        ws.pending_login = PendingLogin::None;
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

            if updated.is_err() && cleanup_dir_on_cancel(mode) {
                // Workspace window closed during the node resolve, before
                // `spawn_login` ever ran: nothing was spawned to leak a
                // process, but the add flow's throwaway config dir still
                // needs the same cleanup the wait-path's closed-window
                // fallback does (reauth's permanent dir must not — no-op).
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
        config_dir: PathBuf,
        recipe_id: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let is_current = matches!(
            &self.pending_login,
            PendingLogin::InProgress { account_id: current, .. } if *current == account_id
        );
        if !is_current {
            return;
        }
        self.pending_login = PendingLogin::None;

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

        // Re-read the on-disk state and apply this login's change to it,
        // rather than overwriting accounts.json with a possibly-stale
        // in-memory snapshot: another window (Settings, or a second
        // Workspace) may have written the file since this login started,
        // and a full-overwrite from `self.accounts` would silently destroy
        // whatever it added. This load-mutate-save narrows the cross-window
        // race to the load→save gap below (vs. the prior full-snapshot
        // overwrite, which clobbered unconditionally) — it is not fully
        // atomic: `save_accounts_in` renames into place but takes no file
        // lock, so two windows racing this exact window can still each
        // read the same pre-write state and one's delta can still overwrite
        // the other's. A real fix needs a file lock (fs4 is already a
        // dependency but unused here) — tracked as a follow-up, not done
        // in this pass.
        let mut state =
            daruda_store::accounts::load_accounts_in(&self.data_dir).unwrap_or_default();

        let is_duplicate = match find_duplicate(
            &state,
            identity.email.as_deref(),
            identity.organization.as_deref(),
        ) {
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
        };

        if let Err(e) = daruda_store::accounts::save_accounts_in(&self.data_dir, &state) {
            log_io_error(
                "Failed to save accounts.json after add-account login",
                "account.add.save_failed",
                &e,
            );
        }
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
        let (account_id, recipe, handle, mode) =
            match std::mem::replace(&mut self.pending_login, PendingLogin::None) {
                PendingLogin::None => return,
                PendingLogin::Preparing {
                    account_id,
                    recipe,
                    mode,
                } => (account_id, recipe, None, mode),
                PendingLogin::InProgress {
                    account_id,
                    recipe,
                    handle,
                    mode,
                } => (account_id, recipe, Some(handle), mode),
            };
        if let Some(handle) = handle {
            handle.cancel();
        }
        if cleanup_dir_on_cancel(mode) {
            cleanup_account_dir(recipe, &account_config_dir(&self.data_dir, account_id));
        }
        cx.notify();
    }

    /// Action handler for [`ReauthenticateAccount`]. Thin shim, mirroring
    /// [`Self::on_add_managed_account`].
    pub(in crate::workspace) fn on_reauthenticate_account(
        &mut self,
        action: &ReauthenticateAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reauthenticate_account(action.0, window, cx);
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_start_login(&self.pending_login) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.login_busy")
                    .build(),
                cx,
            );
            return;
        }

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

        // Scoped to the account's own domain: a login for another domain
        // run against this config dir would overwrite these credentials
        // with a different domain's.
        let Some(command) = self.login_command_for_recipe(recipe) else {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_unavailable())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.login_unavailable")
                    .build(),
                cx,
            );
            return;
        };

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
            account_id,
            config_dir,
            command,
            recipe,
            LoginFinish::Reauth,
            cx,
        );
    }

    /// Picks up a reauthenticate-account login's result on this
    /// `Workspace`, once [`Self::reauthenticate_account`]'s background
    /// `wait()` resolves. Mirrors [`Self::finish_login`]'s staleness
    /// guard and dispatch shape.
    fn finish_reauth(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        recipe: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let is_current = matches!(
            &self.pending_login,
            PendingLogin::InProgress { account_id: current, .. } if *current == account_id
        );
        if !is_current {
            return;
        }
        self.pending_login = PendingLogin::None;

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

        // Apply the reauth to freshly-loaded disk state (see
        // `finish_login_success`'s comment for the same load-mutate-save
        // shape and why it narrows, but does not close, the cross-window
        // race).
        let mut state =
            daruda_store::accounts::load_accounts_in(&self.data_dir).unwrap_or_default();

        let Some(existing) = state.accounts.iter_mut().find(|a| a.id == account_id) else {
            // The account was removed (Settings-window delete) while
            // this reauth was in flight — nothing left to update; the
            // delete flow already removed `config_dir` itself. Still
            // republish the freshly-loaded disk state so this window's
            // mirror matches what's on disk (and any other window's).
            self.accounts = state.clone();
            accounts_global::replace(cx, state);
            cx.notify();
            return;
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

        if let Err(e) = daruda_store::accounts::save_accounts_in(&self.data_dir, &state) {
            log_io_error(
                "Failed to save accounts.json after reauthenticate-account login",
                "account.reauth.save_failed",
                &e,
            );
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
/// still add an account there. `None` only if even the built-in yields no
/// command.
pub(in crate::workspace) fn resolve_login_command(
    agents: &[daruda_config::AgentDefinition],
    active_id: &str,
    requested: AccountRecipeId,
) -> Option<String> {
    let login_args = recipe_for(requested).login_args();
    let signs_into_requested =
        |a: &&daruda_config::AgentDefinition| a.launch.account_recipe() == Some(requested);
    let active = agents
        .iter()
        .find(|a| a.id == active_id)
        .filter(signs_into_requested);
    let from_catalog = || agents.iter().find(signs_into_requested);
    active
        .or_else(from_catalog)
        .map(|a| a.launch.clone())
        .unwrap_or_else(|| builtin_launch_for(requested))
        .login_command(login_args)
}

/// The built-in adapter launch for `recipe` — the fallback when the user's
/// catalog has no agent for that auth domain.
fn builtin_launch_for(recipe: AccountRecipeId) -> daruda_config::AgentLaunch {
    match recipe {
        AccountRecipeId::Claude => daruda_config::AgentDefinition::claude_default().launch,
        AccountRecipeId::Codex => daruda_config::AgentDefinition::codex_default().launch,
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
/// as an ordinary [`daruda_claude::accounts::LoginError`] through the
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

/// Matches an existing managed account with the *same* email *and* the
/// same organization — a duplicate login for an account already tracked.
/// Same email under a *different* organization (e.g. the same person's
/// Claude account under two orgs) is treated as a distinct account, since
/// Claude scopes usage/plan per organization. Two accounts that both have
/// no captured email/org (`None`/`None`) are also considered duplicates of
/// each other under this literal-equality rule — an edge case that
/// shouldn't arise once Plan B logins always capture `oauthAccount`.
pub(in crate::workspace) fn find_duplicate(
    state: &AccountsState,
    email: Option<&str>,
    org: Option<&str>,
) -> Option<AccountId> {
    state
        .accounts
        .iter()
        .find(|a| a.email.as_deref() == email && a.organization.as_deref() == org)
        .map(|a| a.id)
}

/// Best-effort remove `config_dir` — and whatever OS credential store entry
/// `recipe` scopes to it — after a login attempt didn't produce a (kept)
/// managed account: spawn failed, a `Denied`/`TimedOut`/`Failed` outcome, a
/// duplicate dedup, a cancel, or a closed-window race. Each auth domain owns
/// the removal (Claude also drops a Keychain item; Codex must unlink rather
/// than follow the symlinks in its home), so this routes through
/// [`AccountRecipe::cleanup`](daruda_claude::accounts::AccountRecipe::cleanup)
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
            account_id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
            mode: LoginMode::Add,
        };
        assert!(!can_start_login(&pending));
    }

    #[test]
    fn resolve_node_path_env_none_when_command_does_not_need_node() {
        assert_eq!(resolve_node_path_env("/usr/local/bin/codex-acp"), None);
    }

    #[test]
    fn cleanup_dir_on_cancel_add_yes_reauth_no() {
        assert!(cleanup_dir_on_cancel(LoginMode::Add));
        assert!(!cleanup_dir_on_cancel(LoginMode::Reauth));
    }

    #[test]
    fn can_start_login_in_progress_is_not_startable() {
        // A real (near-instant, no-op) child process is the only way to
        // build a `LoginProcessHandle` — it has no public constructor
        // other than `LoginProcess::handle()`.
        let login = spawn_login("/usr/bin/true", &[], &[], Duration::from_secs(5))
            .expect("spawn a trivial process for the test handle");
        let pending = PendingLogin::InProgress {
            account_id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
            handle: login.handle(),
            mode: LoginMode::Add,
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
        let command = resolve_login_command(&agents, "pinned", AccountRecipeId::Claude)
            .expect("active agent signs into Claude");
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
        let command = resolve_login_command(&agents, "claude", AccountRecipeId::Codex)
            .expect("catalog holds a Codex agent");
        assert_eq!(
            command,
            "npx -y @agentclientprotocol/codex-acp@9.9.9 cli login"
        );
    }

    #[test]
    fn resolve_login_command_falls_back_to_the_builtin_for_an_empty_catalog() {
        let command = resolve_login_command(&[], "", AccountRecipeId::Codex)
            .expect("built-in Codex adapter has a login command");
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
            },
            agent(
                "remote-cwd",
                "npx -y @agentclientprotocol/claude-agent-acp@latest --cwd {{cwd}}",
            ),
        ];
        let command = resolve_login_command(&agents, "remote-ssh", AccountRecipeId::Claude)
            .expect("built-in Claude adapter has a login command");
        let daruda_config::AgentLaunch::Raw(raw) =
            daruda_config::AgentDefinition::claude_default().launch
        else {
            panic!("the built-in Claude launch is Raw");
        };
        assert_eq!(command, format!("{raw} --cli auth login --claudeai"));
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
        assert_eq!(find_duplicate(&st, Some("a@x.com"), Some("Org")), Some(id));
        assert_eq!(find_duplicate(&st, Some("a@x.com"), Some("Other")), None); // same email, diff org = distinct
        assert_eq!(find_duplicate(&st, Some("b@x.com"), Some("Org")), None);
    }
}
