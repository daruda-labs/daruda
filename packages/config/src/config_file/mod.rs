//! The config file on disk: its path, reading, the atomic write, and the two
//! save paths (whole-config patch and one Settings field at a time).

mod catalog_tables;
mod field_patch;
mod sections;

use std::{io::Write as _, path::PathBuf};

use crate::{
    Config, CursorStyle, SettingsFieldId, SettingsPatch, StatusBarItem, agent, remote, telegram,
};
use catalog_tables::{
    remove_legacy_agent_keys, remove_legacy_agent_keys_from, replace_agents, replace_session_hosts,
};
use field_patch::patch_settings_document;
pub(crate) use sections::patch_section;
use sections::{migrate_legacy_keys, patch_font_config};

/// Resolve the config file path — the compatibility data directory used by
/// workspaces and other persisted files
/// (`daruda_store::persistence::default_data_dir`): the release build
/// keeps the un-suffixed `daruda/config.toml`, while a debug build or any
/// `DARUDA_PROFILE`-named run (tests, staging, etc.) gets its own
/// `daruda-<profile>/config.toml`, isolated from the release file.
pub fn config_path() -> PathBuf {
    daruda_store::persistence::default_data_dir().join("config.toml")
}

/// Surgically update the Settings-UI-controlled keys in the config file,
/// leaving all other keys (e.g. `[colors]`, `[keybindings]`), comments, and
/// formatting intact.  Reads the existing file first; if it does not exist,
/// writes a fresh file containing only the patched sections.
///
/// Returns an error string if the existing file cannot be parsed, or if the
/// file cannot be written.
pub fn patch_config_file(config: &Config) -> Result<(), String> {
    patch_config_file_to(config, &config_path())
}

/// Apply one Settings UI change against the latest on-disk document.
///
/// Only the addressed TOML key (or structural catalog) is rewritten, so an
/// open Settings window cannot overwrite unrelated changes made after it was
/// opened. The returned config is parsed from the exact document written and
/// is suitable for replacing an in-memory settings cache.
pub fn apply_settings_patch(patch: &SettingsPatch) -> Result<Config, String> {
    apply_settings_patch_to(patch, &config_path())
}

/// Path-aware form of [`apply_settings_patch`] for tests and alternate stores.
pub fn apply_settings_patch_to(
    patch: &SettingsPatch,
    path: &std::path::Path,
) -> Result<Config, String> {
    apply_settings_patch_to_inner(patch, path, None, WriteMode::Set)
        .map_err(|error| error.to_string())
}

/// A Settings patch failed because the addressed field changed or persistence
/// itself failed. Callers use the conflict variant to offer an explicit choice
/// instead of treating a concurrent edit as an I/O error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsPatchApplyError {
    Conflict(SettingsFieldId),
    Persistence(String),
}

impl std::fmt::Display for SettingsPatchApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(field) => write!(f, "{} changed before it could be saved", field.path()),
            Self::Persistence(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SettingsPatchApplyError {}

/// Apply `patch` only when its field still matches `expected` in the latest
/// on-disk document. Unrelated changes are retained and retried if the file
/// changes again while the replacement is being prepared.
pub fn apply_settings_patch_to_if_unchanged(
    patch: &SettingsPatch,
    expected: &Config,
    path: &std::path::Path,
) -> Result<Config, SettingsPatchApplyError> {
    apply_settings_patch_to_inner(patch, path, Some(expected), WriteMode::Set)
}

/// Return one field to its default by deleting its key, so the file tracks a
/// future change to the default instead of pinning today's value. `default`
/// is that field's patch built from `Config::default()`; it names the field
/// and gives the in-memory value. Refused like a write when the field moved
/// on disk since `expected` was read.
pub fn reset_settings_field_to_if_unchanged(
    default: &SettingsPatch,
    expected: &Config,
    path: &std::path::Path,
) -> Result<Config, SettingsPatchApplyError> {
    apply_settings_patch_to_inner(default, path, Some(expected), WriteMode::Remove)
}

/// Whether a write sets the field's key or removes it.
#[derive(Clone, Copy)]
enum WriteMode {
    Set,
    Remove,
}

/// Remove the key a dotted `path` names, leaving its tables in place.
fn remove_key_path(doc: &mut toml_edit::DocumentMut, path: &str) {
    let mut parts: Vec<&str> = path.split('.').collect();
    let Some(key) = parts.pop() else { return };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parts {
        match table
            .get_mut(part)
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            Some(next) => table = next,
            None => return,
        }
    }
    table.remove(key);
}

