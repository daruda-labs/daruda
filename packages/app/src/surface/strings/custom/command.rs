use crate::surface::strings as s;
use daruda_config::BuiltinSection as Section;
use daruda_settings::section_label;

pub(crate) fn settings_general() -> String {
    s::command::settings_section(section_label(Section::General))
}

pub(crate) fn settings_appearance() -> String {
    s::command::settings_section(section_label(Section::Appearance))
}

pub(crate) fn settings_font() -> String {
    s::command::settings_section(section_label(Section::Font))
}

pub(crate) fn settings_terminal() -> String {
    s::command::settings_section(section_label(Section::Terminal))
}

pub(crate) fn settings_workspace() -> String {
    s::command::settings_section(section_label(Section::Workspace))
}

pub(crate) fn settings_keymap() -> String {
    s::command::settings_section(section_label(Section::Keymap))
}

pub(crate) fn settings_agent() -> String {
    s::command::settings_section(section_label(Section::Agent))
}

pub(crate) fn settings_orchestrator() -> String {
    s::command::settings_section(section_label(Section::Orchestrator))
}

pub(crate) fn settings_session_hosts() -> String {
    s::command::settings_section(section_label(Section::SessionHosts))
}

pub(crate) fn settings_accounts() -> String {
    s::command::settings_section(section_label(Section::Accounts))
}

pub(crate) fn settings_notifications() -> String {
    s::command::settings_section(section_label(Section::Notifications))
}

pub(crate) fn settings_remote_control() -> String {
    s::command::settings_section(section_label(Section::RemoteControl))
}

pub(crate) fn settings_plugin() -> String {
    s::command::settings_section(section_label(Section::Plugin))
}

pub(crate) fn settings_about() -> String {
    s::command::settings_section(section_label(Section::About))
}
