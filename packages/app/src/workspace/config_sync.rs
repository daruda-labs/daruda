//! Single source of truth for config-derived fields cached on
//! `Workspace`. `ConfigMirrors::from_config` is the only place that
//! reads `daruda_config::Config` into mirror state — adding a new
//! mirror field here forces the sync site to be updated, so sync
//! omissions become compile errors instead of silent state divergence.

use std::time::Duration;

use daruda_config::{Config, HexColor, IconColorMode, StatusBarConfig};
use daruda_terminal::TerminalConfig;

use super::main_area::agent_chat_pane::transcript_defaults::ReaderDefaults;

#[derive(Clone)]
pub(in crate::workspace) struct ConfigMirrors {
    /// Mirror of `daruda_config::PanelsConfig::grid_columns`. Drives
    /// the macro-key column count in `MacroDock`.
    pub panels_grid_columns: u8,

    /// Mirror of `daruda_config::ShellConfig::close_pane_on_exit`.
    /// When true, a pane closes itself after the PTY exits.
    pub close_pane_on_exit: bool,

    /// Mirror of `daruda_config::LeftDockConfig::files_show_hidden`.
    /// Drives the dotfile filter inside the Files view's `walk_into`.
    /// Toggled at runtime by `FilesToggleHidden`; `apply_config`
    /// overwrites it on live reload.
    pub files_show_hidden: bool,

    /// Mirror of `daruda_config::LeftDockConfig::files_use_gitignore`.
    /// When true, `walk_into` consults the lane's gitignore matcher per row.
    pub files_use_gitignore: bool,

    /// Mirror of `daruda_config::LeftDockConfig::file_icon_color_mode`.
    pub files_icon_color_mode: IconColorMode,

    /// PTY-output batch / repaint interval derived from
    /// `daruda_config::RenderConfig::max_fps` (`1000 / max_fps` ms).
    /// Read by the per-pane stdout poll loop to cap how often terminal
    /// output triggers a repaint; live-updates on config reload.
    pub terminal_redraw_interval: Duration,

    /// The bundled preset `daruda_config::ThemeConfig::ui_preset` paints —
    /// `system` resolved against the OS appearance, or a capture's override.
    /// A change flips the host appearance, so `apply_config` reloads open
    /// markdown panes to re-theme their rendered diagrams for the new surface.
    pub painted_ui_preset: String,

    /// Mirror of `daruda_config::StatusBarConfig`. Drives which segments
    /// `StatusBar::render` includes and whether the Ports scan pump
    /// (`workspace::sync::ports`) bothers scanning at all.
    pub status_bar: StatusBarConfig,

    /// Mirror of `daruda_config::PortsConfig::interval()`. Read by the
    /// Ports scan pump under `read_with` without locking the full
    /// `Config`, same shape as `terminal_redraw_interval`.
    pub ports_poll_interval: Duration,

    /// Mirror of `daruda_config::AgentConfig::hidden_config_option_descriptions`.
    /// Filters which advertised session config options get an input-dock chip.
    pub hidden_config_option_descriptions: Vec<String>,

    /// The values `apply_config` also writes to app-wide globals in
    /// `crate::ui::theme`. Kept here so each window diffs against what *it*
    /// last applied: every window writes the same global, so comparing
    /// against the global tells only the first window that anything moved.
    pub shared_surface: SharedSurface,

    /// Terminal config applied to every new pane (font size + iTerm2-style
    /// spacing multipliers). Single source of truth; `resize_all_tabs`
    /// reads the same settings to measure cells consistently with
    /// TerminalView.
    pub terminal_config: TerminalConfig,

    /// Primary font family from config. Applied to each new pane's
    /// TerminalView so user-specified fonts take effect.
    pub font_family: String,

    /// Effective shell program for new panes — `Some` only when a
    /// project layer (or the user `[shell]` section) sets `program`.
    /// `None` falls back to `$SHELL` / `/bin/zsh` via `PtyConfig::default`.
    /// Picked up by `create_pane_with_cwd` at spawn time; existing
    /// panes keep the program they were spawned with.
    pub shell_program: Option<String>,

    /// Syntect theme name for syntax highlighting in the file viewer.
    /// Updated on every config reload; threaded into background load tasks.
    pub syntax_theme: String,

    /// The app-wide agent-chat presentation a fresh pane starts on. The one
    /// mirror of `daruda_config::AgentConfig`'s reader axes, resolved here so
    /// the pane-creation and config-reload paths cannot read them differently.
    pub agent_reader_defaults: ReaderDefaults,

    /// When true, clicking a file in the left dock reuses the single
    /// existing file-viewer tab instead of opening one per file.
    /// Mirrors `daruda_config::FileViewerConfig::preview_tab`.
    pub file_viewer_preview_tab: bool,

    /// Preferred external-editor preset name (`daruda_config::editor`), or
    /// empty for the OS default handler. Mirrors
    /// `daruda_config::EditorConfig::preferred`.
    pub preferred_editor: String,

    /// Notification + user-attention gates. Drives whether OSC 9 / 777 /
    /// 1337 RequestAttention surface to the OS. Read by per-pane
    /// `TerminalViewEvent` subscriptions and by the long-running command
    /// timer.
    pub notifications: daruda_config::NotificationsConfig,

    /// `[git]` — whether the Git panel's commit and push ask first.
    pub git_config: daruda_config::GitConfig,