fn apply_settings_patch_to_inner(
    patch: &SettingsPatch,
    path: &std::path::Path,
    expected: Option<&Config>,
    mode: WriteMode,
) -> Result<Config, SettingsPatchApplyError> {
    const MAX_WRITE_ATTEMPTS: usize = 8;

    for _ in 0..MAX_WRITE_ATTEMPTS {
        let existing = read_config_text(path).map_err(SettingsPatchApplyError::Persistence)?;
        let mut doc: toml_edit::DocumentMut =
            existing.parse().map_err(|e: toml_edit::TomlError| {
                SettingsPatchApplyError::Persistence(format!(
                    "existing config has a parse error: {e}"
                ))
            })?;
        let mut config = parse_config_text(&existing)?;
        if expected.is_some_and(|baseline| patch.field_changed_between(baseline, &config)) {
            return Err(SettingsPatchApplyError::Conflict(patch.field()));
        }

        patch.apply_to(&mut config);
        config.clamp();
        migrate_legacy_keys(&mut doc, patch.field());
        match mode {
            WriteMode::Set => patch_settings_document(&mut doc, &config, patch),
            WriteMode::Remove => remove_key_path(&mut doc, patch.field().path()),
        }
        remove_legacy_agent_keys_from(&mut doc, &config);

        let reload = |doc: &toml_edit::DocumentMut| {
            let text = doc.to_string();
            let mut written = deserialize_config(&text).map_err(|e| {
                SettingsPatchApplyError::Persistence(format!(
                    "written config could not be reloaded: {e}"
                ))
            })?;
            written.clamp();
            Ok::<_, SettingsPatchApplyError>((text, written))
        };
        let (mut text, mut written) = reload(&doc)?;
        // A reset must land on the default. If a spelling this function does
        // not know still feeds the field, write the default instead.
        if matches!(mode, WriteMode::Remove) && patch.field_changed_between(&written, &config) {
            patch_settings_document(&mut doc, &config, patch);
            (text, written) = reload(&doc)?;
        }
        if write_config_text_atomic(path, &text, Some(&existing))
            .map_err(SettingsPatchApplyError::Persistence)?
        {
            return Ok(written);
        }
    }

    Err(SettingsPatchApplyError::Persistence(
        "config changed repeatedly while settings were being saved".to_string(),
    ))
}

