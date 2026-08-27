//! TOML configuration loader for daruda.
//!
//! Config file location: `~/.config/daruda/config.toml` (XDG) or
//! `~/Library/Application Support/daruda/config.toml` (macOS fallback).
//! Missing file or parse errors fall back to defaults silently.

pub mod account_env;
pub mod agent;
pub mod claude_status;
pub mod clipboard;
pub mod colors;
pub mod cursor;
pub mod editor;
pub mod file_viewer;
pub mod flow;
pub mod font;
pub mod general;
pub mod keybindings;
pub mod left_dock;
pub mod logs;
pub mod notifications;
pub mod panels;
pub mod ports;
pub mod project;
pub mod render;
pub mod scrollback;
pub mod session_host;
mod settings_patch;
pub mod settings_section;
pub mod shell;
pub mod status_bar;
pub mod telegram;
pub mod theme_presets;
pub mod ui_theme_presets;
pub mod update;
pub mod usage;
pub mod window;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::{io::Write as _, path::PathBuf};

pub use account_env::{AccountEnv, account_env};
pub use agent::{
    ACP_REGISTRY_URL, ACP_REGISTRY_VERSION, AgentConfig, AgentDefinition, AgentEntry, AgentLaunch,
    AgentPreset, DefaultPermissionMode, PresetLaunchability, PresetOverrides,
    READING_WIDTH_DEFAULT, READING_WIDTH_MAX, READING_WIDTH_MIN, TAIL_WINDOW_ALL,
    TAIL_WINDOW_CHOICES, TAIL_WINDOW_DEFAULT, account_recipe_for_local_command, agent_preset,
    agent_presets,
};
pub use claude_status::ClaudeStatusConfig;
pub use clipboard::ClipboardConfig;
pub use colors::{AnsiPalette, ColorConfig, HexColor};
pub use cursor::{CursorConfig, CursorStyle};
pub use editor::{
    EditorConfig, ExternalEditorPreset, PRESETS as EXTERNAL_EDITOR_PRESETS,
    preset as external_editor_preset,
};
pub use file_viewer::FileViewerConfig;
pub use font::FontConfig;
pub use general::{GeneralConfig, SUPPORTED_LOCALES};
pub use keybindings::KeybindingConfig;
pub use left_dock::{IconColorMode, LeftDockConfig};
pub use logs::LogsConfig;
pub use notifications::NotificationsConfig;
pub use panels::PanelsConfig;
pub use ports::PortsConfig;
pub use project::{
    ProjectConfig, project_config_dir, project_config_dir_in, project_config_path,
    project_config_path_in, project_id,
};
pub use render::{ALLOWED_MAX_FPS, RenderConfig};
pub use scrollback::ScrollbackConfig;
pub use session_host::{SessionHostEntry, SessionHostKind, SessionHostTombstone};
pub use settings_patch::{SettingsFieldId, SettingsPatch};
pub use settings_section::{BuiltinSection, SettingsSection};
pub use shell::ShellConfig;
pub use status_bar::{StatusBarConfig, StatusBarItem};
pub use telegram::TelegramConfig;
pub use theme_presets::{PRESETS as THEME_PRESETS, ThemePreset};
pub use ui_theme_presets::{PRESETS as UI_THEME_PRESETS, UiThemePreset};
pub use update::UpdateConfig;
pub use usage::{PollConfig, UsageConfig};
pub use window::WindowConfig;

/// Which built-in presets to use for the two independent theme axes
/// — `terminal_preset` controls the cell palette (foreground /
/// background / ANSI 16), `ui_preset` controls workspace chrome
/// (tab bar, dock, modal, status bar, dock, agent panels).
///
/// `terminal_preset = "custom"` falls through to the `[colors]`
/// section. `terminal_preset = "default"` uses xterm-compatible
/// defaults.
///
/// The legacy `preset = "..."` key from pre-split configs is accepted
/// as an alias for `terminal_preset` so users who upgrade keep their
/// existing palette choice.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeConfig {
    #[serde(alias = "preset")]
    pub terminal_preset: String,
    pub ui_preset: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            terminal_preset: "default".to_owned(),
            ui_preset: ui_theme_presets::DEFAULT.to_owned(),
        }
    }
}