    /// Telegram bot bridge settings — gates `relay_to_telegram` (both
    /// `enabled` and a completed pairing are required before a ping is
    /// queued). Mirrored from the live config the same way
    /// `notifications` is, so a Settings-window toggle takes effect
    /// without any extra plumbing.
    pub telegram: daruda_config::TelegramConfig,

    /// Agent chat configuration — permission mode applied on connect.
    pub agent: daruda_config::AgentConfig,

    /// The agent catalog mirrored from config `[[agents]]`, already resolved
    /// (`Config::resolved_agents`) — preset references expanded, entries that
    /// resolve to nothing dropped. A newly opened pane runs under `agents[0]`;
    /// each pane resolves its `agent_id` to a launch command here at connect
    /// time. Guaranteed non-empty by the config layer.
    pub agents: std::sync::Arc<[daruda_config::AgentDefinition]>,

    /// `[flow]` — the budget every run starts with. Cached from config
    /// like the other config mirrors, refreshed in `apply_config`.
    pub flow_config: daruda_config::flow::FlowConfig,

    /// The registered SSH/Docker host catalog mirrored from config
    /// `[[session_hosts]]` — a lane's `session_host.registry_id` resolves
    /// against this via `lane::session_host::effective_session_host`.
    pub session_hosts: Vec<daruda_config::SessionHostEntry>,

    /// Removed catalog rows mirrored from config `[[session_host_tombstones]]`
    /// — chased when a `registry_id` no longer resolves in `session_hosts`,
    /// so a merge (`redirected_to`) still re-resolves. See
    /// `lane::session_host::effective_session_host`.
    pub session_host_tombstones: Vec<daruda_config::SessionHostTombstone>,

    /// Background-poll cadences for the OAuth `/api/oauth/usage` and
    /// public `status.claude.com` endpoints, read by `limits_pump` under
    /// `read_with` without locking the full `Config`.
    pub usage_poll: daruda_config::PollConfig,

    /// Whether the Claude status feature is enabled in `[claude_status]`
    /// config. False suppresses both the indicator and the install banner.
    pub claude_status_enabled: bool,

    /// `[claude_status] stale_threshold_secs` — the same age past which
    /// cold restore resets a session also expires its blocking
    /// notification for the local/remote push gate (see
    /// `maybe_push_hook_notification`).
    pub stale_threshold_secs: u64,
}

/// The config slice mirrored into app-wide `crate::ui::theme` globals; see
/// [`ConfigMirrors::shared_surface`]. Each field gates a rebuild in
/// `apply_config`, so equality is the whole contract.
#[derive(Clone, PartialEq)]
pub(in crate::workspace) struct SharedSurface {
    pub editor_font: (String, f32, f32),
    pub agent_chat_font: (String, f32, f32),
    pub agent_chat_reading_width: f32,
    pub window_opacity: f32,
    pub terminal_fg: HexColor,
    pub terminal_bg: HexColor,
}

impl SharedSurface {
    fn from_config(config: &Config) -> Self {
        let editor = &config.font.editor;
        let chat = &config.font.agent_chat;
        let colors = config.effective_colors();
        Self {
            editor_font: (editor.family.clone(), editor.size, editor.line_height),
            agent_chat_font: (chat.family.clone(), chat.size, chat.line_height),
            agent_chat_reading_width: config.agent.reading_width,
            window_opacity: config.window.opacity,
            terminal_fg: colors.foreground,
            terminal_bg: colors.background,
        }
    }
}

impl ConfigMirrors {
    /// `painted_ui_preset` is what `config`'s preset paints now — see
    /// `crate::ui::theme::painted_ui_preset`, which owns that answer.
    pub(in crate::workspace) fn from_config(config: &Config, painted_ui_preset: String) -> Self {
        Self {
            panels_grid_columns: config.panels.grid_columns,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            files_icon_color_mode: config.left_dock.file_icon_color_mode.clone(),
            terminal_redraw_interval: config.render.redraw_interval(),
            painted_ui_preset,
            status_bar: config.status_bar.clone(),
            ports_poll_interval: config.ports.interval(),
            hidden_config_option_descriptions: config
                .agent
                .hidden_config_option_descriptions
                .clone(),
            shared_surface: SharedSurface::from_config(config),
            // The single config → terminal-config mapping; its resolved
            // colours patch every live pane in `apply_config_to_terminals`.
            terminal_config: super::config_ops::terminal_config_from(config),
            font_family: config.font.terminal.family.clone(),
            shell_program: config.shell.program.clone(),
            syntax_theme: config.file_viewer.syntax_theme.clone(),
            agent_reader_defaults: ReaderDefaults::from_config(&config.agent),
            file_viewer_preview_tab: config.file_viewer.preview_tab,
            preferred_editor: config.editor.preferred.clone(),
            notifications: config.notifications.clone(),
            git_config: config.git.clone(),
            telegram: config.telegram.clone(),
            agent: config.agent.clone(),
            agents: config.resolved_agents().into(),
            flow_config: config.flow.clone(),
            session_hosts: config.session_hosts.clone(),
            session_host_tombstones: config.session_host_tombstones.clone(),
            usage_poll: config.usage.poll.clone(),
            claude_status_enabled: config.claude_status.enable,
            stale_threshold_secs: config.claude_status.stale_threshold_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    // Integration coverage lives in workspace/tests/config_mirror.rs —
    // apply_config_syncs_all_mirrors + toggle_files_show_hidden_flips_mirror.
}
