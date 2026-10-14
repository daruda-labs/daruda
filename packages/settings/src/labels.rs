//! Labels for configured values that surfaces outside Settings show too, so
//! every one of them names the same value the same way.

use daruda_project::lane::session_host::SessionHostError;
use daruda_store::accounts::AccountRecipeId;

use crate::strings as s;

/// An auth domain's display name.
pub fn account_recipe_label(recipe: AccountRecipeId) -> String {
    s::settings::account_recipe_label(recipe)
}

/// A model choice's display label; the default entry names the model it
/// resolves to.
pub fn agent_model_choice_label(value: &str, name: &str, description: Option<&str>) -> String {
    s::agent_chat::agent_model_choice_label(value, name, description)
}

/// A keep-awake mode's name, shared by the Settings picker and the status bar.
pub fn keep_awake_mode_label(mode: daruda_config::KeepAwakeMode) -> String {
    match mode {
        daruda_config::KeepAwakeMode::Off => s::settings::keep_awake_off(),
        daruda_config::KeepAwakeMode::Agent => s::settings::keep_awake_agent(),
        daruda_config::KeepAwakeMode::On => s::settings::keep_awake_on(),
    }
}

/// Why a session host is refused.
pub fn host_error(err: SessionHostError) -> String {
    s::session_host::host_error(err)
}