/// Top-level configuration. Every section uses `#[serde(default)]` so
/// a partial TOML file (or even an empty file) produces a valid config
/// with sensible defaults for all missing fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub font: FontConfig,
    pub cursor: CursorConfig,
    pub window: WindowConfig,
    pub colors: ColorConfig,
    pub theme: ThemeConfig,
    pub scrollback: ScrollbackConfig,
    pub keybindings: KeybindingConfig,
    pub shell: ShellConfig,
    pub left_dock: LeftDockConfig,
    pub file_viewer: FileViewerConfig,
    pub editor: EditorConfig,
    pub flow: flow::FlowConfig,
    pub claude_status: ClaudeStatusConfig,
    pub notifications: NotificationsConfig,
    pub clipboard: ClipboardConfig,
    pub usage: UsageConfig,
    pub panels: PanelsConfig,
    pub status_bar: StatusBarConfig,
    pub ports: PortsConfig,
    pub logs: LogsConfig,
    pub render: RenderConfig,
    pub agent: AgentConfig,
    /// Selectable ACP agent catalog **as persisted** — preset references and
    /// custom entries alike, including entries that resolve to nothing. Absent
    /// `[[agents]]` seeds a single Claude default (see [`agent::default_agents`]);
    /// an explicitly-empty catalog is normalized back to that default in
    /// [`Config::clamp`].
    ///
    /// Runtime consumers want [`Config::resolved_agents`] instead; this field is
    /// for the Settings editor, which has to show (and re-save) unresolved rows.
    #[serde(default = "agent::default_agents")]
    pub agents: Vec<AgentEntry>,
    /// Named, reusable SSH/Docker hosts a lane's `session_host` can
    /// reference by id instead of repeating the same target/container as
    /// free text. Unlike [`Self::agents`], an empty catalog is a completely
    /// valid state ("no hosts registered yet") — no non-empty seed.
    #[serde(default)]
    pub session_hosts: Vec<SessionHostEntry>,
    /// Removed [`SessionHostEntry`] rows, kept so a lane still referencing a
    /// deleted id can show what it used to point at. See
    /// [`SessionHostTombstone`].
    #[serde(default)]
    pub session_host_tombstones: Vec<SessionHostTombstone>,
    pub update: UpdateConfig,
    pub telegram: TelegramConfig,
}

impl Default for Config {
    fn default() -> Self {
        // Every field defaults normally EXCEPT `agents`, which seeds the
        // built-in Claude entry so the catalog is never empty. This is the
        // single source of the non-empty-catalog invariant the whole app relies
        // on (`catalog[0]` must exist) — both `Config::default()` and every
        // deserialize path (`#[serde(default = "agent::default_agents")]`) yield
        // a non-empty catalog. Fields are constructed directly, never by
        // deserializing a TOML string: the container `#[serde(default)]` calls
        // this `Config::default()` for missing-field fallbacks, so a
        // deserializing Default would recurse infinitely.
        Self {
            general: Default::default(),
            font: Default::default(),
            cursor: Default::default(),
            window: Default::default(),
            colors: Default::default(),
            theme: Default::default(),
            scrollback: Default::default(),
            keybindings: Default::default(),
            shell: Default::default(),
            left_dock: Default::default(),
            file_viewer: Default::default(),
            editor: Default::default(),
            flow: Default::default(),
            claude_status: Default::default(),
            notifications: Default::default(),
            clipboard: Default::default(),
            usage: Default::default(),
            panels: Default::default(),
            status_bar: Default::default(),
            ports: Default::default(),
            logs: Default::default(),
            render: Default::default(),
            agent: Default::default(),
            agents: agent::default_agents(),
            session_hosts: Vec::new(),
            session_host_tombstones: Vec::new(),
            update: Default::default(),
            telegram: Default::default(),
        }
    }
}

