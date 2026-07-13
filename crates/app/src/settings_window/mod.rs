//! Settings window — edit common daruda config options in a standalone window.
//!
//! Opened via `Cmd+,` (`OpenSettings(SettingsSection::default())`) or any
//! menu / palette entry that constructs an `OpenSettings(section)`. The
//! window is a singleton: a second open call routes through
//! [`SettingsWindow::focus_section`] to switch the active page rather
//! than spawning a duplicate window.
//!
//! ## Layout (Phase 1)
//!
//! ```text
//! ┌───────────────────────────────────────┐
//! │  [traffic-light bar / title]          │
//! ├──────────┬────────────────────────────┤
//! │          │                            │
//! │ sidebar  │   body for active section  │
//! │ (nav)    │   (form fields)            │
//! │          │                            │
//! ├──────────┴────────────────────────────┤
//! │             [Cancel] [Save]           │
//! └───────────────────────────────────────┘
//! ```
//!
//! Each section is a method on `SettingsWindow` (see `sections.rs`).
//! Adding a new builtin section is:
//!   1. Add the variant to [`daruda_config::BuiltinSection`] +
//!      `BuiltinSection::ALL` + `slug()`.
//!   2. Add a `SETTINGS_NAV_*` const + `SETTINGS_SECTION_*` header
//!      in `surface/strings.rs`.
//!   3. Add a `render_<section>` method in `sections.rs` and route
//!      from `render::render_section_body`.
//!   4. (Optional) extend `validate()` / `submit()` if the section
//!      mutates `Config`.

mod render;
mod sections;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, IntoElement, SharedString, Subscription, Task,
    Window, WindowBackgroundAppearance, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::{InputEvent, InputState};
use crate::window_registry::WindowRegistry;

pub struct SettingsWindow {
    panel_focus_handle: FocusHandle,
    /// Snapshot of the config at window-open time. `validate()` overlays form
    /// values on top of this snapshot so fields not exposed in the UI (e.g.
    /// `[colors]`, `[keybindings]`) are preserved on save.
    base_config: daruda_config::Config,
    /// Which page the body is currently rendering. Updated by sidebar
    /// click + by `focus_section` (called when a second `OpenSettings`
    /// dispatch arrives while the window is already open).
    active_section: BuiltinSection,
    /// Per-section first-input focus targets — used by `focus_section`
    /// so a sidebar click / menu jump immediately lands focus on the
    /// natural starting field (e.g. font size for the Font page).
    section_focus: HashMap<BuiltinSection, FocusHandle>,
    // ---- form fields ----
    // General page.
    language_select: Entity<SelectState>,
    // Theme (rendered inside the General page).
    // `terminal_preset_select` controls the cell palette (16-color
    // ANSI + fg/bg); `ui_preset_select` controls the chrome palette
    // (workspace, sidebar, modal, status bar). The two axes are
    // independent — see `daruda_config::ThemeConfig`.
    terminal_preset_select: Entity<SelectState>,
    ui_preset_select: Entity<SelectState>,
    // Font
    font_family_select: Entity<SelectState>,
    font_size_input: Entity<InputState>,
    editor_font_size_input: Entity<InputState>,
    agent_chat_font_size_input: Entity<InputState>,
    vertical_spacing_input: Entity<InputState>,
    horizontal_spacing_input: Entity<InputState>,
    // Cursor
    cursor_style_select: Entity<SelectState>,
    cursor_blinking: bool,
    // Agent
    default_permission_mode_select: Entity<SelectState>,
    agent_preset_select: Entity<SelectState>,
    agent_use_modifier_to_send: bool,
    agent_rows: Vec<AgentCatalogRow>,
    // Render
    max_fps_select: Entity<SelectState>,
    // Shell
    close_pane_on_exit: bool,
    // Window
    opacity_input: Entity<InputState>,
    window_blur: bool,
    // Terminal
    scrollback_input: Entity<InputState>,
    inset_x_input: Entity<InputState>,
    inset_y_input: Entity<InputState>,
    // Sidebar
    files_show_hidden: bool,
    files_use_gitignore: bool,
    // File Viewer
    syntax_theme_select: Entity<SelectState>,
    // Clipboard (Phase 1 — single field expose)
    clipboard_streaming_input: Entity<InputState>,
    // Panels (bottom-dock macro grid)
    panels_grid_columns_input: Entity<InputState>,
    // Claude Status (Phase 1 — toggle expose only)
    claude_status_enable: bool,
    // Notifications (Telegram)
    telegram_enabled: bool,
    telegram_token_input: Entity<InputState>,
    /// Presence-only cache of whether a token is currently stored in
    /// the Keychain — seeded once at construction, updated by the
    /// Save/Clear button handlers. Never holds the token itself.
    telegram_token_configured: bool,
    /// Transient UI-only state (never persisted): the pairing code
    /// most recently generated by "Generate Pairing Code", shown with
    /// the `/pair <code>` instructions until the window closes.
    telegram_pair_code: Option<String>,
    /// Whether the `/pair <code>` command was just copied — drives the
    /// Copy/Copied! label swap, mirrors `ErrorReportModal::copied`.
    telegram_pair_command_copied: bool,
    /// Reverts `telegram_pair_command_copied` after a short delay.
    /// Re-created on every copy click so a rapid second click resets
    /// the window, mirrors `ErrorReportModal::_copied_revert_task`.
    _telegram_pair_copy_revert_task: Option<Task<()>>,
    // Focus handles for Tab cycling (text inputs only)
    font_size_fh: FocusHandle,
    vertical_spacing_fh: FocusHandle,
    horizontal_spacing_fh: FocusHandle,
    opacity_fh: FocusHandle,
    scrollback_fh: FocusHandle,
    inset_x_fh: FocusHandle,
    inset_y_fh: FocusHandle,
    clipboard_streaming_fh: FocusHandle,
    panels_grid_columns_fh: FocusHandle,
    telegram_token_fh: FocusHandle,
    scroll_handle: gpui::ScrollHandle,
    _input_subscriptions: Vec<Subscription>,
    error: Option<SharedString>,
    /// Plugin ids (`<plugin>@<marketplace>`) with an `install` /
    /// `uninstall` CLI invocation currently spawned on the
    /// `background_executor`. Used by the Plugin section to show a
    /// transient `Installing…` / `Uninstalling…` label and to swallow
    /// duplicate clicks while the request is in flight.
    pub(super) plugin_ops_in_flight: std::collections::HashSet<String>,
    /// Last plugin-op error message — surfaced inline above the plugin
    /// list. `None` clears the banner.
    pub(super) plugin_last_error: Option<SharedString>,
    /// `<plugin>@<marketplace>` of the plugin whose detail pane is on
    /// the right side of the master-detail layout. `None` shows the
    /// "select a plugin" placeholder.
    pub(super) plugin_selected: Option<String>,
    /// When `Some`, the right pane swaps from the plugin detail to a
    /// SKILL.md viewer. Cleared by the `← Back` button.
    pub(super) plugin_view_skill: Option<PluginSkillView>,
    /// Subscription that calls `cx.notify()` whenever the app-wide
    /// `SkillsState` Global changes — so the Plugin page reflects
    /// install / uninstall completions (and external `claude plugin`
    /// CLI runs) without polling.
    _skills_global_subscription: Subscription,
    /// Subscription that calls `cx.notify()` whenever the `Updater`
    /// entity changes status — so the About page reflects check /
    /// download / install progress reactively. `None` when the updater
    /// global never registered (unparseable version). Observing the
    /// entity (not the global) is deliberate: the global holder is set
    /// once at init and never replaced, so `observe_global` would never
    /// fire; the entity self-notifies on every status transition.
    _updater_subscription: Option<Subscription>,
}

