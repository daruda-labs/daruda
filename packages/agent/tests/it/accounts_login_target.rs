//! Tests for the public `daruda_agent::accounts::login_target` API.

use daruda_agent::accounts::login_target::*;
use daruda_store::accounts::{AccountId, AccountRecipeId};

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
