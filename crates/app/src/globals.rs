//! GPUI Global registration at app startup.
//!
//! Order constraints (CLAUDE.md "GPUI shared-state convention"):
//! - `DarudaTheme::init` must precede `apply_daruda_palette` — the
//!   palette reads `cx.global::<DarudaTheme>()` to map slot values
//!   into `gpui_component::Theme`.
//! - `SettingsStore::init` must precede `apply_ui_theme` — the
//!   preset name comes from the store's user layer.
//! - `register_settings_observer` must precede `spawn_file_watch`
//!   so the first watcher fanout never sees a half-initialised
//!   observer chain.
//! - Every `init(cx)` helper is idempotent (`cx.has_global` guard).

use crate::ui;
use crate::window_registry::WindowRegistry;
use gpui::App;

pub(crate) fn init_all(cx: &mut App) {
    // Apply system locale before the settings store loads so that any
    // strings rendered during init use a reasonable language.
    apply_locale_str("auto");
    gpui_component::init(cx);
    ui::theme::DarudaTheme::init(cx);
    ui::theme::apply_daruda_palette(cx);
    crate::settings_store::SettingsStore::init(cx);
    {
        let user = crate::settings_store::SettingsStore::global(cx).user();
        let preset = user.theme.ui_preset.clone();
        let lang = user.general.language.clone();
        let syntax = user.file_viewer.syntax_theme.clone();
        let editor_font_size = user.font.editor_size;
        let agent_chat_font_size = user.font.agent_chat_size;
        let agent_chat_reading_width = user.agent.reading_width;
        let window_opacity = user.window.opacity;
        let term_colors = user.effective_colors();
        let term_bg = term_colors.background;
        let term_fg = term_colors.foreground;
        ui::theme::apply_ui_theme(&preset, cx);
        apply_locale_str(&lang);
        // Seed the selected syntax palette so the editor highlight theme
        // matches the user's choice from the first paint (not the default).
        ui::theme::set_active_syntax_palette(
            cx,
            ui::theme::SyntaxPalette::from_config_name(&syntax),
        );
        // Seed the file-viewer editor font size so raw / diff / markdown
        // rows render at the configured size from the first paint.
        ui::theme::set_editor_font_size(cx, editor_font_size);
        // Seed the agent-chat font size so the conversation pane renders at the
        // configured size from the first paint.
        ui::theme::set_agent_chat_font_size(cx, agent_chat_font_size);
        // Seed the reading-mode content width for AgentChat panes from the
        // first paint.
        ui::theme::set_agent_chat_reading_width(cx, agent_chat_reading_width);
        // Seed the background opacity so the agent-chat pane renders at the
        // configured window opacity from the first paint (mirrors the terminal
        // pane, whose alpha is seeded via its own construction path).
        ui::theme::set_background_alpha(cx, window_opacity);
        // Seed the agent-chat background color from the terminal color theme so
        // the pane matches the terminal (not the UI theme's editor surface)
        // from the first paint.
        ui::theme::set_agent_chat_bg(cx, term_bg.r, term_bg.g, term_bg.b);
        // Seed the agent-chat foreground from the same terminal color theme so
        // the pane's text color matches its terminal-mirrored background from
        // the first paint (counterpart to the background seed above).
        ui::theme::set_agent_chat_fg(cx, term_fg.r, term_fg.g, term_fg.b);
    }
    crate::agent::skills::global::init(cx);
    crate::agent::mcp::global::init(cx);
    crate::agent::tasks_global::init(cx);
    crate::update::init(cx);

    // Kick off a single background update check on startup when enabled.
    // Fired once here (not per-workspace) so multiple windows don't each check.
    if crate::settings_store::SettingsStore::global(cx)
        .user()
        .update
        .auto_check
        && let Some(updater) = crate::update::Updater::get(cx)
    {
        updater.update(cx, |u, cx| u.check(cx));
    }

    register_settings_observer(cx);
    crate::settings_store::spawn_file_watch(cx);
}

/// App-level `cx.observe_global::<SettingsStore>` callback for the
/// side effects that span every open window: locale swap, UI theme swap,
/// keybinding overrides, window background appearance.
///
/// Per-workspace `apply_config` runs through `Workspace`'s own
/// subscription. Keeping the app-wide swap here (not inside
/// `apply_config`) means one config change repaints every window
/// at the same instant.
fn register_settings_observer(cx: &mut App) {
    cx.observe_global::<crate::settings_store::SettingsStore>(|cx| {
        let user = crate::settings_store::SettingsStore::global(cx).user_arc();
        apply_locale_str(&user.general.language);
        // Rebuild the native menu bar after a locale change so menu item
        // labels reflect the new language immediately.
        crate::menus::refresh_recent_menu(cx);
        crate::surface::action_map::apply_keybinding_overrides(&user.keybindings.bindings, cx);
        ui::theme::apply_ui_theme(&user.theme.ui_preset, cx);
        let appearance = crate::settings_window::window_background_for(&user);
        WindowRegistry::for_each_workspace(cx, |_ws, window, _cx| {
            window.set_background_appearance(appearance);
        });
    })
    .detach();
}

/// Resolve and apply the UI locale.
///
/// `"auto"` resolves to the primary tag of the system locale (e.g.
/// `"ko-KR"` → `"ko"`). Unknown locale codes are clamped to `"en"` so
/// neither `rust_i18n` nor `gpui_component` receives an unrecognised tag.
fn apply_locale_str(lang: &str) {
    let candidate = if lang == "auto" {
        let sys = sys_locale::get_locale().unwrap_or_else(|| "en".to_string());
        sys.split('-').next().unwrap_or("en").to_string()
    } else {
        lang.to_string()
    };
    // Clamp to the recognised set (excluding the meta-value "auto").
    let resolved = if daruda_config::SUPPORTED_LOCALES
        .iter()
        .any(|&s| s != "auto" && s == candidate)
    {
        candidate
    } else {
        "en".to_string()
    };
    rust_i18n::set_locale(&resolved);
    gpui_component::set_locale(&resolved);
}
