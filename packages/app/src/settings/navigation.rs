//! Stable section identities with settings-local navigation and page copy.

use crate::surface::strings as s;
use crate::ui::icons;
use daruda_config::BuiltinSection as Section;

type NavigationGroup = (&'static [Section], fn() -> String);

pub(super) const GROUPS: &[NavigationGroup] = &[
    (
        &[Section::General, Section::Appearance, Section::Font],
        s::settings::group_application,
    ),
    (&[Section::Terminal], s::settings::group_terminal),
    (
        &[Section::Workspace, Section::Keymap],
        s::settings::group_workspace,
    ),
    (
        &[
            Section::Agent,
            Section::Orchestrator,
            Section::SessionHosts,
            Section::Accounts,
        ],
        s::settings::group_agents,
    ),
    (
        &[
            Section::Notifications,
            Section::RemoteControl,
            Section::Plugin,
            Section::About,
        ],
        s::settings::group_system,
    ),
];

pub(super) fn label(section: Section) -> String {
    match section {
        Section::General => s::settings::nav_general(),
        Section::Appearance => s::settings::nav_appearance(),
        Section::Font => s::settings::nav_font(),
        Section::Terminal => s::settings::nav_terminal(),
        Section::Workspace => s::settings::nav_workspace(),
        Section::Keymap => s::settings::nav_keymap(),
        Section::Agent => s::settings::nav_agent(),
        Section::Orchestrator => s::settings::nav_orchestrator(),
        Section::SessionHosts => s::settings::nav_session_hosts(),
        Section::Accounts => s::settings::nav_accounts(),
        Section::Notifications => s::settings::nav_notifications(),
        Section::RemoteControl => s::settings::nav_remote_control(),
        Section::Plugin => s::settings::nav_plugin(),
        Section::About => s::settings::nav_about(),
    }
}

pub(super) fn description(section: Section) -> String {
    match section {
        Section::General => s::settings::desc_general(),
        Section::Appearance => s::settings::desc_appearance(),
        Section::Font => s::settings::desc_font(),
        Section::Terminal => s::settings::desc_terminal(),
        Section::Workspace => s::settings::desc_workspace(),
        Section::Keymap => s::settings::desc_keymap(),
        Section::Agent => s::settings::desc_agent(),
        Section::Orchestrator => s::settings::desc_orchestrator(),
        Section::SessionHosts => s::settings::desc_session_hosts(),
        Section::Accounts => s::settings::desc_accounts(),
        Section::Notifications => s::settings::desc_notifications(),
        Section::RemoteControl => s::settings::desc_remote_control(),
        Section::Plugin => s::settings::desc_plugin(),
        Section::About => s::settings::desc_about(),
    }
}

pub(super) fn icon(section: Section) -> &'static str {
    match section {
        Section::General => icons::SETTINGS,
        Section::Appearance => icons::APPEARANCE,
        Section::Font => icons::TEXT_FIELDS,
        Section::Terminal => icons::TERMINAL,
        Section::Workspace => icons::DOCK,
        Section::Keymap => icons::KEYBOARD,
        Section::Agent => icons::AGENT,
        Section::Orchestrator => icons::ORCHESTRATOR,
        Section::SessionHosts => icons::DNS,
        Section::Accounts => icons::PERSON,
        Section::Notifications => icons::NOTIFICATIONS,
        Section::RemoteControl => icons::REMOTE_CONTROL,
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
