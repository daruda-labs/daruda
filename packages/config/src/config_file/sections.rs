//! `toml_edit` section helpers: creating and patching tables, the per-domain
//! font schema, and moving keys an older release spelled differently.

use crate::{FontConfig, SettingsFieldId};

/// [`patch_section`] for a table nested under others (`[usage.poll]`),
/// creating each missing level. A dotted name passed to `patch_section`
/// would instead make one quoted table, `["usage.poll"]`.
pub(super) fn patch_nested_section(
    doc: &mut toml_edit::DocumentMut,
    path: &[&str],
    f: impl FnOnce(&mut dyn toml_edit::TableLike),
) {
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in path {
        if !table.contains_key(part) {
            let mut child = toml_edit::Table::new();
            child.set_implicit(true);
            table.insert(part, toml_edit::Item::Table(child));
        }
        match table
            .get_mut(part)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            Some(next) => table = next,
            None => return,
        }
    }
    f(table);
}

/// Patch one section at a time. Creates the table if absent so saving from a
/// fresh install produces a minimal document.
pub(crate) fn patch_section(
    doc: &mut toml_edit::DocumentMut,
    key: &str,
    f: impl FnOnce(&mut dyn toml_edit::TableLike),
) {
    if !doc.contains_key(key) {
        doc.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    if let Some(table) = doc
        .get_mut(key)
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        f(table);
    }
}

/// Write the domain-based font schema and remove the legacy flat keys. Font
/// changes rewrite this small section as a unit so partially migrated files
/// cannot end up with two competing values for the same surface.
pub(super) fn patch_font_config(doc: &mut toml_edit::DocumentMut, font: &FontConfig) {
    patch_section(doc, "font", |table| {
        for key in [
            "family",
            "size",
            "editor_size",
            "agent_chat_size",
            "vertical_spacing",
            "horizontal_spacing",
            "inset_x",
            "inset_y",
        ] {
            table.remove(key);
        }
    });

    patch_font_domain(doc, "terminal", |table| {
        table.insert("family", toml_edit::value(font.terminal.family.clone()));
        table.insert("size", toml_edit::value(f64::from(font.terminal.size)));
        table.insert(
            "line_height",
            toml_edit::value(f64::from(font.terminal.line_height)),
        );
        table.insert(
            "cell_width",
            toml_edit::value(f64::from(font.terminal.cell_width)),
        );
        table.insert(
            "inset_x",
            toml_edit::value(f64::from(font.terminal.inset_x)),
        );
        table.insert(
            "inset_y",
            toml_edit::value(f64::from(font.terminal.inset_y)),
        );
    });
    patch_font_domain(doc, "editor", |table| {
        table.insert("family", toml_edit::value(font.editor.family.clone()));
        table.insert("size", toml_edit::value(f64::from(font.editor.size)));
        table.insert(
            "line_height",
            toml_edit::value(f64::from(font.editor.line_height)),
        );
    });
    patch_font_domain(doc, "agent_chat", |table| {
        table.insert("family", toml_edit::value(font.agent_chat.family.clone()));
        table.insert("size", toml_edit::value(f64::from(font.agent_chat.size)));
        table.insert(
            "line_height",
            toml_edit::value(f64::from(font.agent_chat.line_height)),
        );
    });
}

pub(super) fn font_key(
    doc: &mut toml_edit::DocumentMut,
    domain: &str,
    key: &str,
    value: impl Into<toml_edit::Value>,
) {
    let value = value.into();
    patch_font_domain(doc, domain, |t| {
        t.insert(key, toml_edit::Item::Value(value));
    });
}

/// Flat `[font]` keys an older release wrote, and where each is read now.
/// `family` fed both the terminal and the editor.
const FLAT_FONT_KEYS: &[(&str, &[(&str, &str)])] = &[
    ("family", &[("terminal", "family"), ("editor", "family")]),
    ("size", &[("terminal", "size")]),
    ("editor_size", &[("editor", "size")]),
    ("agent_chat_size", &[("agent_chat", "size")]),
    ("vertical_spacing", &[("terminal", "line_height")]),
    ("horizontal_spacing", &[("terminal", "cell_width")]),
    ("inset_x", &[("terminal", "inset_x")]),
    ("inset_y", &[("terminal", "inset_y")]),
];

