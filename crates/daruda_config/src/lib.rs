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
pub mod file_viewer;
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
use std::path::PathBuf;

pub use account_env::{AccountEnv, account_env};
pub use agent::{
    ACP_REGISTRY_URL, ACP_REGISTRY_VERSION, AgentConfig, AgentDefinition, AgentEntry, AgentLaunch,
    AgentPreset, DefaultPermissionMode, PresetLaunchability, PresetOverrides,
    READING_WIDTH_DEFAULT, READING_WIDTH_MAX, READING_WIDTH_MIN, account_recipe_for_local_command,
    agent_preset, agent_presets,
};
pub use claude_status::ClaudeStatusConfig;
pub use clipboard::ClipboardConfig;
pub use colors::{AnsiPalette, ColorConfig, HexColor};
pub use cursor::{CursorConfig, CursorStyle};
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

/// Like [`patch_config_file`] but writes to an explicit path.  Used by tests
/// so they can operate on a temp directory instead of the user's real config.
pub fn patch_config_file_to(config: &Config, path: &std::path::Path) -> Result<(), String> {
    // Clamp before writing so disk never holds out-of-range values, even if
    // the caller forgot. Out-of-range values would otherwise survive on disk
    // until the next `Config::load` call clamped them in memory.
    let mut clamped = config.clone();
    clamped.clamp();
    let config = &clamped;

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = existing
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("existing config has a parse error: {e}"))?;

    // Patch one section at a time.  Creates the table if absent so saving
    // from a fresh install produces a minimal file.
    fn patch_section(
        doc: &mut toml_edit::DocumentMut,
        key: &str,
        f: impl FnOnce(&mut toml_edit::Table),
    ) {
        if !doc.contains_key(key) {
            doc.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        if let Some(t) = doc.get_mut(key).and_then(|i| i.as_table_mut()) {
            f(t);
        }
    }

    patch_section(&mut doc, "general", |t| {
        t["language"] = toml_edit::value(config.general.language.clone());
    });

    patch_section(&mut doc, "theme", |t| {
        // Drop the legacy `preset` key on first write so `config.toml` ends
        // up with only `terminal_preset` (cell palette) / `ui_preset`
        // (chrome palette).
        t.remove("preset");
        t["terminal_preset"] = toml_edit::value(config.theme.terminal_preset.clone());
        t["ui_preset"] = toml_edit::value(config.theme.ui_preset.clone());
    });

    patch_section(&mut doc, "font", |t| {
        t["family"] = toml_edit::value(config.font.family.clone());
        t["size"] = toml_edit::value(f64::from(config.font.size));
        t["editor_size"] = toml_edit::value(f64::from(config.font.editor_size));
        t["agent_chat_size"] = toml_edit::value(f64::from(config.font.agent_chat_size));
        t["vertical_spacing"] = toml_edit::value(f64::from(config.font.vertical_spacing));
        t["horizontal_spacing"] = toml_edit::value(f64::from(config.font.horizontal_spacing));
        t["inset_x"] = toml_edit::value(f64::from(config.font.inset_x));
        t["inset_y"] = toml_edit::value(f64::from(config.font.inset_y));
    });

    patch_section(&mut doc, "cursor", |t| {
        let style_str = match config.cursor.style {
            CursorStyle::Block => "block",
            CursorStyle::Underline => "underline",
            CursorStyle::Bar => "bar",
        };
        t["style"] = toml_edit::value(style_str);
        t["blinking"] = toml_edit::value(config.cursor.blinking);
    });

    patch_section(&mut doc, "shell", |t| {
        t["close_pane_on_exit"] = toml_edit::value(config.shell.close_pane_on_exit);
    });

    patch_section(&mut doc, "window", |t| {
        t["opacity"] = toml_edit::value(f64::from(config.window.opacity));
        t["blur"] = toml_edit::value(config.window.blur);
    });

    patch_section(&mut doc, "scrollback", |t| {
        t["max_rows"] = toml_edit::value(config.scrollback.max_rows as i64);
    });

    patch_section(&mut doc, "left_dock", |t| {
        t["files_show_hidden"] = toml_edit::value(config.left_dock.files_show_hidden);
        t["files_use_gitignore"] = toml_edit::value(config.left_dock.files_use_gitignore);
    });

    patch_section(&mut doc, "file_viewer", |t| {
        t["syntax_theme"] = toml_edit::value(config.file_viewer.syntax_theme.clone());
        t["preview_tab"] = toml_edit::value(config.file_viewer.preview_tab);
    });

    patch_section(&mut doc, "clipboard", |t| {
        t["streaming_max_bytes"] = toml_edit::value(config.clipboard.streaming_max_bytes as i64);
    });

    patch_section(&mut doc, "panels", |t| {
        t["grid_columns"] = toml_edit::value(i64::from(config.panels.grid_columns));
    });

    patch_section(&mut doc, "status_bar", |t| {
        let mut arr = toml_edit::Array::new();
        for item in &config.status_bar.visible_items {
            let slug = match item {
                StatusBarItem::ProjectBranch => "project_branch",
                StatusBarItem::AccountSlot => "account_slot",
                StatusBarItem::Ports => "ports",
                StatusBarItem::ClaudeUsage => "claude_usage",
            };
            arr.push(slug);
        }
        t["visible_items"] = toml_edit::value(arr);
    });

    patch_section(&mut doc, "ports", |t| {
        t["poll_secs"] = toml_edit::value(config.ports.poll_secs as i64);
    });

    patch_section(&mut doc, "claude_status", |t| {
        t["enable"] = toml_edit::value(config.claude_status.enable);
    });

    patch_section(&mut doc, "agent", |t| {
        t["default_permission_mode"] =
            toml_edit::value(config.agent.default_permission_mode.mode_id());
        t["use_modifier_to_send"] = toml_edit::value(config.agent.use_modifier_to_send);
        t["input_max_rows"] = toml_edit::value(i64::from(config.agent.input_max_rows));
    });

    patch_section(&mut doc, "telegram", |t| {
        t["enabled"] = toml_edit::value(config.telegram.enabled);
        t["defer_while_active"] = toml_edit::value(config.telegram.defer_while_active);
        t["active_idle_secs"] = toml_edit::value(config.telegram.active_idle_secs as i64);
        match config.telegram.authorized_chat_id {
            Some(id) => t["authorized_chat_id"] = toml_edit::value(id),
            None => {
                t.remove("authorized_chat_id");
            }
        }
    });

    if doc.contains_key("agents") || config.agents != agent::default_agents() {
        let mut agents = toml_edit::ArrayOfTables::new();
        for entry in &config.agents {
            agents.push(agent_entry_table(entry));
        }
        doc.insert("agents", toml_edit::Item::ArrayOfTables(agents));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())
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
