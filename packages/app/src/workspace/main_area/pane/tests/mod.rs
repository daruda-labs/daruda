mod accounts;
mod model;
mod polling;

use super::*;

const CAP_30FPS: Duration = Duration::from_millis(33);

/// One managed account of `recipe`, registered as that domain's default.
fn accounts_with(
    recipe: daruda_store::accounts::AccountRecipeId,
) -> (
    daruda_store::accounts::AccountId,
    daruda_store::accounts::AccountsState,
) {
    use daruda_store::accounts::{AccountId, AccountsState, ManagedAccount};
    let id = AccountId::new();
    let mut st = AccountsState::default();
    st.accounts.push(ManagedAccount {
        id,
        recipe,
        email: None,
        organization: None,
        config_dir: std::path::PathBuf::from("/x"),
        created_at: 0,
        last_authenticated_at: 0,
    });
    st.default_by_recipe.insert(recipe, id);
    (id, st)
}
