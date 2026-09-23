//! Stable section identities with settings-local navigation and page copy.

use crate::surface::strings as s;
use crate::ui::icons;
use daruda_config::BuiltinSection as Section;

type NavigationGroup = (&'static [Section], fn() -> String);

pub(super) const GROUPS: &[NavigationGroup] = &[
    (
        &[Section::General, Section::Appearance, Section::Font],
        s::settings_group_application,
    ),
    (&[Section::Terminal], s::settings_group_terminal),
    (
        &[Section::Workspace, Section::Keymap],
        s::settings_group_workspace,
    ),
    (
        &[
            Section::Agent,
            Section::Orchestrator,
            Section::SessionHosts,
            Section::Accounts,
        ],
        s::settings_group_agents,
    ),
    (
        &[
            Section::Notifications,
            Section::RemoteControl,
            Section::Plugin,
            Section::About,
        ],
        s::settings_group_system,
    ),
];

pub(super) fn label(section: Section) -> String {
    match section {
        Section::General => s::settings_nav_general(),
        Section::Appearance => s::settings_nav_appearance(),
        Section::Font => s::settings_nav_font(),
        Section::Terminal => s::settings_nav_terminal(),
        Section::Workspace => s::settings_nav_workspace(),
        Section::Keymap => s::settings_nav_keymap(),
        Section::Agent => s::settings_nav_agent(),
        Section::Orchestrator => s::settings_nav_orchestrator(),
        Section::SessionHosts => s::settings_nav_session_hosts(),
        Section::Accounts => s::settings_nav_accounts(),
        Section::Notifications => s::settings_nav_notifications(),
        Section::RemoteControl => s::settings_nav_remote_control(),
        Section::Plugin => s::settings_nav_plugin(),
        Section::About => s::settings_nav_about(),
    }
}

pub(super) fn description(section: Section) -> String {
    match section {
        Section::General => s::settings_desc_general(),
        Section::Appearance => s::settings_desc_appearance(),
        Section::Font => s::settings_desc_font(),
        Section::Terminal => s::settings_desc_terminal(),
        Section::Workspace => s::settings_desc_workspace(),
        Section::Keymap => s::settings_desc_keymap(),
        Section::Agent => s::settings_desc_agent(),
        Section::Orchestrator => s::settings_desc_orchestrator(),
        Section::SessionHosts => s::settings_desc_session_hosts(),
        Section::Accounts => s::settings_desc_accounts(),
        Section::Notifications => s::settings_desc_notifications(),
        Section::RemoteControl => s::settings_desc_remote_control(),
        Section::Plugin => s::settings_desc_plugin(),
        Section::About => s::settings_desc_about(),
    }
}

pub(super) fn icon(section: Section) -> &'static str {
    match section {
        Section::General => icons::SETTINGS,
        Section::Appearance => icons::MAXIMIZE,
        Section::Font => icons::TEXT_FIELDS,
        Section::Terminal => icons::TERMINAL,
        Section::Workspace => icons::DOCK,
        Section::Keymap => icons::KEYBOARD,
        Section::Agent | Section::Orchestrator => icons::AGENT,
        Section::SessionHosts => icons::DNS,
        Section::Accounts => icons::PERSON,
        Section::Notifications => icons::NOTIFICATIONS,
        Section::RemoteControl => icons::FORWARD,
        Section::Plugin => icons::EXTENSION,
        Section::About => icons::INFO,
    }
}

pub(super) fn is_catalog(section: Section) -> bool {
    matches!(
        section,
        Section::Agent | Section::SessionHosts | Section::Accounts | Section::Plugin
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource as _;

    #[test]
    fn every_stable_page_appears_once_and_has_an_embedded_icon() {
        let sections = GROUPS
            .iter()
            .flat_map(|(items, _)| items.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(sections.len(), Section::ALL.len());
        for section in Section::ALL {
            assert_eq!(sections.iter().filter(|item| *item == section).count(), 1);
            assert!(
                crate::assets::DarudaAssets
                    .load(icon(*section))
                    .unwrap()
                    .is_some()
            );
            assert!(!label(*section).is_empty());
            assert!(!description(*section).is_empty());
        }
    }
}