/// Carry the keys an older release spelled differently to where `field` is
/// read now, so neither a write nor a reset of `field` is undone by the old
/// spelling. A key already at its new place wins, as it does on load.
pub(super) fn migrate_legacy_keys(doc: &mut toml_edit::DocumentMut, field: SettingsFieldId) {
    use SettingsFieldId as F;
    match field {
        F::TerminalPreset => move_key(doc, "theme", "preset", &["theme"], "terminal_preset"),
        F::TelegramOnlyWhenAway => move_key(
            doc,
            "telegram",
            "defer_while_active",
            &["telegram"],
            "only_when_away",
        ),
        F::PresenceIdleSecs => move_key(
            doc,
            "telegram",
            "active_idle_secs",
            &["presence"],
            "away_idle_secs",
        ),
        F::PresenceGraceSecs => move_key(
            doc,
            "telegram",
            "away_grace_secs",
            &["presence"],
            "away_grace_secs",
        ),
        F::KeepAwake | F::KeepAwakeApprovalWaitMinutes => migrate_legacy_keep_awake(doc),
        F::TerminalFontFamily
        | F::TerminalFontSize
        | F::TerminalLineHeight
        | F::TerminalCellWidth
        | F::TerminalInsetX
        | F::TerminalInsetY
        | F::EditorFontFamily
        | F::EditorFontSize
        | F::EditorLineHeight
        | F::AgentChatFontFamily
        | F::AgentChatFontSize
        | F::AgentChatLineHeight => migrate_flat_font_keys(doc),
        _ => {}
    }
}

/// Move `[from] key` to `to.new_key` unless the new key is already set.
fn move_key(doc: &mut toml_edit::DocumentMut, from: &str, key: &str, to: &[&str], new_key: &str) {
    let Some(item) = doc
        .get_mut(from)
        .and_then(toml_edit::Item::as_table_like_mut)
        .and_then(|t| t.remove(key))
    else {
        return;
    };
    patch_nested_section(doc, to, |t| {
        if !t.contains_key(new_key) {
            t.insert(new_key, item);
        }
    });
}

/// Fold the old `keep_awake_while_working` flag into `keep_awake`. As on
/// load, an explicit `keep_awake` wins and `false` is the default.
fn migrate_legacy_keep_awake(doc: &mut toml_edit::DocumentMut) {
    let Some(desktop) = doc
        .get_mut("desktop")
        .and_then(toml_edit::Item::as_table_like_mut)
    else {
        return;
    };
    let Some(legacy) = desktop.remove("keep_awake_while_working") else {
        return;
    };
    if legacy.as_bool() == Some(true) && !desktop.contains_key("keep_awake") {
        desktop.insert("keep_awake", toml_edit::value("agent"));
    }
}

fn migrate_flat_font_keys(doc: &mut toml_edit::DocumentMut) {
    for (flat, targets) in FLAT_FONT_KEYS {
        let Some(item) = doc
            .get_mut("font")
            .and_then(toml_edit::Item::as_table_like_mut)
            .and_then(|t| t.remove(flat))
        else {
            continue;
        };
        for (domain, key) in *targets {
            let item = item.clone();
            patch_font_domain(doc, domain, |t| {
                if !t.contains_key(key) {
                    t.insert(key, item);
                }
            });
        }
    }
}

fn patch_font_domain(
    doc: &mut toml_edit::DocumentMut,
    domain: &str,
    patch: impl FnOnce(&mut dyn toml_edit::TableLike),
) {
    let parent_is_inline = doc
        .get("font")
        .and_then(toml_edit::Item::as_inline_table)
        .is_some();
    patch_section(doc, "font", |font| {
        if !font.contains_key(domain) {
            let item = if parent_is_inline {
                toml_edit::Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()))
            } else {
                toml_edit::Item::Table(toml_edit::Table::new())
            };
            font.insert(domain, item);
        }
        if let Some(table) = font
            .get_mut(domain)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            patch(table);
        }
    });
}
