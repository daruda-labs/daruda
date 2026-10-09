//! Workspace account queries and preparation before starting a pane.

mod model;

use super::*;
pub(in crate::workspace) use model::{
    AccountDomain, AccountPane, FocusedAccount, resolve_pane_account,
};
pub(super) use model::{apply_account_env, resolve_focused_account};

impl Workspace {
    /// Resolve the focused pane's effective account — the usage-cache key,
    /// auth domain and config dir behind it ([`FocusedAccount`]). Gathers
    /// the focused pane's selection ([`AccountSelection::SystemDefault`]
    /// when the pane kind doesn't track an account at all, or there is no
    /// live pane at `focused_pane_id`) and delegates the resolution to
    /// [`resolve_focused_account`].
    pub(in crate::workspace) fn focused_account(&self) -> FocusedAccount {
        let focused_pane_id = self.active_runtime().focused_pane_id;
        let selection = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_pane_id)
            .and_then(Pane::account_selection)
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault);

        resolve_focused_account(selection, &self.accounts, &self.data_dir)
    }

    /// The focused pane as the account layer sees it. Every caller
    /// needs the resulting [`AccountDomain`], so the derivation lives here
    /// rather than being rebuilt per surface.
    pub(in crate::workspace) fn focused_account_pane(&self) -> AccountPane {
        self.account_pane_for(self.active_runtime().focused_pane_id)
    }

    /// How the account layer sees one pane by id — the cross-lane form of
    /// [`Self::focused_account_pane`], for a decision made about a specific
    /// pane rather than about whatever currently has focus (a failed pane's
    /// re-login button acts on the pane that failed, which need not be the
    /// focused one by the time it is clicked).
    ///
    /// Answered from the wrapper caches, never the view: the status bar asks
    /// on every frame, including frames drawn behind Settings.
    pub(in crate::workspace) fn account_pane_for(
        &self,
        pane_id: crate::workspace::main_area::PaneId,
    ) -> AccountPane {
        match self.agent_chat_identity(pane_id) {
            Some((agent_id, cwd)) => AccountPane::AgentChat {
                launch: self.agent_launch_for(agent_id).map(|spec| spec.launch),
                is_remote: matches!(cwd, Some(PaneCwd::Remote(_))),
            },
            None => AccountPane::Terminal,
        }
    }

    /// The account a *freshly created* pane is seeded with: `recipe`'s
    /// configured default, else [`AccountSelection::SystemDefault`]. Seeding
    /// explicitly at creation is what lets resolve-time lookups avoid a
    /// default fallback (see [`resolve_pane_account`]'s doc).
    ///
    /// `recipe: None` is the terminal case — no agent, so no auth domain whose
    /// default could apply; it stays on the ambient environment until the user
    /// picks an account by hand.
    pub(in crate::workspace) fn default_account_selection_for_new_pane(
        &self,
        recipe: Option<daruda_store::accounts::AccountRecipeId>,
    ) -> daruda_store::accounts::AccountSelection {
        recipe
            .and_then(|r| self.accounts.default_account(r))
            .map(|a| daruda_store::accounts::AccountSelection::Managed(a.id))
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault)
    }

    /// Materialize a managed account's config dir before the shell that will
    /// read it exists — Codex symlinks its `CODEX_HOME` in, Claude mirrors the
    /// shared MCP servers.
    ///
    /// Runs inline rather than on the background executor: `spawn_pty` below it
    /// is synchronous, so an off-thread prep would race whatever gets typed (or
    /// programmatically sent) into the fresh pane. The cost is bounded local FS
    /// work on the same order as the PTY spawn it precedes, once per pane
    /// creation — measured at ~8 ms (Claude) / ~0.4 ms (Codex) in a debug build.
    ///
    /// A failure is reported but not fatal, unlike the agent-chat connect: the
    /// config-dir env var is injected either way, so the pane still runs under
    /// the account the user picked — only its mirrored extras are degraded, and
    /// a shell has uses beyond the agent.
    pub(in crate::workspace) fn prepare_account_dir(
        &mut self,
        prepared: &PreparedAccount,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) =
            daruda_agent::accounts::recipe_for(prepared.recipe).prepare_dir(&prepared.config_dir)
        {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::settings::accounts_prepare_dir_failed(),
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("dir", prepared.config_dir.display().to_string())
                .with_context("error", format!("{e}"))
                .dedup("account.pane.prepare_dir_failed")
                .build(),
                cx,
            );
        }
    }

    /// [`resolve_pane_account`] against this window's account cache and
    /// data directory — the pair every pane-spawn path passes together.
    pub(in crate::workspace) fn resolve_account(
        &self,
        selection: daruda_store::accounts::AccountSelection,
        domain: AccountDomain,
    ) -> Option<PreparedAccount> {
        resolve_pane_account(&self.accounts, &self.data_dir, selection, domain)
    }
}
