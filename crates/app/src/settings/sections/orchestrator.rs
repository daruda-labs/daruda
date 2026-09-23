//! The orchestrator's Settings controls: enable, which agent, which account.
//!
//! The two pickers persist an `Option`, and an empty select value is what
//! carries the `None`. Both directions live here as pure functions so the
//! encoding is stated once rather than re-derived by the widget and the
//! reload path separately.

use daruda_config::Config;
use daruda_store::accounts::{AccountId, ManagedAccount};
use gpui::{AnyElement, IntoElement, SharedString, prelude::*};

use crate::settings::presentation::{card, row};
use crate::settings::{BoolSetting, SettingsView};
use crate::surface::strings as s;
use crate::ui::select;

/// The select value standing for "no explicit choice". Empty rather than a
/// sentinel word, because a select's value is also its option key and any
/// word would collide with a real agent id.
const UNSET: &str = "";

/// Agent options: the catalog, headed by the follow-the-catalog choice.
pub(in crate::settings) fn agent_options(config: &Config) -> Vec<select::SelectOption> {
    let mut opts = vec![select::SelectOption::new(
        UNSET,
        s::settings_orchestrator_agent_default(),
    )];
    opts.extend(
        config
            .resolved_agents()
            .into_iter()
            .map(|a| select::SelectOption::new(a.id, a.name)),
    );
    opts
}

/// Account options: the managed accounts, headed by the system default.
pub(in crate::settings) fn account_options(
    accounts: &[ManagedAccount],
) -> Vec<select::SelectOption> {
    let mut opts = vec![select::SelectOption::new(
        UNSET,
        s::settings_orchestrator_account_system(),
    )];
    opts.extend(
        accounts
            .iter()
            .map(|a| select::SelectOption::new(a.id.0.to_string(), account_label(a))),
    );
    opts
}

/// How one managed account reads in the picker. Prefixed by its auth domain
/// because two domains can hold the same address.
fn account_label(account: &ManagedAccount) -> String {
    let identity = account
        .email
        .clone()
        .unwrap_or_else(s::settings_accounts_unknown_email);
    s::settings_orchestrator_account_option(&s::account_recipe_label(account.recipe), &identity)
}

/// The select value the live config implies.
pub(in crate::settings) fn agent_select_value(config: &Config) -> SharedString {
    config
        .orchestrator
        .agent_id
        .clone()
        .map_or_else(|| SharedString::new_static(UNSET), SharedString::from)
}

/// Same, for the account picker.
pub(in crate::settings) fn account_select_value(config: &Config) -> SharedString {
    config.orchestrator.account_id.map_or_else(
        || SharedString::new_static(UNSET),
        |id| SharedString::from(id.0.to_string()),
    )
}

/// Inverse of [`agent_select_value`]: an empty pick is "follow the catalog".
pub(in crate::settings) fn agent_id_from_select(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// Inverse of [`account_select_value`]. An unparseable value cannot be one of
/// the options this module built, so it reads as the system default rather
/// than pinning the session to an account that does not exist.
pub(in crate::settings) fn account_id_from_select(value: &str) -> Option<AccountId> {
    uuid::Uuid::parse_str(value).ok().map(AccountId)
}

impl SettingsView {
    /// The Orchestrator page: enable, which agent, which account.
    pub(in crate::settings) fn render_orchestrator(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        card(s::settings_nav_orchestrator(), cx)
            .child(self.switch_row(
                BoolSetting::OrchestratorEnabled,
                s::settings_orchestrator_enabled_label(),
                String::new(),
                cx,
            ))
            .child(row(
                s::settings_orchestrator_agent_label(),
                String::new(),
                select::select(&self.orchestrator_agent_select, cx, 0),
                cx,
            ))
            .child(row(
                s::settings_orchestrator_account_label(),
                String::new(),
                select::select(&self.orchestrator_account_select, cx, 0),
                cx,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_config::OrchestratorConfig;

    fn config_with(agent: Option<&str>, account: Option<AccountId>) -> Config {
        Config {
            orchestrator: OrchestratorConfig {
                enabled: true,
                agent_id: agent.map(str::to_owned),
                account_id: account,
            },
            ..Config::default()
        }
    }

    #[test]
    fn an_unset_agent_round_trips_through_the_empty_value() {
        let value = agent_select_value(&config_with(None, None));
        assert_eq!(value, UNSET);
        assert_eq!(agent_id_from_select(value.to_string()), None);
    }

    #[test]
    fn a_named_agent_round_trips() {
        let value = agent_select_value(&config_with(Some("codex-acp"), None));
        assert_eq!(
            agent_id_from_select(value.to_string()),
            Some("codex-acp".to_owned())
        );
    }

    #[test]
    fn an_account_round_trips_and_the_system_default_stays_none() {
        let id = AccountId::new();
        let value = account_select_value(&config_with(None, Some(id)));
        assert_eq!(account_id_from_select(&value), Some(id));
        assert_eq!(account_select_value(&config_with(None, None)), UNSET);
        assert_eq!(account_id_from_select(UNSET), None);
    }

    #[test]
    fn a_value_that_is_not_an_account_id_reads_as_the_system_default() {
        assert_eq!(account_id_from_select("not-a-uuid"), None);
    }

    /// The catalog is never empty (`Config::default` seeds it), so the picker
    /// always offers the fallback choice plus at least one real agent.
    #[test]
    fn the_agent_picker_heads_the_catalog_with_the_fallback_choice() {
        let opts = agent_options(&Config::default());
        assert!(opts.len() >= 2);
        assert_eq!(opts[0].value, UNSET);
    }

    #[test]
    fn the_account_picker_offers_the_system_default_even_with_no_accounts() {
        let opts = account_options(&[]);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, UNSET);
    }
}