/// In-Settings SKILL.md viewer state. The body load is async (disk
/// read on the background executor) so the variant tracks the three
/// observable phases — request in flight, body ready, body failed.
#[derive(Clone)]
pub(super) struct PluginSkillView {
    /// `display_name_for_invocation(skill)` — the namespaced form
    /// shown in the header (`<plugin>:<skill>`).
    pub(super) display_name: String,
    /// Absolute path to the SKILL.md file being viewed. Captured at
    /// open time so the async loader knows exactly which file to read
    /// even if the underlying `SkillsState` reshuffles mid-load.
    pub(super) skill_md_path: std::path::PathBuf,
    pub(super) body: PluginSkillBodyState,
}

#[derive(Clone)]
pub(super) enum PluginSkillBodyState {
    Loading,
    Loaded(SharedString),
    Error(SharedString),
}

#[derive(Clone)]
pub(super) struct AgentCatalogRow {
    pub(super) id_input: Entity<InputState>,
    pub(super) name_input: Entity<InputState>,
    /// The command that runs the ACP adapter — `Raw`'s full string, or the
    /// `adapter_command` sub-field when `transport_select` is `ssh`/`docker`.
    pub(super) command_input: Entity<InputState>,
    /// Transport kind for this row: `"raw"` / `"ssh"` / `"docker"` — mirrors
    /// [`daruda_config::AgentLaunch`]'s three variants.
    pub(super) transport_select: Entity<SelectState>,
    /// SSH host — only meaningful (and only rendered) when `transport_select`
    /// is `"ssh"`.
    pub(super) host_input: Entity<InputState>,
    /// Docker container name — only meaningful (and only rendered) when
    /// `transport_select` is `"docker"`.
    pub(super) container_input: Entity<InputState>,
}

