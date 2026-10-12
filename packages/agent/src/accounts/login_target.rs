//! The home a login signs into — the key a login flow, the auth-status cache
//! and the Settings rows all track it by.

use daruda_store::accounts::{AccountId, AccountRecipeId};

/// Whose credentials a headless login writes into — the key every part of
/// the flow is tracked by: the app's pending login, its process-wide
/// login slot, and each finish path's staleness guard.
///
/// A system login has no [`AccountId`] at all (there is no `accounts.json`
/// row for the ambient home), which is why this is an enum rather than an
/// `AccountId` with a sentinel value: a fake id would make an unreal account
/// representable everywhere a real one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginTarget {
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
    pub fn recipe(self) -> AccountRecipeId {
        match self {
            LoginTarget::Managed { recipe, .. } | LoginTarget::System { recipe } => recipe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_kinds_of_home_name_their_auth_domain() {
        let managed = LoginTarget::Managed {
            id: AccountId::new(),
            recipe: AccountRecipeId::Codex,
        };
        assert_eq!(managed.recipe(), AccountRecipeId::Codex);
        let system = LoginTarget::System {
            recipe: AccountRecipeId::Claude,
        };
        assert_eq!(system.recipe(), AccountRecipeId::Claude);
    }
}
