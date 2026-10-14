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
mod config_file;
pub mod cursor;
pub mod desktop;
pub mod editor;
pub mod env_lines;
pub mod file_viewer;
pub mod flow;
pub mod font;
pub mod general;
pub mod git;
pub mod keybindings;
pub mod left_dock;
pub mod logs;
pub mod notifications;
pub mod orchestrator;
pub mod panels;
pub mod ports;
pub mod presence;
pub mod project;
pub mod remote;
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

pub use account_env::{AccountEnv, account_env};
pub use agent::{
    ACP_REGISTRY_URL, ACP_REGISTRY_VERSION, AgentConfig, AgentDefinition, AgentEntry, AgentLaunch,
    AgentPreset, AgentSource, AgentVocabularySeed, CODEX_CONFIG_ENV, LaunchTransport,
    PresetLaunchability, PresetOverrides, READING_WIDTH_DEFAULT, READING_WIDTH_MAX,
    READING_WIDTH_MIN, SessionPreferences, TAIL_WINDOW_ALL, TAIL_WINDOW_CHOICES,
    TAIL_WINDOW_DEFAULT, TranscriptDefault, account_recipe_for_local_command, agent_preset,
    agent_presets, agent_vocabulary_seed, assemble_launch_command, canonical_env,
    is_valid_env_name, with_transcript_default,
};
pub use claude_status::ClaudeStatusConfig;
pub use clipboard::ClipboardConfig;
pub use colors::{AnsiPalette, ColorConfig, HexColor};
pub use config_file::{
    SettingsPatchApplyError, apply_settings_patch, apply_settings_patch_to,
    apply_settings_patch_to_if_unchanged, config_path, patch_config_file, patch_config_file_to,
    reset_settings_field_to_if_unchanged,
};
use config_file::{deserialize_config, patch_section};
pub use git::GitConfig;

/// `value` clamped to `[min, max]`, or `fallback` when it is NaN. TOML reads
/// `nan` as a float and `f32::clamp` passes NaN through, so a NaN size, width
/// or alpha would otherwise reach every consumer — and, comparing unequal to
/// itself, read as "changed" on every reload.
pub(crate) fn clamp_or(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_nan() {
        fallback
    } else {
        value.clamp(min, max)
    }
}
pub use cursor::{CursorConfig, CursorStyle};
pub use desktop::KeepAwakeMode;
pub use editor::{
    EditorConfig, ExternalEditorPreset, PRESETS as EXTERNAL_EDITOR_PRESETS,
    preset as external_editor_preset,
};
pub use file_viewer::FileViewerConfig;
pub use font::{FontConfig, SYSTEM_UI_FONT_FAMILY};
pub use general::{GeneralConfig, SUPPORTED_LOCALES};
pub use keybindings::KeybindingConfig;
pub use left_dock::{IconColorMode, LeftDockConfig};
pub use logs::LogsConfig;
pub use notifications::NotificationsConfig;
pub use orchestrator::OrchestratorConfig;
pub use panels::PanelsConfig;
pub use ports::PortsConfig;
pub use presence::PresenceConfig;
pub use project::{ProjectConfig, project_config_dir_in, project_config_path_in, project_id};
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
/// defaults. `ui_preset = "system"` follows the OS light / dark
/// appearance (see [`ui_theme_presets::resolve`]).
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
    pub desktop: desktop::DesktopConfig,
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
    pub git: GitConfig,
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
    pub presence: PresenceConfig,
    pub telegram: TelegramConfig,
    pub remote: remote::RemoteConfig,
    pub orchestrator: OrchestratorConfig,
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
            desktop: Default::default(),
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
            git: Default::default(),
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
            presence: Default::default(),
            telegram: Default::default(),
            remote: Default::default(),
            orchestrator: Default::default(),
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
                let mut cfg = deserialize_config(&text).unwrap_or_default();
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
        let legacy_default_mode = self.agent.take_legacy_default_permission_mode();
        let legacy_transcript = self.agent.take_legacy_transcript();
        self.font.clamp();
        self.window.clamp();
        self.left_dock.clamp();
        self.claude_status.clamp();
        self.panels.clamp();
        self.render.clamp();
        self.agent.clamp();
        self.status_bar.clamp();
        self.presence.clamp();
        self.desktop.clamp();
        // A missing `[[agents]]` is handled by the serde field default, and the
        // manual `Config::default()` (used on load errors) seeds the Claude
        // default directly (since a232e44). This guard exists for the remaining
        // case: an explicit `agents = []` in user TOML, which deserializes to the
        // provided empty array (not the field default) — normalize it back to the
        // Claude default so every load path hands the app a non-empty catalog.
        if self.agents.is_empty() {
            self.agents = agent::default_agents();
        }
        if let Some(mode) = legacy_default_mode {
            migrate_legacy_default_permission_mode(&mut self.agents, &mode);
        }
        migrate_legacy_transcript(&mut self.agents, &legacy_transcript);
    }

    /// The launchable agent catalog: every switched-on [`AgentEntry`] that
    /// resolves, in config order. A switched-off entry stays in [`Self::agents`]
    /// with its overrides, for Settings to switch back on. An entry
    /// referencing a preset daruda no longer knows is
    /// skipped — it stays in [`Self::agents`] (so a save preserves it and the
    /// Settings editor can flag it) but never reaches the runtime, where a
    /// nameless, commandless agent would only be selectable-then-broken.
    ///
    /// Falls back to the built-in default when nothing resolves, upholding the
    /// same non-empty-catalog invariant [`Self::clamp`] enforces for an
    /// explicitly-empty array — every consumer relies on `catalog[0]` existing.
    pub fn resolved_agents(&self) -> Vec<AgentDefinition> {
        let resolved: Vec<AgentDefinition> = self
            .agents
            .iter()
            .filter(|entry| entry.enabled)
            .filter_map(AgentEntry::resolve)
            .collect();
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
        let ProjectConfig { shell } = project;
        if let Some(shell) = shell.clone() {
            self.shell = shell;
        }
        self
    }
}