impl Config {
    /// Load configuration from the default path. Returns defaults on
    /// any error (missing file, parse failure, permission denied).
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    /// Load from an explicit path. Useful for testing.
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut cfg: Self = toml::from_str(&text).unwrap_or_default();
                cfg.clamp();
                cfg
            }
            Err(_) => {
                let mut cfg = Self::default();
                cfg.clamp();
                cfg
            }
        }
    }

    /// Clamp all numeric fields to their valid ranges.
    fn clamp(&mut self) {
        self.font.clamp();
        self.window.clamp();
        self.left_dock.clamp();
        self.claude_status.clamp();
        self.panels.clamp();
        self.render.clamp();
        self.agent.clamp();
        self.status_bar.clamp();
        // A missing `[[agents]]` is handled by the serde field default, and the
        // manual `Config::default()` (used on load errors) seeds the Claude
        // default directly (since a232e44). This guard exists for the remaining
        // case: an explicit `agents = []` in user TOML, which deserializes to the
        // provided empty array (not the field default) — normalize it back to the
        // Claude default so every load path hands the app a non-empty catalog.
        if self.agents.is_empty() {
            self.agents = agent::default_agents();
        }
    }

    /// The launchable agent catalog: every [`AgentEntry`] that resolves, in
    /// config order. An entry referencing a preset daruda no longer knows is
    /// skipped — it stays in [`Self::agents`] (so a save preserves it and the
    /// Settings editor can flag it) but never reaches the runtime, where a
    /// nameless, commandless agent would only be selectable-then-broken.
    ///
    /// Falls back to the built-in default when nothing resolves, upholding the
    /// same non-empty-catalog invariant [`Self::clamp`] enforces for an
    /// explicitly-empty array — every consumer relies on `catalog[0]` existing.
    pub fn resolved_agents(&self) -> Vec<AgentDefinition> {
        let resolved: Vec<AgentDefinition> =
            self.agents.iter().filter_map(AgentEntry::resolve).collect();
        if resolved.is_empty() {
            return agent::default_agents()
                .iter()
                .filter_map(AgentEntry::resolve)
                .collect();
        }
        resolved
    }

    /// Return the effective `ColorConfig` for this configuration.
    ///
    /// When `theme.terminal_preset` names a built-in preset (anything
    /// other than `"custom"`), the preset's `ColorConfig` takes
    /// precedence over the `[colors]` section. An unknown preset name
    /// falls back to `[colors]` so a typo doesn't silently produce
    /// wrong colours.
    pub fn effective_colors(&self) -> ColorConfig {
        if self.theme.terminal_preset != "custom"
            && let Some(colors) = theme_presets::colors_for_preset(&self.theme.terminal_preset)
        {
            return colors;
        }
        self.colors.clone()
    }

    /// Apply a project-layer override on top of `self` (the user
    /// layer) and return the resolved [`Config`]. Sections present in
    /// `project` replace the corresponding user section wholesale;
    /// sections absent from `project` keep the user value. Only
    /// `[shell]` is honoured today.
    pub fn resolve(mut self, project: &ProjectConfig) -> Self {
        if let Some(shell) = project.shell.clone() {
            self.shell = shell;
        }
        self
    }
}

/// Resolve the config file path — same profile-scoped data directory as
/// logs, workspaces, and every other persisted file
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
    apply_settings_patch_to_inner(patch, path, None).map_err(|error| error.to_string())
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
    apply_settings_patch_to_inner(patch, path, Some(expected))
}

fn apply_settings_patch_to_inner(
    patch: &SettingsPatch,
    path: &std::path::Path,
    expected: Option<&Config>,
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
        patch_settings_document(&mut doc, &config, patch);

        let text = doc.to_string();
        let mut written: Config = toml::from_str(&text).map_err(|e| {
            SettingsPatchApplyError::Persistence(format!(
                "written config could not be reloaded: {e}"
            ))
        })?;
        written.clamp();
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

    patch_section(&mut doc, "font", |t| {
        t.insert("family", toml_edit::value(config.font.family.clone()));
        t.insert("size", toml_edit::value(f64::from(config.font.size)));
        t.insert(
            "editor_size",
            toml_edit::value(f64::from(config.font.editor_size)),
        );
        t.insert(
            "agent_chat_size",
            toml_edit::value(f64::from(config.font.agent_chat_size)),
        );
        t.insert(
            "vertical_spacing",
            toml_edit::value(f64::from(config.font.vertical_spacing)),
        );
        t.insert(
            "horizontal_spacing",
            toml_edit::value(f64::from(config.font.horizontal_spacing)),
        );
        t.insert("inset_x", toml_edit::value(f64::from(config.font.inset_x)));
        t.insert("inset_y", toml_edit::value(f64::from(config.font.inset_y)));
    });

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
            "default_permission_mode",
            toml_edit::value(config.agent.default_permission_mode.mode_id()),
        );
        t.insert(
            "use_modifier_to_send",
            toml_edit::value(config.agent.use_modifier_to_send),
        );
        t.insert(
            "input_max_rows",
            toml_edit::value(i64::from(config.agent.input_max_rows)),
        );
    });

    patch_section(&mut doc, "telegram", |t| {
        t.insert("enabled", toml_edit::value(config.telegram.enabled));
        t.insert(
            "defer_while_active",
            toml_edit::value(config.telegram.defer_while_active),
        );
        t.insert(
            "active_idle_secs",
            toml_edit::value(config.telegram.active_idle_secs as i64),
        );
        match config.telegram.authorized_chat_id {
            Some(id) => {
                t.insert("authorized_chat_id", toml_edit::value(id));
            }
            None => {
                t.remove("authorized_chat_id");
            }
        }
    });

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

