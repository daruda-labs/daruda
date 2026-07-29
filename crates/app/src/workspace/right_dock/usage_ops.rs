//! Usage-tab domain switcher state — the manual pick the switcher tab row
//! writes when the focused pane's agent domain is ambiguous (a terminal, or
//! an agent daruda can't resolve a domain for). See
//! `ClaudeContext::usage_domain_override` for why this is volatile.

use daruda_store::accounts::AccountRecipeId;
use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Record the switcher's manual pick. Ignored by the renderer while the
    /// focused pane names its domain exactly — see
    /// `right_dock::usage::resolve_displayed_domain`.
    pub(in crate::workspace) fn set_usage_domain_override(
        &mut self,
        recipe: AccountRecipeId,
        cx: &mut Context<Self>,
    ) {
        if self.claude.usage_domain_override != Some(recipe) {
            self.claude.usage_domain_override = Some(recipe);
            cx.notify();
        }
    }
}

// Tested alongside the snapshot-threading cases in `workspace/tests/accounts.rs`,
// which already has access to the `build_workspace` test harness this setter
// needs (a full windowed `Workspace`) — see
// `set_usage_domain_override_updates_the_field` there.