fn migrate_legacy_default_permission_mode(agents: &mut [AgentEntry], mode: &str) {
    for entry in agents {
        let Some(definition) = entry.resolve() else {
            continue;
        };
        if definition
            .default_mode
            .as_deref()
            .is_some_and(|m| !m.trim().is_empty())
        {
            continue;
        }
        if legacy_mode_can_apply_to(&definition.launch, mode) {
            set_agent_entry_default_mode(entry, mode);
        }
    }
}

fn legacy_mode_can_apply_to(launch: &AgentLaunch, mode: &str) -> bool {
    let command = match launch {
        AgentLaunch::Raw(command) => command,
        AgentLaunch::Ssh {
            adapter_command, ..
        }
        | AgentLaunch::Docker {
            adapter_command, ..
        } => adapter_command,
    };
    match agent_vocabulary_seed(command) {
        Some(seed) => seed.modes.iter().any(|entry| entry.id == mode),
        // Unknown adapters previously received the global candidate too; keep
        // that safe behavior because `daruda_acp` skips unadvertised modes.
        None => true,
    }
}

/// Lift the pre-catalog `[agent]` transcript keys onto every entry that states
/// nothing on that axis — which is what those keys used to do at resolve time.
///
/// Per axis, so an entry that already states one keeps it. Nothing writes the
/// `[agent]` keys back, so this runs once in practice: the next save moves the
/// value into `[[agents]]` and the old key, left untouched in the file by the
/// `toml_edit` patcher, stops being read.
fn migrate_legacy_transcript(agents: &mut [AgentEntry], legacy: &agent::LegacyTranscript) {
    if legacy.is_empty() {
        return;
    }
    for entry in agents {
        let Some(definition) = entry.resolve() else {
            continue;
        };
        let fold_mode = definition
            .fold_mode
            .is_none()
            .then(|| legacy.fold_mode.clone())
            .flatten();
        let tail_window = definition
            .tail_window
            .is_none()
            .then_some(legacy.tail_window)
            .flatten();
        let display_filter = definition
            .display_filter
            .is_none()
            .then(|| legacy.display_filter.clone())
            .flatten();
        set_agent_entry_transcript(entry, fold_mode, tail_window, display_filter);
    }
}

fn set_agent_entry_transcript(
    entry: &mut AgentEntry,
    fold_mode: Option<Vec<String>>,
    tail_window: Option<u8>,
    display_filter: Option<Vec<String>>,
) {
    match &mut entry.source {
        AgentSource::Preset { overrides, .. } => {
            overrides.fold_mode = overrides.fold_mode.take().or(fold_mode);
            overrides.tail_window = overrides.tail_window.take().or(tail_window);
            overrides.display_filter = overrides.display_filter.take().or(display_filter);
        }
        AgentSource::Custom(definition) => {
            definition.fold_mode = definition.fold_mode.take().or(fold_mode);
            definition.tail_window = definition.tail_window.take().or(tail_window);
            definition.display_filter = definition.display_filter.take().or(display_filter);
        }
    }
}

fn set_agent_entry_default_mode(entry: &mut AgentEntry, mode: &str) {
    match &mut entry.source {
        AgentSource::Preset { overrides, .. } => {
            overrides.default_mode = Some(mode.to_string());
        }
        AgentSource::Custom(definition) => {
            definition.default_mode = Some(mode.to_string());
        }
    }
}
