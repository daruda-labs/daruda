use crate::surface::strings as s;

pub(crate) fn settings_general() -> String {
    s::command::settings_section(s::settings::nav_general())
}

pub(crate) fn settings_appearance() -> String {
    s::command::settings_section(s::settings::nav_appearance())
}

pub(crate) fn settings_font() -> String {
    s::command::settings_section(s::settings::nav_font())
}

pub(crate) fn settings_terminal() -> String {
    s::command::settings_section(s::settings::nav_terminal())
}

pub(crate) fn settings_workspace() -> String {
    s::command::settings_section(s::settings::nav_workspace())
}

pub(crate) fn settings_keymap() -> String {
    s::command::settings_section(s::settings::nav_keymap())
}

pub(crate) fn settings_agent() -> String {
    s::command::settings_section(s::settings::nav_agent())
}

pub(crate) fn settings_orchestrator() -> String {
    s::command::settings_section(s::settings::nav_orchestrator())
}

pub(crate) fn settings_session_hosts() -> String {
    s::command::settings_section(s::settings::nav_session_hosts())
}

pub(crate) fn settings_accounts() -> String {
    s::command::settings_section(s::settings::nav_accounts())
}

pub(crate) fn settings_notifications() -> String {
    s::command::settings_section(s::settings::nav_notifications())
}

pub(crate) fn settings_remote_control() -> String {
    s::command::settings_section(s::settings::nav_remote_control())
}

pub(crate) fn settings_plugin() -> String {
    s::command::settings_section(s::settings::nav_plugin())
}

pub(crate) fn settings_about() -> String {
    s::command::settings_section(s::settings::nav_about())
}
