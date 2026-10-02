use crate::surface::strings as s;

/// Display name of an auth domain — the only place the enum maps to user
/// text, shared by the Settings section and the status-bar dropdown.
pub(crate) fn account_recipe_label(recipe: daruda_store::accounts::AccountRecipeId) -> String {
    match recipe {
        daruda_store::accounts::AccountRecipeId::Claude => {
            s::settings::status_bar_account_provider_claude()
        }
        daruda_store::accounts::AccountRecipeId::Codex => {
            s::settings::status_bar_account_provider_codex()
        }
    }
}

pub(crate) fn advanced_count(count: usize) -> String {
    if count == 1 {
        s::settings::advanced_count_one()
    } else {
        rust_i18n::t!("settings.advanced_count", count = count).into_owned()
    }
}

pub(crate) fn search_count(count: usize) -> String {
    if count == 1 {
        s::settings::search_count_one()
    } else {
        rust_i18n::t!("settings.search_count", count = count).into_owned()
    }
}

/// Subtitle on a plugin row. The singular branch lives here rather than at the
/// call site because which counts need their own wording is a locale fact.
pub(crate) fn plugin_skill_count(count: usize) -> String {
    if count == 1 {
        s::settings::plugin_skill_count_one()
    } else {
        rust_i18n::t!("settings.plugin_skill_count", count => count).into_owned()
    }
}
