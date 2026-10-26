//! Reconnect only the failed panes unblocked by a successful login.

use crate::workspace::{Workspace, main_area::pane_tree::PaneId};
use daruda_chat::view::AgentSessionStatus;
use gpui::Context;

impl Workspace {
    /// Reconnect every pane a successful login into `target` has actually
    /// unblocked — the tail of both reauthenticate flows.
    ///
    /// Without this a user who signs in from a failure banner is left looking
    /// at the same error, with a second button to press for the thing they
    /// just asked for. Scoped by [`login_revives_pane`] rather than applied to
    /// every failed pane: a login fixes only the panes that run on the
    /// credentials it wrote.
    ///
    /// Only reaches panes whose *connection* failed, which is deliberately the
    /// narrower half. An expired login usually surfaces as a **turn** failure
    /// instead — the agent creates the session without checking credentials
    /// and refuses at the first real request — and such a pane is still
    /// `Connected`, so it is left alone here: whether a running adapter picks
    /// up freshly written credentials without a reconnect is not something
    /// this has established, and tearing down a live session to find out would
    /// cost the user their conversation on a guess. That pane keeps its own
    /// sign-in button and its next prompt shows whether the login took.
    pub(in crate::workspace) fn reconnect_panes_after_login(
        &mut self,
        target: daruda_agent::accounts::LoginTarget,
        cx: &mut Context<Self>,
    ) {
        // Collected before any reconnect runs: `retry_agent_chat_connect`
        // leases each view, and holding a read across that would re-enter the
        // same entity (AGENTS.md Pitfall #5).
        let revived: Vec<PaneId> = self
            .every_agent_chat()
            .map(|(id, _)| (id, Some(self.agent_chat_account_selection(id))))
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|&(pane_id, selection)| {
                let pane_target = selection.and_then(|selection| {
                    let domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
                        &self.account_pane_for(pane_id),
                    );
                    crate::workspace::account_login_ops::pane_login_target(
                        selection,
                        domain,
                        &self.accounts,
                    )
                });
                let remedy =
                    self.agent_chat_view(pane_id)
                        .and_then(|view| match &view.read(cx).status() {
                            AgentSessionStatus::Error { remedy, .. } => Some(*remedy),
                            _ => None,
                        });
                login_revives_pane(target, pane_target, remedy)
            })
            .map(|(pane_id, _)| pane_id)
            .collect();

        for pane_id in revived {
            self.retry_agent_chat_connect(pane_id, cx);
        }
    }
}

/// Whether a successful login into `target` should reconnect a pane that
/// resolves to `pane_target` and last failed with `remedy` (`None` when its
/// connection did not fail).
///
/// Both conditions are load-bearing. Matching the target keeps the sweep off
/// panes running on other credentials, which this login did nothing for. And
/// only `Reauthenticate` is a failure a fresh sign-in actually clears — an
/// organization-blocked account reconnects into the identical refusal, so
/// retrying it automatically would spend the user's attention on a loop
/// dressed up as progress.
pub(super) fn login_revives_pane(
    target: daruda_agent::accounts::LoginTarget,
    pane_target: Option<daruda_agent::accounts::LoginTarget>,
    remedy: Option<daruda_acp::Remedy>,
) -> bool {
    pane_target == Some(target) && remedy == Some(daruda_acp::Remedy::Reauthenticate)
}

#[cfg(test)]
mod tests;
