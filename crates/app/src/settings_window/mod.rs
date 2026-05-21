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

use std::collections::HashMap;

use crate::ui::theme;
use daruda_config::BuiltinSection;
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, IntoElement, SharedString, Subscription, Window,
    WindowBackgroundAppearance, div, prelude::*, px,
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
    vertical_spacing_input: Entity<InputState>,
    horizontal_spacing_input: Entity<InputState>,
    // Cursor
    cursor_style_select: Entity<SelectState>,
    cursor_blinking: bool,
    // Shell
    close_pane_on_exit: bool,
    // Window
    opacity_input: Entity<InputState>,
    window_blur: bool,
    // Terminal
    scrollback_input: Entity<InputState>,
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
    // Focus handles for Tab cycling (text inputs only)
    font_size_fh: FocusHandle,
    vertical_spacing_fh: FocusHandle,
    horizontal_spacing_fh: FocusHandle,
    opacity_fh: FocusHandle,
    scrollback_fh: FocusHandle,
    clipboard_streaming_fh: FocusHandle,
    panels_grid_columns_fh: FocusHandle,
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

impl SettingsWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_section(BuiltinSection::default(), window, cx)
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

        let syntax_theme = SharedString::from(config.file_viewer.syntax_theme.clone());
        let syntax_theme_select = cx.new(|cx| {
            let opts = SYNTAX_THEMES
                .iter()
                .map(|(v, l)| SelectOption::new(*v, *l))
                .collect();
            select::state_with_options(opts, Some(&syntax_theme), window, cx)
        });

        let font_size_fh = font_size_input.read(cx).focus_handle(cx);
        let vertical_spacing_fh = vertical_spacing_input.read(cx).focus_handle(cx);
        let horizontal_spacing_fh = horizontal_spacing_input.read(cx).focus_handle(cx);
        let opacity_fh = opacity_input.read(cx).focus_handle(cx);
        let scrollback_fh = scrollback_input.read(cx).focus_handle(cx);
        let clipboard_streaming_fh = clipboard_streaming_input.read(cx).focus_handle(cx);
        let panels_grid_columns_fh = panels_grid_columns_input.read(cx).focus_handle(cx);

        let make_sub = |state: &Entity<InputState>, this_cx: &mut Context<Self>| {
            this_cx.subscribe_in(
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
        };

        let _input_subscriptions = vec![
            make_sub(&font_size_input, cx),
            make_sub(&vertical_spacing_input, cx),
            make_sub(&horizontal_spacing_input, cx),
            make_sub(&opacity_input, cx),
            make_sub(&scrollback_input, cx),
            make_sub(&clipboard_streaming_input, cx),
            make_sub(&panels_grid_columns_input, cx),
        ];

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
            vertical_spacing_input,
            horizontal_spacing_input,
            cursor_style_select,
            cursor_blinking: config.cursor.blinking,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            opacity_input,
            window_blur: config.window.blur,
            scrollback_input,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            syntax_theme_select,
            clipboard_streaming_input,
            panels_grid_columns_input,
            claude_status_enable: config.claude_status.enable,
            font_size_fh,
            vertical_spacing_fh,
            horizontal_spacing_fh,
            opacity_fh,
            scrollback_fh,
            clipboard_streaming_fh,
            panels_grid_columns_fh,
            scroll_handle: gpui::ScrollHandle::new(),
            _input_subscriptions,
            error: None,
            plugin_ops_in_flight: std::collections::HashSet::new(),
            plugin_last_error: None,
            plugin_selected: None,
            plugin_view_skill: None,
            _skills_global_subscription: cx
                .observe_global::<crate::agent::skills::SkillsState>(|_, cx| cx.notify()),
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

        Ok(config)
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
        let handles: Vec<&FocusHandle> = match self.active_section {
            BuiltinSection::Font => vec![
                &self.font_size_fh,
                &self.vertical_spacing_fh,
                &self.horizontal_spacing_fh,
            ],
            BuiltinSection::Window => vec![&self.opacity_fh],
            BuiltinSection::Terminal => vec![&self.scrollback_fh],
            BuiltinSection::Clipboard => vec![&self.clipboard_streaming_fh],
            BuiltinSection::Panels => vec![&self.panels_grid_columns_fh],
            BuiltinSection::General
            | BuiltinSection::Cursor
            | BuiltinSection::Shell
            | BuiltinSection::LeftDock
            | BuiltinSection::FileViewer
            | BuiltinSection::ClaudeStatus
            | BuiltinSection::Notifications
            | BuiltinSection::Keymap
            | BuiltinSection::Plugin => return,
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
            .text_color(theme::current(cx).muted_text)
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

/// Syntect bundled theme names (ThemeSet::load_defaults) paired with display labels.
const SYNTAX_THEMES: &[(&str, &str)] = &[
    ("base16-ocean.dark", "Base16 Ocean Dark"),
    ("base16-ocean.light", "Base16 Ocean Light"),
    ("base16-eighties.dark", "Base16 Eighties Dark"),
    ("base16-mocha.dark", "Base16 Mocha Dark"),
    ("InspiredGitHub", "Inspired GitHub"),
    ("Solarized (dark)", "Solarized Dark"),
    ("Solarized (light)", "Solarized Light"),
];

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