/// Patch one section at a time. Creates the table if absent so saving from a
/// fresh install produces a minimal document.
fn patch_section(
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

fn read_config_text(path: &std::path::Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("failed to read config: {error}")),
    }
}

fn parse_config_text(text: &str) -> Result<Config, SettingsPatchApplyError> {
    let mut config = if text.trim().is_empty() {
        Config::default()
    } else {
        toml::from_str::<Config>(text).map_err(|e| {
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
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::canonicalize(path)
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

fn patch_settings_document(
    doc: &mut toml_edit::DocumentMut,
    config: &Config,
    patch: &SettingsPatch,
) {
    match patch {
        SettingsPatch::GeneralLanguage(_) => patch_section(doc, "general", |t| {
            t.insert(
                "language",
                toml_edit::value(config.general.language.clone()),
            );
        }),
        SettingsPatch::TerminalPreset(_) => patch_section(doc, "theme", |t| {
            t.remove("preset");
            t.insert(
                "terminal_preset",
                toml_edit::value(config.theme.terminal_preset.clone()),
            );
        }),
        SettingsPatch::UiPreset(_) => patch_section(doc, "theme", |t| {
            t.insert(
                "ui_preset",
                toml_edit::value(config.theme.ui_preset.clone()),
            );
        }),
        SettingsPatch::FontFamily(_) => patch_section(doc, "font", |t| {
            t.insert("family", toml_edit::value(config.font.family.clone()));
        }),
        SettingsPatch::FontSize(_) => patch_section(doc, "font", |t| {
            t.insert("size", toml_edit::value(f64::from(config.font.size)));
        }),
        SettingsPatch::EditorFontSize(_) => patch_section(doc, "font", |t| {
            t.insert(
                "editor_size",
                toml_edit::value(f64::from(config.font.editor_size)),
            );
        }),
        SettingsPatch::AgentChatFontSize(_) => patch_section(doc, "font", |t| {
            t.insert(
                "agent_chat_size",
                toml_edit::value(f64::from(config.font.agent_chat_size)),
            );
        }),
        SettingsPatch::VerticalSpacing(_) => patch_section(doc, "font", |t| {
            t.insert(
                "vertical_spacing",
                toml_edit::value(f64::from(config.font.vertical_spacing)),
            );
        }),
        SettingsPatch::HorizontalSpacing(_) => patch_section(doc, "font", |t| {
            t.insert(
                "horizontal_spacing",
                toml_edit::value(f64::from(config.font.horizontal_spacing)),
            );
        }),
        SettingsPatch::CursorStyle(_) => patch_section(doc, "cursor", |t| {
            let value = match config.cursor.style {
                CursorStyle::Block => "block",
                CursorStyle::Underline => "underline",
                CursorStyle::Bar => "bar",
            };
            t.insert("style", toml_edit::value(value));
        }),
        SettingsPatch::CursorBlinking(_) => patch_section(doc, "cursor", |t| {
            t.insert("blinking", toml_edit::value(config.cursor.blinking));
        }),
        SettingsPatch::AgentPermissionMode(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "default_permission_mode",
                toml_edit::value(config.agent.default_permission_mode.mode_id()),
            );
        }),
        SettingsPatch::AgentUseModifierToSend(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "use_modifier_to_send",
                toml_edit::value(config.agent.use_modifier_to_send),
            );
        }),
        SettingsPatch::AgentCatalog(_) => replace_agents(doc, &config.agents),
        SettingsPatch::SessionHosts { .. } => {
            replace_session_hosts(doc, &config.session_hosts, &config.session_host_tombstones)
        }
        SettingsPatch::RenderMaxFps(_) => patch_section(doc, "render", |t| {
            t.insert(
                "max_fps",
                toml_edit::value(i64::from(config.render.max_fps)),
            );
        }),
        SettingsPatch::ShellClosePaneOnExit(_) => patch_section(doc, "shell", |t| {
            t.insert(
                "close_pane_on_exit",
                toml_edit::value(config.shell.close_pane_on_exit),
            );
        }),
        SettingsPatch::WindowOpacity(_) => patch_section(doc, "window", |t| {
            t.insert(
                "opacity",
                toml_edit::value(f64::from(config.window.opacity)),
            );
        }),
        SettingsPatch::WindowBlur(_) => patch_section(doc, "window", |t| {
            t.insert("blur", toml_edit::value(config.window.blur));
        }),
        SettingsPatch::ScrollbackMaxRows(_) => patch_section(doc, "scrollback", |t| {
            t.insert(
                "max_rows",
                toml_edit::value(config.scrollback.max_rows as i64),
            );
        }),
        SettingsPatch::TerminalInsetX(_) => patch_section(doc, "font", |t| {
            t.insert("inset_x", toml_edit::value(f64::from(config.font.inset_x)));
        }),
        SettingsPatch::TerminalInsetY(_) => patch_section(doc, "font", |t| {
            t.insert("inset_y", toml_edit::value(f64::from(config.font.inset_y)));
        }),
        SettingsPatch::FilesShowHidden(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "files_show_hidden",
                toml_edit::value(config.left_dock.files_show_hidden),
            );
        }),
        SettingsPatch::FilesUseGitignore(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "files_use_gitignore",
                toml_edit::value(config.left_dock.files_use_gitignore),
            );
        }),
        SettingsPatch::SyntaxTheme(_) => patch_section(doc, "file_viewer", |t| {
            t.insert(
                "syntax_theme",
                toml_edit::value(config.file_viewer.syntax_theme.clone()),
            );
        }),
        SettingsPatch::ClipboardStreamingMaxBytes(_) => patch_section(doc, "clipboard", |t| {
            t.insert(
                "streaming_max_bytes",
                toml_edit::value(config.clipboard.streaming_max_bytes as i64),
            );
        }),
        SettingsPatch::PreferredEditor(_) => patch_section(doc, "editor", |t| {
            t.insert(
                "preferred",
                toml_edit::value(config.editor.preferred.clone()),
            );
        }),
        SettingsPatch::PanelsGridColumns(_) => patch_section(doc, "panels", |t| {
            t.insert(
                "grid_columns",
                toml_edit::value(i64::from(config.panels.grid_columns)),
            );
        }),
        SettingsPatch::ToggleStatusBarItem(_) => patch_section(doc, "status_bar", |t| {
            let mut items = toml_edit::Array::new();
            for item in &config.status_bar.hidden_items {
                items.push(status_bar_item_slug(*item));
            }
            t.insert("hidden_items", toml_edit::value(items));
            t.remove("visible_items");
        }),
        SettingsPatch::ClaudeStatusEnabled(_) => patch_section(doc, "claude_status", |t| {
            t.insert("enable", toml_edit::value(config.claude_status.enable));
        }),
        SettingsPatch::TelegramEnabled(_) => patch_section(doc, "telegram", |t| {
            t.insert("enabled", toml_edit::value(config.telegram.enabled));
        }),
        SettingsPatch::TelegramAuthorizedChatId(_) => patch_section(doc, "telegram", |t| {
            if let Some(id) = config.telegram.authorized_chat_id {
                t.insert("authorized_chat_id", toml_edit::value(id));
            } else {
                t.remove("authorized_chat_id");
            }
        }),
    }
}

