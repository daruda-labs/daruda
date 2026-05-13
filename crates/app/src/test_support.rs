//! Shared test helpers for `daruda` integration / unit tests.
//!
//! Tests that mount a window which renders any `gpui_component` widget must
//! initialise the upstream theme + global state — otherwise rendering panics
//! at `gpui_component::theme::ActiveTheme::theme(cx)` because `Theme` is a
//! global that lives on `App`. Production code calls
//! `gpui_component::init(&mut cx)` once in `main.rs`; tests don't reach that
//! path.
//!
//! Call [`init_gpui_component`] at the top of every `#[gpui::test]` that
//! constructs a `gpui_component::*` widget, modal, or workspace shell.
//! Idempotent — calling more than once is harmless.

#![cfg(test)]

use gpui::TestAppContext;

/// Initialise `gpui_component`'s theme + globals on a `TestAppContext`,
/// then overlay daruda's palette so tests render with the same colors
/// as production. Idempotent — calling more than once is harmless.
pub(crate) fn init_gpui_component(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        // DarudaTheme must be registered before apply_daruda_palette
        // because the palette reads `cx.global::<DarudaTheme>()` to
        // map slot values into `gpui_component::Theme`.
        crate::ui::theme::DarudaTheme::init(cx);
        crate::ui::theme::apply_daruda_palette(cx);
        // Register every app-wide Global the production `main.rs::app.run`
        // would set up — Workspace constructors poke them defensively
        // too, but tests that build only a sub-entity (no Workspace)
        // still need these.
        crate::agent::skills::global::init(cx);
        crate::agent::mcp::global::init(cx);
        crate::agent::tasks_global::init(cx);
    });
}