/// Like [`patch_config_file`] but writes to an explicit path.  Used by tests
/// so they can operate on a temp directory instead of the user's real config.
pub fn patch_config_file_to(config: &Config, path: &std::path::Path) -> Result<(), String> {
    // Clamp before writing so disk never holds out-of-range values, even if
    // the caller forgot. Out-of-range values would otherwise survive on disk
    // until the next `Config::load` call clamped them in memory.
    let mut clamped = config.clone();
    clamped.clamp();
    let config = &clamped;

    let existing = read_config_text(path)?;
    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("existing config has a parse error: {e}"))?;

    patch_section(&mut doc, "general", |t| {
        t.insert(
            "language",
            toml_edit::value(config.general.language.clone()),
        );
    });

    patch_section(&mut doc, "theme", |t| {
        // Drop the legacy `preset` key on first write so `config.toml` ends
        // up with only `terminal_preset` (cell palette) / `ui_preset`
        // (chrome palette).
        t.remove("preset");
        t.insert(
            "terminal_preset",
            toml_edit::value(config.theme.terminal_preset.clone()),
        );
        t.insert(
            "ui_preset",
            toml_edit::value(config.theme.ui_preset.clone()),
        );
    });

    patch_font_config(&mut doc, &config.font);

    patch_section(&mut doc, "cursor", |t| {
        let style_str = match config.cursor.style {
            CursorStyle::Block => "block",
            CursorStyle::Underline => "underline",
            CursorStyle::Bar => "bar",
        };
        t.insert("style", toml_edit::value(style_str));
        t.insert("blinking", toml_edit::value(config.cursor.blinking));
    });

    patch_section(&mut doc, "shell", |t| {
        t.insert(
            "close_pane_on_exit",
            toml_edit::value(config.shell.close_pane_on_exit),
        );
    });

    patch_section(&mut doc, "window", |t| {
        t.insert(
            "opacity",
            toml_edit::value(f64::from(config.window.opacity)),
        );
        t.insert("blur", toml_edit::value(config.window.blur));
    });

    patch_section(&mut doc, "scrollback", |t| {
        t.insert(
            "max_rows",
            toml_edit::value(config.scrollback.max_rows as i64),
        );
    });

    patch_section(&mut doc, "left_dock", |t| {
        t.insert(
            "files_show_hidden",
            toml_edit::value(config.left_dock.files_show_hidden),
        );
        t.insert(
            "files_use_gitignore",
            toml_edit::value(config.left_dock.files_use_gitignore),
        );
    });

    patch_section(&mut doc, "file_viewer", |t| {
        t.insert(
            "syntax_theme",
            toml_edit::value(config.file_viewer.syntax_theme.clone()),
        );
        t.insert(
            "preview_tab",
            toml_edit::value(config.file_viewer.preview_tab),
        );
    });

    patch_section(&mut doc, "editor", |t| {
        t.insert(
            "preferred",
            toml_edit::value(config.editor.preferred.clone()),
        );
    });

    patch_section(&mut doc, "clipboard", |t| {
        t.insert(
            "streaming_max_bytes",
            toml_edit::value(config.clipboard.streaming_max_bytes as i64),
        );
    });

    patch_section(&mut doc, "panels", |t| {
        t.insert(
            "grid_columns",
            toml_edit::value(i64::from(config.panels.grid_columns)),
        );
    });

    patch_section(&mut doc, "render", |t| {
        t.insert(
            "max_fps",
            toml_edit::value(i64::from(config.render.max_fps)),
        );
    });

    patch_section(&mut doc, "status_bar", |t| {
        let mut arr = toml_edit::Array::new();
        for item in &config.status_bar.hidden_items {
            let slug = match item {
                StatusBarItem::ProjectBranch => "project_branch",
                StatusBarItem::AccountSlot => "account_slot",
                StatusBarItem::Ports => "ports",
                StatusBarItem::ClaudeUsage => "claude_usage",
                StatusBarItem::Flow => "flow",
            };
            arr.push(slug);
        }
        t.insert("hidden_items", toml_edit::value(arr));
        // The opt-in list is gone; leaving it behind would be read back on
        // the next launch and undo the migration that just ran.
        t.remove("visible_items");
    });

    patch_section(&mut doc, "ports", |t| {
        t.insert("poll_secs", toml_edit::value(config.ports.poll_secs as i64));
    });

    patch_section(&mut doc, "claude_status", |t| {
        t.insert("enable", toml_edit::value(config.claude_status.enable));
    });

    patch_section(&mut doc, "agent", |t| {
        t.insert(
            "use_modifier_to_send",
            toml_edit::value(config.agent.use_modifier_to_send),
        );
        t.insert(
            "input_max_rows",
            toml_edit::value(i64::from(config.agent.input_max_rows)),
        );
        t.insert(
            "use_reading_width",
            toml_edit::value(config.agent.use_reading_width),
        );
        t.insert(
            "tool_summary_labels",
            toml_edit::value(config.agent.tool_summary_labels),
        );
        remove_legacy_agent_keys(t);
    });

    patch_section(&mut doc, "presence", |t| {
        t.insert(
            "away_grace_secs",
            toml_edit::value(config.presence.away_grace_secs as i64),
        );
        t.insert(
            "away_idle_secs",
            toml_edit::value(config.presence.away_idle_secs as i64),
        );
        t.insert(
            "away_idle_foreground_secs",
            toml_edit::value(config.presence.away_idle_foreground_secs as i64),
        );
    });

    patch_section(&mut doc, "telegram", |t| {
        t.insert("enabled", toml_edit::value(config.telegram.enabled));
        t.insert(
            "only_when_away",
            toml_edit::value(config.telegram.only_when_away),
        );
        // Superseded by `only_when_away` + the `[presence]` section; leaving
        // them behind would let a stale value read as live configuration.
        for legacy in ["defer_while_active", "active_idle_secs", "away_grace_secs"] {
            t.remove(legacy);
        }
        match config.telegram.authorized_chat_id {
            Some(id) => {
                t.insert("authorized_chat_id", toml_edit::value(id));
            }
            None => {
                t.remove("authorized_chat_id");
            }
        }
    });

    if doc.contains_key("remote") || !config.remote.channels.is_empty() {
        remote::patch_document(&mut doc, &config.remote);
    }

    if doc.contains_key("agents") || config.agents != agent::default_agents() {
        replace_agents(&mut doc, &config.agents);
    }

    replace_session_hosts(
        &mut doc,
        &config.session_hosts,
        &config.session_host_tombstones,
    );

    write_config_text_atomic(path, &doc.to_string(), None).map(|_| ())
}

