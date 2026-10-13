//! Tests for the public `daruda_agent::providers` API.

use daruda_agent::providers::*;
use daruda_store::accounts::AccountRecipeId;

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