fn replace_agents(doc: &mut toml_edit::DocumentMut, entries: &[AgentEntry]) {
    let mut tables = toml_edit::ArrayOfTables::new();
    for entry in entries {
        tables.push(agent_entry_table(entry));
    }
    doc.insert("agents", toml_edit::Item::ArrayOfTables(tables));
}

fn replace_session_hosts(
    doc: &mut toml_edit::DocumentMut,
    entries: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) {
    replace_array_of_tables(doc, "session_hosts", entries, session_host_entry_table);
    replace_array_of_tables(
        doc,
        "session_host_tombstones",
        tombstones,
        session_host_tombstone_table,
    );
}

fn replace_array_of_tables<T>(
    doc: &mut toml_edit::DocumentMut,
    key: &str,
    values: &[T],
    table: impl Fn(&T) -> toml_edit::Table,
) {
    if values.is_empty() {
        doc.remove(key);
        return;
    }
    let mut tables = toml_edit::ArrayOfTables::new();
    for value in values {
        tables.push(table(value));
    }
    doc.insert(key, toml_edit::Item::ArrayOfTables(tables));
}

fn session_host_entry_table(entry: &SessionHostEntry) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["id"] = toml_edit::value(entry.id.as_inner().to_string());
    table["label"] = toml_edit::value(entry.label.clone());
    table["kind"] = toml_edit::Item::Table(session_host_kind_table(&entry.kind));
    table
}