fn read_config_text(path: &std::path::Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("failed to read config: {error}")),
    }
}

pub(crate) fn deserialize_config(text: &str) -> Result<Config, toml::de::Error> {
    let mut document = toml::from_str::<toml::Table>(text)?;
    telegram::migrate_legacy_timing(&mut document);
    document.try_into()
}

fn parse_config_text(text: &str) -> Result<Config, SettingsPatchApplyError> {
    let mut config = if text.trim().is_empty() {
        Config::default()
    } else {
        deserialize_config(text).map_err(|e| {
            SettingsPatchApplyError::Persistence(format!(
                "existing config has invalid settings: {e}"
            ))
        })?
    };
    config.clamp();
    Ok(config)
}

fn config_write_path(path: &std::path::Path) -> Result<PathBuf, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => daruda_core::path::canonicalize(path)
            .map_err(|e| format!("failed to resolve config symlink: {e}")),
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(format!("failed to inspect config path: {error}")),
    }
}

/// Write a prepared document atomically. When `expected` is present, return
/// `Ok(false)` if the logical config path changed while the temporary file was
/// being prepared so the caller can rebuild its patch from the new contents.
fn write_config_text_atomic(
    path: &std::path::Path,
    text: &str,
    expected: Option<&str>,
) -> Result<bool, String> {
    let write_path = config_write_path(path)?;
    let parent = write_path
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("failed to create config dir: {e}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create temporary config: {e}"))?;
    temp.write_all(text.as_bytes())
        .map_err(|e| format!("failed to write temporary config: {e}"))?;
    temp.flush()
        .map_err(|e| format!("failed to flush temporary config: {e}"))?;
    if let Ok(metadata) = std::fs::metadata(&write_path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| format!("failed to preserve config permissions: {e}"))?;
    }
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("failed to sync temporary config: {e}"))?;
    if let Some(expected) = expected
        && read_config_text(path)? != expected
    {
        return Ok(false);
    }
    temp.persist(&write_path)
        .map_err(|e| format!("failed to replace config: {}", e.error))?;
    Ok(true)
}
