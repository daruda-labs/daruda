//! Where a headless login lands: folding its outcome back into the account
//! store (add / reauth / system), guarded so a stale callback changes nothing.

use super::*;

impl Workspace {
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
        let ambient_before = match &self.login.pending {
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
                s::settings::accounts_login_denied_detail(),
                cx,
            ),
            LoginOutcome::TimedOut => self.finish_login_failed(
                recipe_id,
                config_dir,
                s::settings::accounts_login_timed_out_detail(),
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
    pub(super) fn finish_login_success(
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
                s::settings::accounts_login_no_credentials_detail(),
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
                // The list could not take the account (unwritable, or a file
                // this build refuses to overwrite): nothing references the
                // fresh home, so it goes the way a failed login's does.
                Err(e) => {
                    self.finish_login_failed(recipe_id, config_dir, e.to_string(), cx);
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
            s::settings::accounts_login_already_exists()
        } else {
            s::settings::accounts_login_added()
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
    pub(super) fn finish_login_failed(
        &mut self,
        recipe: AccountRecipeId,
        config_dir: PathBuf,
        detail: String,
        cx: &mut Context<Self>,
    ) {
        cleanup_account_dir(recipe, &config_dir);
        self.report_error(
            ErrorReport::new(s::settings::accounts_login_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
    }

    /// Report a managed login that replaced the user's own ambient sign-in.
    ///
    /// Nothing is written back: the store is the user's, and a restore built on
    /// an unverified premise could corrupt a working sign-in. Saying so is what
    /// turns a silent replacement into one the user can act on — their
    /// `SystemDefault` panes are now running as the managed account.
    pub(super) fn report_ambient_clobber(
        &mut self,
        before: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = before else {
            return;
        };
        if daruda_agent::accounts::credentials::system_credentials_digest()
            .is_none_or(|after| after == before)
        {
            return;
        }
        self.report_error(
            ErrorReport::new(s::settings::accounts_ambient_login_replaced())
                .message(s::settings::accounts_ambient_login_replaced_detail())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("account.login.ambient_clobbered")
                .build(),
            cx,
        );
    }

    /// Take ownership of a resolved login on this `Workspace`: `true` when
    /// `target` is still the pending one, having cleared both the per-window
    /// state and the process-wide slot; `false` when this callback is stale.
    ///
    /// A stale callback means the login was already handled — cancelled and
    /// cleaned up, or superseded by a newer one — so running its finish path
    /// anyway would either double-clean an already-removed dir or clobber an
    /// `InProgress` it knows nothing about.
    pub(super) fn claim_finished_login(
        &mut self,
        attempt: LoginAttempt,
        cx: &mut Context<Self>,
    ) -> bool {
        if !claims_pending(&self.login.pending, attempt) {
            return false;
        }
        self.login.pending = PendingLogin::None;
        accounts_global::finish_login(cx, attempt);
        true
    }

    /// Picks up a system login's result. Unlike the managed flows there is
    /// no row to file or update and no directory to clean up either way —
    /// the credentials landed in the user's own home (or the Keychain) and
    /// that *is* the durable state. All that remains is telling the user,
    /// and letting the panes that were failing on the old credentials try
    /// again.
    pub(super) fn finish_system_login(
        &mut self,
        attempt: LoginAttempt,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        // Read before the claim clears it — the reconnect sweep below needs to
        // know which credentials this login wrote.
        let PendingLogin::InProgress { target, .. } = self.login.pending else {
            return;
        };
        if !self.claim_finished_login(attempt, cx) {
            return;
        }

        match outcome {
            LoginOutcome::Success => {
                self.report_error(
                    ErrorReport::new(s::settings::accounts_reauth_added())
                        .severity(ErrorSeverity::Info)
                        .build(),
                    cx,
                );
                self.probe_auth_status(target, true, cx);
                self.reconnect_panes_after_login(target, cx);
                cx.notify();
            }
            LoginOutcome::Denied => {
                self.finish_reauth_failed(s::settings::accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => {
                self.finish_reauth_failed(s::settings::accounts_login_timed_out_detail(), cx)
            }
            LoginOutcome::Failed(detail) => self.finish_reauth_failed(detail, cx),
        }
    }

    /// Picks up a reauthenticate-account login's result on this
    /// `Workspace`, once [`Self::reauthenticate_account`]'s background
    /// `wait()` resolves. Mirrors [`Self::finish_login`]'s staleness
    /// guard and dispatch shape.
    pub(super) fn finish_reauth(
        &mut self,
        account_id: AccountId,
        attempt: LoginAttempt,
        config_dir: PathBuf,
        recipe: AccountRecipeId,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let ambient_before = match &self.login.pending {
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
                self.finish_reauth_failed(s::settings::accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => {
                self.finish_reauth_failed(s::settings::accounts_login_timed_out_detail(), cx)
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
    pub(super) fn finish_reauth_success(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        recipe_id: AccountRecipeId,
        cx: &mut Context<Self>,
    ) {
        let recipe = recipe_for(recipe_id);
        if !recipe.has_credentials(&config_dir) {
            self.finish_reauth_failed(s::settings::accounts_reauth_no_credentials_detail(), cx);
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
                    self.finish_reauth_failed(e.to_string(), cx);
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
            ErrorReport::new(s::settings::accounts_reauth_added())
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
    pub(super) fn finish_reauth_failed(&mut self, detail: String, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::settings::accounts_reauth_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
    }
}