fn session_host_tombstone_table(entry: &SessionHostTombstone) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["old_id"] = toml_edit::value(entry.old_id.as_inner().to_string());
    table["value"] = toml_edit::value(entry.value.clone());
    table["removed_at"] = toml_edit::value(entry.removed_at as i64);
    if let Some(id) = entry.redirected_to {
        table["redirected_to"] = toml_edit::value(id.as_inner().to_string());
    }
    table["kind"] = toml_edit::Item::Table(session_host_kind_table(&entry.kind));
    table
}

fn session_host_kind_table(kind: &SessionHostKind) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    match kind {
        SessionHostKind::Ssh { target } => {
            table["type"] = toml_edit::value("ssh");
            table["target"] = toml_edit::value(target.clone());
        }
        SessionHostKind::Docker { container } => {
            table["type"] = toml_edit::value("docker");
            table["container"] = toml_edit::value(container.clone());
        }
    }
    table
}

fn status_bar_item_slug(item: StatusBarItem) -> &'static str {
    match item {
        StatusBarItem::ProjectBranch => "project_branch",
        StatusBarItem::AccountSlot => "account_slot",
        StatusBarItem::Ports => "ports",
        StatusBarItem::ClaudeUsage => "claude_usage",
        StatusBarItem::Flow => "flow",
    }
}

/// One `[[agents]]` table as [`patch_config_file_to`] writes it. Hand-built
/// rather than serialized because the surrounding document is a `toml_edit`
/// tree that preserves the user's comments and formatting — which means this
/// function is the real persistence boundary for the catalog and has to mirror
/// [`AgentEntry`]'s shape exactly: a reference writes `preset` plus only the
/// overrides that are set, so an untouched field keeps tracking the preset, and
/// an entry daruda cannot resolve is written back unchanged instead of pruned.
///
/// Scalars are written before the `ssh` / `docker` sub-tables — TOML gives a
/// value after a table to that table.
fn agent_entry_table(entry: &AgentEntry) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    match entry {
        AgentEntry::Preset { preset, overrides } => {
            table["preset"] = toml_edit::value(preset.clone());
            if let Some(name) = &overrides.name {
                table["name"] = toml_edit::value(name.clone());
            }
            if let Some(command) = &overrides.command {
                table["command"] = toml_edit::value(command.clone());
            }
            if let Some(default_mode) = &overrides.default_mode {
                table["default_mode"] = toml_edit::value(default_mode.clone());
            }
        }
        AgentEntry::Custom(agent) => {
            table["id"] = toml_edit::value(agent.id.clone());
            table["name"] = toml_edit::value(agent.name.clone());
            // A remote launch has no flat `command` key; it yields the sub-table
            // held back until every scalar key is in place.
            let remote = match &agent.launch {
                AgentLaunch::Raw(command) => {
                    table["command"] = toml_edit::value(command.clone());
                    None
                }
                AgentLaunch::Ssh {
                    adapter_command,
                    host,
                } => {
                    let mut ssh = toml_edit::Table::new();
                    ssh["adapter_command"] = toml_edit::value(adapter_command.clone());
                    ssh["host"] = toml_edit::value(host.clone());
                    Some(("ssh", ssh))
                }
                AgentLaunch::Docker {
                    adapter_command,
                    container,
                } => {
                    let mut docker = toml_edit::Table::new();
                    docker["adapter_command"] = toml_edit::value(adapter_command.clone());
                    docker["container"] = toml_edit::value(container.clone());
                    Some(("docker", docker))
                }
            };
            if let Some(default_mode) = &agent.default_mode {
                table["default_mode"] = toml_edit::value(default_mode.clone());
            }
            if let Some((key, sub_table)) = remote {
                table[key] = toml_edit::Item::Table(sub_table);
            }
        }
    }
    table
}