impl SettingsWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_section(BuiltinSection::default(), window, cx)
    }

    fn subscribe_input_state(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        )
    }

    fn agent_row_from_definition(
        definition: &daruda_config::AgentDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentCatalogRow {
        let id = definition.id.clone();
        let name = definition.name.clone();
        let (command, transport_kind, host, container) = match &definition.launch {
            daruda_config::AgentLaunch::Raw(command) => {
                (command.clone(), "raw", String::new(), String::new())
            }
            daruda_config::AgentLaunch::Ssh {
                adapter_command,
                host,
            } => (adapter_command.clone(), "ssh", host.clone(), String::new()),
            daruda_config::AgentLaunch::Docker {
                adapter_command,
                container,
            } => (
                adapter_command.clone(),
                "docker",
                String::new(),
                container.clone(),
            ),
        };
        let transport_kind = SharedString::from(transport_kind);
        AgentCatalogRow {
            id_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("agent-id")
                    .default_value(id)
            }),
            name_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("Display name")
                    .default_value(name)
            }),
            command_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("command --acp")
                    .default_value(command)
            }),
            transport_select: cx.new(|cx| {
                let opts = vec![
                    SelectOption::new("raw", s::settings_agent_transport_raw()),
                    SelectOption::new("ssh", s::settings_agent_transport_ssh()),
                    SelectOption::new("docker", s::settings_agent_transport_docker()),
                ];
                select::state_with_options(opts, Some(&transport_kind), window, cx)
            }),
            host_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("user@host")
                    .default_value(host)
            }),
            container_input: cx.new(|cx_state| {
                InputState::new(window, cx_state)
                    .placeholder("container-name")
                    .default_value(container)
            }),
        }
    }

    fn subscribe_agent_row(
        &mut self,
        row: &AgentCatalogRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._input_subscriptions
            .push(Self::subscribe_input_state(&row.id_input, window, cx));
        self._input_subscriptions
            .push(Self::subscribe_input_state(&row.name_input, window, cx));
        self._input_subscriptions
            .push(Self::subscribe_input_state(&row.command_input, window, cx));
        self._input_subscriptions
            .push(Self::subscribe_input_state(&row.host_input, window, cx));
        self._input_subscriptions.push(Self::subscribe_input_state(
            &row.container_input,
            window,
            cx,
        ));
        // Re-render on transport pick so the row immediately shows/hides the
        // matching host/container field (rows are added/removed at runtime,
        // unlike the fixed global dropdowns wired in `new_with_section`).
        self._input_subscriptions.push(cx.subscribe_in(
            &row.transport_select,
            window,
            |_this, _state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    cx.notify();
                }
            },
        ));
    }

    pub(super) fn add_agent_row(
        &mut self,
        definition: daruda_config::AgentDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = Self::agent_row_from_definition(&definition, window, cx);
        self.subscribe_agent_row(&row, window, cx);
        self.agent_rows.push(row);
        self.error = None;
        cx.notify();
    }

    pub(super) fn remove_agent_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.agent_rows.len() {
            self.agent_rows.remove(index);
            self.error = None;
            cx.notify();
        }
    }

    pub fn new_with_section(
        active: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Defensive idempotent init for test fixtures that open a
        // Settings window without going through `globals::init_all`.
        crate::settings_store::SettingsStore::init(cx);
        let config = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();

        // Language select — options driven by the canonical locale list so
        // adding a new locale only requires updating SUPPORTED_LOCALES.
        let lang = SharedString::from(config.general.language.clone());
        let language_select = cx.new(|cx| {
            let opts: Vec<select::SelectOption> = daruda_config::SUPPORTED_LOCALES
                .iter()
                .map(|&slug| {
                    let label = match slug {
                        "auto" => s::settings_language_auto(),
                        "en" => s::settings_language_en(),
                        "ko" => s::settings_language_ko(),
                        other => other.to_owned(),
                    };
                    select::SelectOption::new(slug, label)
                })
                .collect();
            select::state_with_options(opts, Some(&lang), window, cx)
        });

        // Terminal preset select — cell palette (fg/bg + ANSI 16).
        let terminal_preset = SharedString::from(config.theme.terminal_preset.clone());
        let terminal_preset_select = cx.new(|cx| {
            let opts = daruda_config::THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&terminal_preset), window, cx)
        });

        // UI preset select — chrome palette (workspace, modal,
        // status bar, …). Phase 2 ships one preset so the dropdown
        // renders but the user can't change it yet; Phase 3 will
        // populate this from `ThemeRegistry`.
        let ui_preset = SharedString::from(config.theme.ui_preset.clone());
        let ui_preset_select = cx.new(|cx| {
            let opts = daruda_config::UI_THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&ui_preset), window, cx)
        });

        let font_names = all_font_names(cx, &config.font.family);
        let font_family = SharedString::from(config.font.family.clone());
        let font_family_select = cx.new(|cx| {
            let opts = font_names
                .iter()
                .map(|n| SelectOption::simple(n.clone()))
                .collect();
            select::state_with_options(opts, Some(&font_family), window, cx)
        });

        let font_size_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 13")
                .default_value(format!("{}", config.font.size))
        });
        let editor_font_size_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 13")
                .default_value(format!("{}", config.font.editor_size))
        });
        let agent_chat_font_size_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 13")
                .default_value(format!("{}", config.font.agent_chat_size))
        });
        let vertical_spacing_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 1.0")
                .default_value(format!("{}", config.font.vertical_spacing))
        });
        let horizontal_spacing_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 1.0")
                .default_value(format!("{}", config.font.horizontal_spacing))
        });
        let opacity_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("0.1 – 1.0")
                .default_value(format!("{}", config.window.opacity))
        });
        let scrollback_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 10000")
                .default_value(format!("{}", config.scrollback.max_rows))
        });
        let inset_x_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 4")
                .default_value(format!("{}", config.font.inset_x))
        });
        let inset_y_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 2")
                .default_value(format!("{}", config.font.inset_y))
        });
        let clipboard_streaming_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("e.g. 10485760")
                .default_value(format!("{}", config.clipboard.streaming_max_bytes))
        });
        let panels_grid_columns_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder("1 – 16")
                .default_value(format!("{}", config.panels.grid_columns))
        });
        // Never pre-filled with the real token (`default_value`) — a
        // stored secret is never re-displayed in a text field. The
        // "Token configured" status line covers presence instead.
        let telegram_token_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::settings_telegram_token_placeholder())
                .masked(true)
        });
        let telegram_token_configured = crate::telegram::keychain::read_token().is_some();

        let cursor_style_str: SharedString = match config.cursor.style {
            daruda_config::CursorStyle::Block => "block".into(),
            daruda_config::CursorStyle::Underline => "underline".into(),
            daruda_config::CursorStyle::Bar => "bar".into(),
        };
        let cursor_style_select = cx.new(|cx| {
            select::state_with_options(
                vec![
                    SelectOption::new("block", s::settings_cursor_block()),
                    SelectOption::new("underline", s::settings_cursor_underline()),
                    SelectOption::new("bar", s::settings_cursor_bar()),
                ],
                Some(&cursor_style_str),
                window,
                cx,
            )
        });

        let permission_mode_str: SharedString =
            config.agent.default_permission_mode.mode_id().into();
        let default_permission_mode_select = cx.new(|cx| {
            use daruda_config::DefaultPermissionMode as M;
            // The dropdown shows just the bare mode id; the human-readable
            // explanation for the selected mode is rendered below the field
            // (see `render_agent`).
            let opts = M::ALL
                .into_iter()
                .map(|m| SelectOption::new(m.mode_id(), m.mode_id()))
                .collect();
            select::state_with_options(opts, Some(&permission_mode_str), window, cx)
        });

        let agent_preset = SharedString::from("codex-acp");
        let agent_preset_select = cx.new(|cx| {
            let opts = daruda_config::ACP_REGISTRY_AGENT_PRESETS
                .iter()
                .map(|preset| {
                    SelectOption::new(preset.id, format!("{} ({})", preset.name, preset.id))
                })
                .collect();
            select::state_with_options(opts, Some(&agent_preset), window, cx)
        });

        let agent_rows = config
            .agents
            .iter()
            .map(|agent| Self::agent_row_from_definition(agent, window, cx))
            .collect::<Vec<_>>();

        let max_fps_str: SharedString = config.render.max_fps.to_string().into();
        let max_fps_select = cx.new(|cx| {
            let opts = daruda_config::ALLOWED_MAX_FPS
                .iter()
                .map(|fps| {
                    SelectOption::new(
                        SharedString::from(fps.to_string()),
                        s::settings_max_fps_option(*fps),
                    )
                })
                .collect();
            select::state_with_options(opts, Some(&max_fps_str), window, cx)
        });

        // Resolve through the palette so a legacy / unknown stored value
        // (e.g. an old "base16-ocean.dark") still shows its effective
        // palette selected instead of a blank dropdown.
        let syntax_theme = SharedString::from(
            crate::ui::theme::SyntaxPalette::from_config_name(&config.file_viewer.syntax_theme)
                .config_name(),
        );
        let syntax_theme_select = cx.new(|cx| {
            let opts = SYNTAX_THEMES
                .iter()
                .map(|v| SelectOption::new(*v, syntax_theme_label(v)))
                .collect();
            select::state_with_options(opts, Some(&syntax_theme), window, cx)
        });

        let font_size_fh = font_size_input.read(cx).focus_handle(cx);
        let vertical_spacing_fh = vertical_spacing_input.read(cx).focus_handle(cx);
        let horizontal_spacing_fh = horizontal_spacing_input.read(cx).focus_handle(cx);
        let opacity_fh = opacity_input.read(cx).focus_handle(cx);
        let scrollback_fh = scrollback_input.read(cx).focus_handle(cx);
        let inset_x_fh = inset_x_input.read(cx).focus_handle(cx);
        let inset_y_fh = inset_y_input.read(cx).focus_handle(cx);
        let clipboard_streaming_fh = clipboard_streaming_input.read(cx).focus_handle(cx);
        let panels_grid_columns_fh = panels_grid_columns_input.read(cx).focus_handle(cx);
        let telegram_token_fh = telegram_token_input.read(cx).focus_handle(cx);

        let mut _input_subscriptions = vec![
            Self::subscribe_input_state(&font_size_input, window, cx),
            Self::subscribe_input_state(&editor_font_size_input, window, cx),
            Self::subscribe_input_state(&agent_chat_font_size_input, window, cx),
            Self::subscribe_input_state(&vertical_spacing_input, window, cx),
            Self::subscribe_input_state(&horizontal_spacing_input, window, cx),
            Self::subscribe_input_state(&opacity_input, window, cx),
            Self::subscribe_input_state(&scrollback_input, window, cx),
            Self::subscribe_input_state(&inset_x_input, window, cx),
            Self::subscribe_input_state(&inset_y_input, window, cx),
            Self::subscribe_input_state(&clipboard_streaming_input, window, cx),
            Self::subscribe_input_state(&panels_grid_columns_input, window, cx),
            Self::subscribe_input_state(&telegram_token_input, window, cx),
            // Theme dropdowns apply live on pick (no Save needed): the
            // commit persists just that one field, and the existing config
            // fan-out repaints every open editor / diff view / pane.
            cx.subscribe_in(
                &syntax_theme_select,
                window,
                |this, state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        this.persist_theme_field(state, |c, v| c.file_viewer.syntax_theme = v, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &terminal_preset_select,
                window,
                |this, state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        this.persist_theme_field(state, |c, v| c.theme.terminal_preset = v, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &ui_preset_select,
                window,
                |this, state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        this.persist_theme_field(state, |c, v| c.theme.ui_preset = v, cx);
                    }
                },
            ),
            // The permission-mode dropdown is persisted on Save (not live), but
            // the explanatory text below it tracks the selection — repaint the
            // window on each pick so `render_agent` shows the matching blurb.
            cx.subscribe_in(
                &default_permission_mode_select,
                window,
                |_this, _state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        cx.notify();
                    }
                },
            ),
        ];
        for row in &agent_rows {
            _input_subscriptions.push(Self::subscribe_input_state(&row.id_input, window, cx));
            _input_subscriptions.push(Self::subscribe_input_state(&row.name_input, window, cx));
            _input_subscriptions.push(Self::subscribe_input_state(&row.command_input, window, cx));
            _input_subscriptions.push(Self::subscribe_input_state(&row.host_input, window, cx));
            _input_subscriptions.push(Self::subscribe_input_state(
                &row.container_input,
                window,
                cx,
            ));
            _input_subscriptions.push(cx.subscribe_in(
                &row.transport_select,
                window,
                |_this, _state, ev: &select::ConfirmEvent, _window, cx| {
                    if matches!(ev, select::SelectEvent::Confirm(_)) {
                        cx.notify();
                    }
                },
            ));
        }

        // Map each section to the focus target it should land on when
        // jumped to from outside the window. Sections without a text
        // input (Cursor / Shell / FileViewer / placeholders / …) fall
        // back to the panel-level focus inside `focus_section`.
        let mut section_focus: HashMap<BuiltinSection, FocusHandle> = HashMap::new();
        section_focus.insert(BuiltinSection::Font, font_size_fh.clone());
        section_focus.insert(BuiltinSection::Window, opacity_fh.clone());
        section_focus.insert(BuiltinSection::Terminal, scrollback_fh.clone());
        section_focus.insert(BuiltinSection::Clipboard, clipboard_streaming_fh.clone());
        section_focus.insert(BuiltinSection::Panels, panels_grid_columns_fh.clone());
        section_focus.insert(BuiltinSection::Notifications, telegram_token_fh.clone());
        if let Some(row) = agent_rows.first() {
            section_focus.insert(
                BuiltinSection::Agent,
                row.id_input.read(cx).focus_handle(cx),
            );
        }

        let _updater_subscription =
            crate::update::Updater::get(cx).map(|e| cx.observe(&e, |_, _, cx| cx.notify()));

        let result = Self {
            panel_focus_handle: cx.focus_handle(),
            base_config: config.clone(),
            active_section: active,
            section_focus,
            language_select,
            terminal_preset_select,
            ui_preset_select,
            font_family_select,
            font_size_input,
            editor_font_size_input,
            agent_chat_font_size_input,
            vertical_spacing_input,
            horizontal_spacing_input,
            cursor_style_select,
            cursor_blinking: config.cursor.blinking,
            default_permission_mode_select,
            agent_preset_select,
            agent_use_modifier_to_send: config.agent.use_modifier_to_send,
            agent_rows,
            max_fps_select,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            opacity_input,
            window_blur: config.window.blur,
            scrollback_input,
            inset_x_input,
            inset_y_input,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            syntax_theme_select,
            clipboard_streaming_input,
            panels_grid_columns_input,
            claude_status_enable: config.claude_status.enable,
            telegram_enabled: config.telegram.enabled,
            telegram_token_input,
            telegram_token_configured,
            telegram_pair_code: None,
            telegram_pair_command_copied: false,
            _telegram_pair_copy_revert_task: None,
            font_size_fh,
            vertical_spacing_fh,
            horizontal_spacing_fh,
            opacity_fh,
            scrollback_fh,
            inset_x_fh,
            inset_y_fh,
            clipboard_streaming_fh,
            panels_grid_columns_fh,
            telegram_token_fh,
            scroll_handle: gpui::ScrollHandle::new(),
            _input_subscriptions,
            error: None,
            plugin_ops_in_flight: std::collections::HashSet::new(),
            plugin_last_error: None,
            plugin_selected: None,
            plugin_view_skill: None,
            _skills_global_subscription: cx
                .observe_global::<crate::agent::skills::SkillsState>(|_, cx| cx.notify()),
            _updater_subscription,
        };
        // The scroll handle is populated during prepaint, which runs after render.
        // Schedule a re-render so the scrollbar thumb appears on first display
        // without requiring an initial scroll event.
        cx.spawn(async move |this, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();

        // Track this window in the WindowRegistry so `open_settings_window`
        // can raise it instead of opening a second copy. The window's root
        // view is `gpui_component::Root` (the SettingsWindow needs Root in
        // the tree so `gpui_component::Input::TextElement::paint` can call
        // `Root::read` without panicking), so the registry stores a typed
        // `SettingsHandle` that bundles the window handle with a
        // `WeakEntity<SettingsWindow>` to recover the inner entity.
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        WindowRegistry::register_settings(window_handle, weak, cx);
        cx.on_release(move |_: &mut SettingsWindow, cx: &mut gpui::App| {
            WindowRegistry::clear_settings(cx);
        })
        .detach();

        result
    }

    /// Switch the active page and (when applicable) land focus on the
    /// section's natural starting field. Called both by sidebar clicks
    /// and by `windows::open_settings_window` when a second open
    /// dispatch arrives while a Settings window is already alive.
    pub fn focus_section(
        &mut self,
        section: BuiltinSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_section != section {
            self.active_section = section;
            // Reset scroll so the new section starts at the top —
            // otherwise switching from a long page leaves the new page
            // mid-scroll which is confusing.
            self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
        }
        if let Some(fh) = self.section_focus.get(&section).cloned() {
            fh.focus(window, cx);
        } else {
            self.panel_focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub fn active_section(&self) -> BuiltinSection {
        self.active_section
    }

    fn dismiss(&mut self, window: &mut Window) {
        window.remove_window();
    }

    fn validate(&self, cx: &gpui::App) -> Result<daruda_config::Config, SharedString> {
        // Start from the snapshot taken at window-open time so fields not
        // exposed in the UI (e.g. [colors], [keybindings]) are preserved.
        let mut config = self.base_config.clone();

        config.general.language = self
            .language_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "auto".to_owned());

        config.theme.terminal_preset = self
            .terminal_preset_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default".to_owned());

        config.theme.ui_preset = self
            .ui_preset_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::ui_theme_presets::DEFAULT.to_owned());

        config.font.family = self
            .font_family_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::FontConfig::default().family);

        let size_str = self.font_size_input.read(cx).value().trim().to_string();
        config.font.size = size_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (6.0..=72.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_font_size()))?;

        let editor_size_str = self
            .editor_font_size_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.font.editor_size = editor_size_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (6.0..=72.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_editor_font_size()))?;

        let agent_chat_size_str = self
            .agent_chat_font_size_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.font.agent_chat_size = agent_chat_size_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (6.0..=72.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_agent_chat_font_size()))?;

        let vs_str = self
            .vertical_spacing_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.font.vertical_spacing = vs_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (0.5..=2.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_spacing()))?;

        let hs_str = self
            .horizontal_spacing_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.font.horizontal_spacing = hs_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (0.5..=2.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_spacing()))?;

        config.cursor.style = match self
            .cursor_style_select
            .read(cx)
            .selected_value()
            .map(|s| s.as_ref())
        {
            Some("underline") => daruda_config::CursorStyle::Underline,
            Some("bar") => daruda_config::CursorStyle::Bar,
            _ => daruda_config::CursorStyle::Block,
        };
        config.cursor.blinking = self.cursor_blinking;

        config.agent.default_permission_mode = self
            .default_permission_mode_select
            .read(cx)
            .selected_value()
            .and_then(|s| daruda_config::DefaultPermissionMode::from_mode_id(s.as_ref()))
            .unwrap_or_default();
        config.agent.use_modifier_to_send = self.agent_use_modifier_to_send;

        let mut agents = Vec::with_capacity(self.agent_rows.len());
        let mut seen_agent_ids = HashSet::new();
        for (index, row) in self.agent_rows.iter().enumerate() {
            let id = row.id_input.read(cx).value().trim().to_string();
            let name = row.name_input.read(cx).value().trim().to_string();
            let command = row.command_input.read(cx).value().trim().to_string();
            if id.is_empty() || name.is_empty() || command.is_empty() {
                return Err(SharedString::from(s::settings_err_agent_catalog_field(
                    index + 1,
                )));
            }
            if !is_valid_agent_id(&id) {
                return Err(SharedString::from(s::settings_err_agent_catalog_id(&id)));
            }
            if !seen_agent_ids.insert(id.clone()) {
                return Err(SharedString::from(s::settings_err_agent_catalog_duplicate(
                    &id,
                )));
            }

            let kind = row
                .transport_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "raw".to_string());
            let host = row.host_input.read(cx).value().trim().to_string();
            let container = row.container_input.read(cx).value().trim().to_string();
            if !agent_row_is_valid(&kind, &host, &container) {
                let msg = if kind == "ssh" {
                    s::settings_err_agent_catalog_host(index + 1)
                } else {
                    s::settings_err_agent_catalog_container(index + 1)
                };
                return Err(SharedString::from(msg));
            }

            let launch = match kind.as_str() {
                "ssh" => daruda_config::AgentLaunch::Ssh {
                    adapter_command: command,
                    host,
                },
                "docker" => daruda_config::AgentLaunch::Docker {
                    adapter_command: command,
                    container,
                },
                _ => daruda_config::AgentLaunch::Raw(command),
            };
            agents.push(daruda_config::AgentDefinition { id, name, launch });
        }
        if agents.is_empty() {
            return Err(SharedString::from(s::settings_err_agent_catalog_empty()));
        }
        config.agents = agents;

        config.render.max_fps = self
            .max_fps_select
            .read(cx)
            .selected_value()
            .and_then(|s| s.as_ref().parse::<u32>().ok())
            .unwrap_or(config.render.max_fps);
        config.render.clamp();

        config.shell.close_pane_on_exit = self.close_pane_on_exit;

        let op_str = self.opacity_input.read(cx).value().trim().to_string();
        config.window.opacity = op_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (0.1..=1.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_opacity()))?;
        config.window.blur = self.window_blur;

        let sb_str = self.scrollback_input.read(cx).value().trim().to_string();
        config.scrollback.max_rows = sb_str
            .parse::<usize>()
            .ok()
            .filter(|&v| (1_000..=500_000).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_scrollback()))?;

        let ix_str = self.inset_x_input.read(cx).value().trim().to_string();
        config.font.inset_x = ix_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (0.0..=32.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_inset()))?;
        let iy_str = self.inset_y_input.read(cx).value().trim().to_string();
        config.font.inset_y = iy_str
            .parse::<f32>()
            .ok()
            .filter(|&v| (0.0..=32.0).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_inset()))?;

        config.left_dock.files_show_hidden = self.files_show_hidden;
        config.left_dock.files_use_gitignore = self.files_use_gitignore;

        config.file_viewer.syntax_theme = self
            .syntax_theme_select
            .read(cx)
            .selected_value()
            .map(|s| s.to_string())
            .unwrap_or_else(|| daruda_config::FileViewerConfig::default().syntax_theme);

        let cb_str = self
            .clipboard_streaming_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.clipboard.streaming_max_bytes = cb_str
            .parse::<usize>()
            .ok()
            .filter(|&v| (4_096..=67_108_864).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_clipboard()))?;

        let pg_str = self
            .panels_grid_columns_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        config.panels.grid_columns = pg_str
            .parse::<u8>()
            .ok()
            .filter(|&v| (1..=16).contains(&v))
            .ok_or_else(|| SharedString::from(s::settings_err_grid_columns()))?;

        config.claude_status.enable = self.claude_status_enable;

        // `authorized_chat_id` is managed outside this form's staged-Save flow
        // (set asynchronously by the Telegram bridge's poll loop on a successful
        // pairing, or cleared immediately by the "Unpair" button) — re-read the
        // live value here instead of trusting `self.base_config`'s window-open
        // snapshot, so clicking Save can never revert a pairing that completed
        // while Settings was open.
        config.telegram.authorized_chat_id = crate::settings_store::SettingsStore::global(cx)
            .user_arc()
            .telegram
            .authorized_chat_id;
        config.telegram.enabled = self.telegram_enabled;

        Ok(config)
    }

    /// Persist a picked theme dropdown value immediately (no Save
    /// required). Writes only the one field `set` touches; `patch_user`'s
    /// fan-out then re-bridges themes and repaints open views, so the
    /// change is visible the moment the dropdown commits. `set` is a
    /// non-capturing fn so it coerces to a plain `fn` pointer.
    fn persist_theme_field(
        &mut self,
        select: &Entity<SelectState>,
        set: fn(&mut daruda_config::Config, String),
        cx: &mut Context<Self>,
    ) {
        let Some(value) = select.read(cx).selected_value().map(|s| s.to_string()) else {
            return;
        };
        use gpui::BorrowAppContext as _;
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.patch_user(|cfg| set(cfg, value.clone()))
        });
        if let Err(msg) = result {
            self.error = Some(SharedString::from(msg));
            cx.notify();
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = match self.validate(cx) {
            Ok(c) => c,
            Err(msg) => {
                self.error = Some(msg);
                cx.notify();
                return;
            }
        };

        // `patch_user` persists the file, updates the Global, and
        // fires every `observe_global` subscriber on return — the
        // app-level theme/keybinding observer and each workspace's
        // own subscription fan the change out, keeping the in-memory
        // store in sync with disk in a single update cycle.
        use gpui::BorrowAppContext as _;
        let result = cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.patch_user(|cfg| *cfg = config.clone())
        });
        if let Err(msg) = result {
            self.error = Some(SharedString::from(msg));
            cx.notify();
            return;
        }

        self.dismiss(window);
    }

    fn focus_next_input(&self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Cycle only through inputs that belong to the *active* section
        // — Tab on the Font page must not jump into the Window page's
        // opacity input while it is hidden.
        let handles: Vec<FocusHandle> = match self.active_section {
            BuiltinSection::Font => vec![
                self.font_size_fh.clone(),
                self.vertical_spacing_fh.clone(),
                self.horizontal_spacing_fh.clone(),
            ],
            BuiltinSection::Window => vec![self.opacity_fh.clone()],
            BuiltinSection::Terminal => {
                vec![
                    self.scrollback_fh.clone(),
                    self.inset_x_fh.clone(),
                    self.inset_y_fh.clone(),
                ]
            }
            BuiltinSection::Clipboard => vec![self.clipboard_streaming_fh.clone()],
            BuiltinSection::Panels => vec![self.panels_grid_columns_fh.clone()],
            BuiltinSection::Notifications => vec![self.telegram_token_fh.clone()],
            BuiltinSection::Agent => self
                .agent_rows
                .iter()
                .flat_map(|row| {
                    [
                        row.id_input.read(cx).focus_handle(cx),
                        row.name_input.read(cx).focus_handle(cx),
                        row.command_input.read(cx).focus_handle(cx),
                    ]
                })
                .collect(),
            BuiltinSection::General
            | BuiltinSection::Cursor
            | BuiltinSection::Shell
            | BuiltinSection::LeftDock
            | BuiltinSection::ClaudeStatus
            | BuiltinSection::Keymap
            | BuiltinSection::Plugin
            | BuiltinSection::About => return,
        };
        let n = handles.len();
        if n == 0 {
            return;
        }
        let current = handles.iter().position(|h| h.is_focused(window));
        let next = if forward {
            current.map_or(0, |i| (i + 1) % n)
        } else {
            match current {
                Some(0) | None => n - 1,
                Some(i) => i - 1,
            }
        };
        handles[next].focus(window, cx);
    }

    pub(super) fn section_label(
        label: impl Into<gpui::SharedString>,
        cx: &gpui::App,
    ) -> impl IntoElement {
        div()
            .text_size(px(theme::LANE_SECTION_HEADER_FONT_SIZE))
            .text_color(theme::current(cx).text_muted)
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(label.into())
    }
}

