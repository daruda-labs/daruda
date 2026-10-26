//! Snapshot operations for ordinary fields and explicit structural boundaries.

use super::fields::field_value;
use super::{Config, SettingsFieldId, SettingsPatch};

macro_rules! impl_settings_fields {
    ($($variant:ident: $($member:ident).+, $path:literal, $ownership:ident, $serialize:expr;)*) => {
        impl SettingsFieldId {
            pub const fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => $path,)*
                    Self::CursorStyle => "cursor.style",
                    Self::AgentCatalog => "agents",
                    Self::SessionHosts => "session_hosts",
                    Self::KeepAwake => "desktop.keep_awake",
                    Self::LeftDockSize => "left_dock.size",
                    Self::FileIconColorMode => "left_dock.file_icon_color_mode",
                    Self::ShellProgram => "shell.program",
                    Self::StatusBarHiddenItems => "status_bar.hidden_items",
                    Self::TelegramAuthorizedChatId => "telegram.authorized_chat_id",
                    Self::RemoteChannels => "remote.channels",
                    Self::OrchestratorAgentId => "orchestrator.agent_id",
                    Self::OrchestratorAccountId => "orchestrator.account_id",
                }
            }
        }
        impl SettingsPatch {
            pub const fn field(&self) -> SettingsFieldId {
                match self {
                    $(Self::$variant(_) => SettingsFieldId::$variant,)*
                    Self::CursorStyle(_) => SettingsFieldId::CursorStyle,
                    Self::AgentCatalog(_) => SettingsFieldId::AgentCatalog,
                    Self::SessionHosts { .. } => SettingsFieldId::SessionHosts,
                    Self::KeepAwake(_) => SettingsFieldId::KeepAwake,
                    Self::LeftDockSize(_) => SettingsFieldId::LeftDockSize,
                    Self::FileIconColorMode(_) => SettingsFieldId::FileIconColorMode,
                    Self::StatusBarHiddenItems(_) => SettingsFieldId::StatusBarHiddenItems,
                    Self::ShellProgram(_) => SettingsFieldId::ShellProgram,
                    Self::ToggleStatusBarItem(_) => SettingsFieldId::StatusBarHiddenItems,
                    Self::TelegramAuthorizedChatId(_) => SettingsFieldId::TelegramAuthorizedChatId,
                    Self::RemoteChannels(_) => SettingsFieldId::RemoteChannels,
                    Self::OrchestratorAgentId(_) => SettingsFieldId::OrchestratorAgentId,
                    Self::OrchestratorAccountId(_) => SettingsFieldId::OrchestratorAccountId,
                }
            }

            /// Apply this one field to an in-memory snapshot. Persistence callers use
            /// this to advance only the addressed conflict baseline.
            pub fn apply_to(&self, config: &mut Config) {
                match self {
                    $(Self::$variant(value) => config.$($member).+ = field_value!($ownership, value),)*
                    Self::CursorStyle(value) => config.cursor.style = *value,
                    Self::AgentCatalog(value) => config.agents = value.clone(),
                    Self::SessionHosts {
                        entries,
                        tombstones,
                    } => {
                        config.session_hosts = entries.clone();
                        config.session_host_tombstones = tombstones.clone();
                    },
                    Self::KeepAwake(value) => config.desktop.keep_awake = *value,
                    Self::LeftDockSize(value) => config.left_dock.size = *value,
                    Self::FileIconColorMode(value) => config.left_dock.file_icon_color_mode = value.clone(),
                    Self::StatusBarHiddenItems(value) => config.status_bar.hidden_items = value.clone(),
                    Self::ShellProgram(value) => config.shell.program = value.clone(),
                    Self::ToggleStatusBarItem(value) => config.status_bar.toggle(*value),
                    Self::TelegramAuthorizedChatId(value) => config.telegram.authorized_chat_id = *value,
                    Self::RemoteChannels(value) => config.remote.channels = value.clone(),
                    Self::OrchestratorAgentId(value) => config.orchestrator.agent_id = value.clone(),
                    Self::OrchestratorAccountId(value) => config.orchestrator.account_id = *value,
                }
            }

            /// Whether the addressed value differs between two config snapshots.
            pub fn field_changed_between(&self, left: &Config, right: &Config) -> bool {
                match self {
                    $(Self::$variant(_) => left.$($member).+ != right.$($member).+,)*
                    Self::RemoteChannels(_) => left.remote != right.remote,
                    Self::CursorStyle(_) => left.cursor.style != right.cursor.style,
                    Self::AgentCatalog(_) => left.agents != right.agents,
                    Self::SessionHosts { .. } => {
                        left.session_hosts != right.session_hosts
                            || left.session_host_tombstones != right.session_host_tombstones
                    },
                    Self::KeepAwake(_) => left.desktop.keep_awake != right.desktop.keep_awake,
                    Self::LeftDockSize(_) => left.left_dock.size != right.left_dock.size,
                    Self::FileIconColorMode(_) => {
                        left.left_dock.file_icon_color_mode != right.left_dock.file_icon_color_mode
                    },
                    Self::StatusBarHiddenItems(_) => {
                        left.status_bar.hidden_items != right.status_bar.hidden_items
                    },
                    Self::ShellProgram(_) => left.shell.program != right.shell.program,
                    Self::ToggleStatusBarItem(_) => {
                        left.status_bar.hidden_items != right.status_bar.hidden_items
                    },
                    Self::TelegramAuthorizedChatId(_) => {
                        left.telegram.authorized_chat_id != right.telegram.authorized_chat_id
                    },
                    Self::OrchestratorAgentId(_) => {
                        left.orchestrator.agent_id != right.orchestrator.agent_id
                    },
                    Self::OrchestratorAccountId(_) => {
                        left.orchestrator.account_id != right.orchestrator.account_id
                    },
                }
            }
        }
    };
}

super::fields::with_settings_fields!(impl_settings_fields);

#[cfg(test)]
mod tests;
