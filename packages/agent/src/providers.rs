//! Central registry for auth-domain provider capabilities.
//!
//! Capability traits stay split by lifecycle (`AccountRecipe`, `UsageSource`,
//! `ActivitySource`), but this registry keeps the provider-to-capability wiring
//! in one exhaustive match. Adding an `AccountRecipeId` variant must land here
//! with every required implementation instead of scattering independent match
//! arms across the crate.

use daruda_store::accounts::AccountRecipeId;

use crate::accounts::AccountRecipe;
use crate::activity::ActivitySource;
use crate::usage::UsageSource;

#[derive(Clone, Copy)]
pub struct ProviderIntegration {
    pub id: AccountRecipeId,
    pub account: &'static dyn AccountRecipe,
    pub usage: &'static dyn UsageSource,
    pub activity: &'static dyn ActivitySource,
}

/// All capabilities for `id`. Total and exhaustive over `AccountRecipeId`.
pub fn integration_for(id: AccountRecipeId) -> ProviderIntegration {
    match id {
        AccountRecipeId::Claude => ProviderIntegration {
            id,
            account: &crate::accounts::claude::ClaudeRecipe,
            usage: &crate::usage::claude::ClaudeUsage,
            activity: &crate::activity::ClaudeActivity,
        },
        AccountRecipeId::Codex => ProviderIntegration {
            id,
            account: &crate::accounts::codex::CodexRecipe,
            usage: &crate::usage::codex::CodexUsage,
            activity: &crate::codex_activity::CodexActivity,
        },
    }
}

/// Every registered provider in display order.
pub fn all() -> impl Iterator<Item = ProviderIntegration> {
    AccountRecipeId::all().map(integration_for)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_id_has_a_complete_provider_integration() {
        let ids: Vec<_> = all()
            .map(|provider| {
                assert_eq!(provider.account.id(), provider.id);
                provider.id
            })
            .collect();
        assert_eq!(ids.len(), AccountRecipeId::count());
    }
}