/// Map config window settings to the GPUI window background appearance.
pub(crate) fn window_background_for(config: &daruda_config::Config) -> WindowBackgroundAppearance {
    if config.window.blur {
        WindowBackgroundAppearance::Blurred
    } else if config.window.opacity < 1.0 {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// Curated syntax palettes. Value = config key resolved by
/// `palette::SyntaxPalette::from_config_name`; labels are i18n'd via
/// [`syntax_theme_label`]. The recommended Daruda palette is first.
const SYNTAX_THEMES: &[&str] = &[
    "daruda",
    "one-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "dracula",
    "github-dark",
    "material-palenight",
    "monokai",
    "nord",
    "gruvbox-dark",
    "solarized-dark",
    "ayu-mirage",
    "night-owl",
    "darcula",
];

/// Localized display label for a syntax-palette config value.
fn syntax_theme_label(value: &str) -> String {
    match value {
        "one-dark" => s::settings_syntax_theme_one_dark(),
        "tokyo-night" => s::settings_syntax_theme_tokyo_night(),
        "catppuccin-mocha" => s::settings_syntax_theme_catppuccin_mocha(),
        "dracula" => s::settings_syntax_theme_dracula(),
        "github-dark" => s::settings_syntax_theme_github_dark(),
        "material-palenight" => s::settings_syntax_theme_material_palenight(),
        "monokai" => s::settings_syntax_theme_monokai(),
        "nord" => s::settings_syntax_theme_nord(),
        "gruvbox-dark" => s::settings_syntax_theme_gruvbox_dark(),
        "solarized-dark" => s::settings_syntax_theme_solarized_dark(),
        "ayu-mirage" => s::settings_syntax_theme_ayu_mirage(),
        "night-owl" => s::settings_syntax_theme_night_owl(),
        "darcula" => s::settings_syntax_theme_darcula(),
        _ => s::settings_syntax_theme_daruda(),
    }
}

fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Whether an agent catalog row's transport-specific fields are filled in.
/// `"ssh"` requires a non-blank `host`; `"docker"` requires a non-blank
/// `container`; any other `kind` (`"raw"`, or an unrecognized/absent select
/// value) has no extra requirement. Pure and GPUI-free so it is directly
/// unit-testable — the save loop in [`SettingsWindow::validate`] is the only
/// caller.
fn agent_row_is_valid(kind: &str, host: &str, container: &str) -> bool {
    match kind {
        "ssh" => !host.trim().is_empty(),
        "docker" => !container.trim().is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod agent_row_validation_tests {
    use super::agent_row_is_valid;

    #[test]
    fn ssh_requires_non_blank_host() {
        assert!(!agent_row_is_valid("ssh", "", ""));
        assert!(!agent_row_is_valid("ssh", "   ", ""));
        assert!(agent_row_is_valid("ssh", "vm-work", ""));
    }

    #[test]
    fn docker_requires_non_blank_container() {
        assert!(!agent_row_is_valid("docker", "", ""));
        assert!(!agent_row_is_valid("docker", "", "   "));
        assert!(agent_row_is_valid("docker", "", "ubuntu-dev"));
    }

    #[test]
    fn raw_is_always_valid_regardless_of_host_or_container() {
        assert!(agent_row_is_valid("raw", "", ""));
        assert!(agent_row_is_valid("raw", "irrelevant", "irrelevant"));
    }

    #[test]
    fn unrecognized_kind_is_treated_as_valid() {
        assert!(agent_row_is_valid("", "", ""));
        assert!(agent_row_is_valid("bogus", "", ""));
    }
}

/// Return all font family names available on this system, sorted alphabetically.
/// The `current` family is always included as the first entry so the select
/// can show the currently-configured font even if it is not yet installed.
fn all_font_names(cx: &gpui::App, current: &str) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    if !current.is_empty() && !names.iter().any(|n| n == current) {
        names.insert(0, current.to_owned());
    }
    names
}

// `render::*` is just the `impl Render for SettingsWindow` block —
// no items are re-exported, but keeping the module declaration above
// is what makes the impl visible to external callers.
