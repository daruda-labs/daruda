//! Rewrites the one key (or structural catalog) a Settings field patch addresses.

use super::catalog_tables::{replace_agents, replace_session_hosts, status_bar_item_slug};
use super::sections::{font_key, patch_nested_section, patch_section};
use crate::settings_patch::fields::field_value;
use crate::{Config, CursorStyle, SettingsPatch, remote};

macro_rules! impl_document_patch {
    ($($variant:ident: $($member:ident).+, $path:literal, $ownership:ident, $serialize:expr;)*) => {
        pub(super) fn patch_settings_document(
            doc: &mut toml_edit::DocumentMut,
            config: &Config,
            patch: &SettingsPatch,
        ) {
            match patch {
                $(SettingsPatch::$variant(_) => {
                    let (section, key) = $path.rsplit_once('.').expect("field has a section");
                    let value = ($serialize)(field_value!($ownership, &config.$($member).+));
                    // One key each: rewriting the whole `[font]` tree would pin every
                    // other font field, including one just reset, to its current value.
                    if let Some(domain) = section.strip_prefix("font.") {
                        font_key(doc, domain, key, value);
                    } else if section.contains('.') {
                        patch_nested_section(doc, &section.split('.').collect::<Vec<_>>(), |t| {
                            t.insert(key, toml_edit::value(value));
                        });
                    } else {
                        patch_section(doc, section, |t| {
                            t.insert(key, toml_edit::value(value));
                        });
                    }
                },)*
                SettingsPatch::RemoteChannels(_) => remote::patch_document(doc, &config.remote),
                SettingsPatch::CursorStyle(_) => patch_section(doc, "cursor", |t| {
                    let value = match config.cursor.style {
                        CursorStyle::Block => "block",
                        CursorStyle::Underline => "underline",
                        CursorStyle::Bar => "bar",
                    };
                    t.insert("style", toml_edit::value(value));
                }),
                SettingsPatch::AgentCatalog(_) => replace_agents(doc, &config.agents),
                SettingsPatch::SessionHosts { .. } => {
                    replace_session_hosts(doc, &config.session_hosts, &config.session_host_tombstones)
                },
                SettingsPatch::KeepAwake(_) => patch_section(doc, "desktop", |t| {
                    t.insert(
                        "keep_awake",
                        toml_edit::value(config.desktop.keep_awake.as_str()),
                    );
                }),
                SettingsPatch::ShellProgram(_) => patch_section(doc, "shell", |t| {
                    match &config.shell.program {
                        Some(program) => t.insert("program", toml_edit::value(program.clone())),
                        None => t.remove("program"),
                    };
                }),
                SettingsPatch::ToggleStatusBarItem(_) | SettingsPatch::StatusBarHiddenItems(_) => {
                    patch_section(doc, "status_bar", |t| {
                        let mut items = toml_edit::Array::new();
                        for item in &config.status_bar.hidden_items {
                            items.push(status_bar_item_slug(*item));
                        }
                        t.insert("hidden_items", toml_edit::value(items));
                        t.remove("visible_items");
                    })
                },
                SettingsPatch::LeftDockSize(_) => patch_section(doc, "left_dock", |t| {
                    t.insert("size", toml_edit::value(config.left_dock.size.as_str()));
                }),
                SettingsPatch::FileIconColorMode(_) => patch_section(doc, "left_dock", |t| {
                    let mode = match config.left_dock.file_icon_color_mode {
                        crate::IconColorMode::Color => "color",
                        crate::IconColorMode::Monochrome => "monochrome",
                    };
                    t.insert("file_icon_color_mode", toml_edit::value(mode));
                }),
                SettingsPatch::TelegramAuthorizedChatId(_) => patch_section(doc, "telegram", |t| {
                    if let Some(id) = config.telegram.authorized_chat_id {
                        t.insert("authorized_chat_id", toml_edit::value(id));
                    } else {
                        t.remove("authorized_chat_id");
                    }
                }),
                // Each `None` removes its key rather than writing a null: absent is
                // what the deserializer reads as "follow the default", and a written
                // empty string would be a real agent id that resolves to nothing.
                SettingsPatch::OrchestratorAgentId(_) => patch_section(doc, "orchestrator", |t| {
                    match &config.orchestrator.agent_id {
                        Some(id) => t.insert("agent_id", toml_edit::value(id.clone())),
                        None => t.remove("agent_id"),
                    };
                }),
                SettingsPatch::OrchestratorAccountId(_) => patch_section(doc, "orchestrator", |t| {
                    match config.orchestrator.account_id {
                        Some(id) => t.insert("account_id", toml_edit::value(id.0.to_string())),
                        None => t.remove("account_id"),
                    };
                }),
            }
        }
    };
}

crate::settings_patch::fields::with_settings_fields!(impl_document_patch);

#[cfg(test)]
mod tests;
