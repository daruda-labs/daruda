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
    gpui_component::init(cx);
    ui::theme::DarudaTheme::init(cx);
    ui::theme::apply_daruda_palette(cx);
    crate::settings_store::SettingsStore::init(cx);
    {
        let preset = crate::settings_store::SettingsStore::global(cx)
            .user()
            .theme
            .ui_preset
            .clone();
        ui::theme::apply_ui_theme(&preset, cx);
    }
    crate::agent::skills::global::init(cx);
    crate::agent::mcp::global::init(cx);
    crate::agent::tasks_global::init(cx);

    register_settings_observer(cx);
    crate::settings_store::spawn_file_watch(cx);
}

/// App-level `cx.observe_global::<SettingsStore>` callback for the
/// side effects that span every open window: UI theme swap,
/// keybinding overrides, window background appearance.
///
/// Per-workspace `apply_config` runs through `Workspace`'s own
/// subscription. Keeping the app-wide swap here (not inside
/// `apply_config`) means one config change repaints every window
/// at the same instant.
fn register_settings_observer(cx: &mut App) {
    cx.observe_global::<crate::settings_store::SettingsStore>(|cx| {
        let user = crate::settings_store::SettingsStore::global(cx).user_arc();
        crate::surface::action_map::apply_keybinding_overrides(&user.keybindings.bindings, cx);
        ui::theme::apply_ui_theme(&user.theme.ui_preset, cx);
        let appearance = crate::settings_window::window_background_for(&user);
        WindowRegistry::for_each_workspace(cx, |_ws, window, _cx| {
            window.set_background_appearance(appearance);
        });
    })
    .detach();
}
