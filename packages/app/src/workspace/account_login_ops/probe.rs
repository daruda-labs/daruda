//! Auth-status probes: how each set of credentials daruda can name was
//! signed in, read off the UI thread for the Settings rows.

use super::*;

/// Budget for one auth-status probe. Far shorter than a login: nothing here
/// waits on a human, only on the CLI starting up and printing a line — and a
/// probe that outlives this is one the user is no longer looking at.
const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(30);

impl Workspace {
    /// Read how each set of credentials daruda can name was signed in, and
    /// cache it for the Settings rows.
    ///
    /// Scoped to domains this user actually has a stake in: one with a managed
    /// account, or one some configured agent signs into. A probe runs the
    /// domain's CLI and may first install Node.js and the adapter to do it
    /// ([`adapter_cli::prepare`]), so probing a domain the user has never
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
        for recipe in AccountRecipeId::all() {
            let domain: Vec<LoginTarget> = targets
                .iter()
                .copied()
                .filter(|target| target.recipe() == recipe)
                .collect();
            self.probe_auth_domain(recipe, domain, false, cx);
        }
    }

    /// Whether this user has anything in `recipe`'s auth domain: an account
    /// filed under it, or a configured agent that signs into it.
    ///
    /// Deliberately *not* satisfied by the built-in adapter fallback that
    /// [`resolve_login_cli`] leans on. That fallback exists so a login can
    /// always be started on request; it must not turn a passive refresh into a
    /// download for a domain nothing here uses.
    fn has_stake_in_domain(&self, recipe: AccountRecipeId) -> bool {
        stake_in_domain(&self.accounts, &self.mirrors.agents, recipe)
    }

    /// One reading, off the UI thread.
    ///
    /// The probe prepares and spawns the agent's CLI (an install, at worst),
    /// so it is resolved here and run on the background executor — the same
    /// division the login flow uses. A domain with no confirmed status command
    /// is skipped rather than guessed at.
    ///
    /// `supersede` marks a caller whose reading is newer than anything already
    /// running — a login that just changed these credentials. Without it an
    /// equivalent request is dropped while one is in flight, so reopening
    /// Settings costs nothing.
    pub(super) fn probe_auth_status(
        &mut self,
        target: LoginTarget,
        supersede: bool,
        cx: &mut Context<Self>,
    ) {
        self.probe_auth_domain(target.recipe(), vec![target], supersede, cx);
    }

    /// Read `targets`, all of `recipe`'s domain, on one preparation: installs
    /// do not depend on the account, so each extra account costs a CLI run
    /// rather than another registry round trip under the install lock.
    fn probe_auth_domain(
        &mut self,
        recipe_id: AccountRecipeId,
        targets: Vec<LoginTarget>,
        supersede: bool,
        cx: &mut Context<Self>,
    ) {
        let active_id = self.open_agent_id(None);
        let Some((request, format)) =
            resolve_status_cli(&self.mirrors.agents, &active_id, recipe_id)
        else {
            return;
        };
        let probes: Vec<_> = targets
            .into_iter()
            .filter_map(|target| {
                let ticket = auth_status_global::begin_probe(cx, target, supersede)?;
                let env = match target {
                    LoginTarget::Managed { id, recipe } => {
                        managed_env(recipe, &account_config_dir(&self.data_dir, id))
                    }
                    // The ambient home is read exactly as a pane reads it.
                    LoginTarget::System { .. } => daruda_config::AccountEnv::ambient(),
                };
                Some((ticket, env))
            })
            .collect();
        if probes.is_empty() {
            return;
        }

        cx.spawn(async move |_this, cx| {
            let readings = cx
                .background_executor()
                .spawn(async move { read_domain(&request, format, probes) })
                .await;
            cx.update(|cx| {
                for (ticket, reading) in readings {
                    match reading {
                        Some(reading) => auth_status_global::record(cx, ticket, reading),
                        // Release the scope, or the next probe reads as a
                        // duplicate of one that never answered.
                        None => auth_status_global::abandon_probe(cx, ticket),
                    }
                }
            });
        })
        .detach();
    }
}

/// One domain's readings on one preparation. **Blocking**; prepared first,
/// so an install never counts against a read's timeout or dies to it.
fn read_domain(
    request: &AdapterCliRequest,
    format: daruda_agent::accounts::auth_status::AuthStatusFormat,
    probes: Vec<(auth_status_global::ProbeTicket, daruda_config::AccountEnv)>,
) -> Vec<(
    auth_status_global::ProbeTicket,
    Option<daruda_agent::accounts::auth_status::AuthStatus>,
)> {
    // `adapter_cli::prepare` refuses under test; skipping it here keeps that
    // refusal out of the log.
    let prepared = if cfg!(test) {
        None
    } else {
        let cancellation = daruda_acp::preparation::PreparationCancellation::default();
        match adapter_cli::prepare(request, &cancellation) {
            Ok(prepared) => Some(prepared),
            Err(detail) => {
                LogWriter::log(
                    ErrorReport::new("Could not prepare the auth status probe")
                        .message(detail)
                        .severity(ErrorSeverity::Warning)
                        .dedup("account.auth_probe.prepare")
                        .at(file!(), line!())
                        .build(),
                );
                None
            }
        }
    };
    probes
        .into_iter()
        .map(|(ticket, env)| {
            let reading = prepared
                .as_ref()
                .and_then(|prepared| prepared.for_account(&env).ok())
                .and_then(|cli| {
                    daruda_agent::accounts::auth_status::read_auth_status(
                        &cli.command,
                        format,
                        AUTH_STATUS_TIMEOUT,
                    )
                });
            (ticket, reading)
        })
        .collect()
}
