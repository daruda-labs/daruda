//! Bottom dock body for the built-in "Terminal Input" panel.
//!
//! Renders one of two `crate::ui::input_with_action*` wrappers — both
//! paint the same chrome cell (`MODAL_INPUT_BG` + border + radius);
//! only the layout differs. Cmd+Enter or the button click forwards the
//! current text to the focused terminal pane followed by `\r`, then
//! clears the field. The keyboard path lives in `workspace::mod.rs`
//! via `subscribe_in` on the `InputState`; this file owns the visual
//! layout + the button-click handler.
//!
//! The wrapper choice tracks the dock's row-preset bucket
//! (`tab_strip::nearest_row_preset`). 2/3-row presets get a stacked
//! layout (input fills the top, button hugs the bottom-right) so the
//! text region can grow with the dock height; the 1-row preset is too
//! short to fit both stacked, so it switches to a single horizontal
//! row (input ↔ submit) and clips nothing.
//!
//! Layout sketches:
//!
//! ```text
//! 1 row:   ┌──────────────────────────┐
//!          │ text             [Submit]│
//!          └──────────────────────────┘
//!
//! 2/3 row: ┌──────────────────────────┐
//!          │ text                     │
//!          │                          │
//!          │                  [Submit]│
//!          └──────────────────────────┘
//! ```
//!
//! Drop handling — dropped paths are quoted with [`shell_quote`] using
//! the focused pane's shell flavour so a path like
//! `/Users/me/My File.txt` reaches the shell as a single token. Two
//! sources are handled:
//!
//! - [`PathDrag`] — internal sidebar (Files / Git Changes) row drag.
//! - [`gpui::ExternalPaths`] — Finder / desktop / other-app file drops.

use crate::ui::theme;
use gpui::{AnyElement, ClickEvent, Context, ExternalPaths, IntoElement, div, prelude::*, px};

use crate::shell_quote::{format_paths_for_drop, quote_path};
use crate::workspace::main_area::bottom_dock::tab_strip::nearest_row_preset;
use crate::workspace::layout::Dock;
use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::path_drag::PathDrag;

/// Build the terminal input panel body.
pub(in crate::workspace) fn render_body(
    snap: &BottomDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let state = snap.terminal_input.clone();
    let state_for_path = state.clone();
    let state_for_external = state.clone();
    let workspace = snap.workspace.clone();
    let shell = snap.shell;
    // Cmd+C / Cmd+X / Cmd+V / Cmd+A on the input are handled by
    // `gpui_component::Input`'s own action handlers (key context
    // `"Input"`). The user must have keyboard focus on the input for
    // the shortcuts to fire — the same constraint every other modal
    // input has.
    let submit =
        crate::ui::button_primary("send", crate::surface::strings::BOTTOM_INPUT_SEND_BUTTON)
            .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.send_terminal_input(window, cx));
                }
            }));
    // 1-row preset packs everything (tab strip + body padding + chrome)
    // into a height that can't fit a stacked text-area-above / button-
    // below layout without clipping. Swap to a single horizontal row
    // (input ↔ submit) in that case; 2/3-row presets keep the
    // text-area-above / button-below layout that gives the input room
    // to grow.
    let inline_layout = nearest_row_preset(snap.bottom_dock_size) == 1;
    let cell = div().flex_1().flex();
    let cell = if inline_layout {
        cell.child(crate::ui::input_with_action_inline(
            &state, submit, cx, 0_isize,
        ))
    } else {
        cell.min_h(px(theme::INPUT_PANEL_MIN_H))
            .child(crate::ui::input_with_action(&state, submit, cx, 0_isize))
    };
    div()
        .flex_1()
        .flex()
        .px(px(theme::PANEL_BODY_PAD_X))
        .py(px(theme::PANEL_BODY_PAD_Y))
        .drag_over::<PathDrag>(|style, _, _, cx| {
            style.bg(theme::current(cx).input_panel_drop_target_bg)
        })
        .drag_over::<ExternalPaths>(|style, _, _, cx| {
            style.bg(theme::current(cx).input_panel_drop_target_bg)
        })
        .on_drop::<PathDrag>(cx.listener(move |_dock, drag: &PathDrag, window, cx| {
            let quoted = quote_path(&drag.path, shell);
            state_for_path.update(cx, |s, cx_state| s.insert(quoted, window, cx_state));
        }))
        .on_drop::<ExternalPaths>(
            cx.listener(move |_dock, paths: &ExternalPaths, window, cx| {
                if paths.paths().is_empty() {
                    return;
                }
                let formatted = format_paths_for_drop(paths.paths(), shell);
                state_for_external.update(cx, |s, cx_state| s.insert(formatted, window, cx_state));
            }),
        )
        .child(cell)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_exists() {
        // Compile-time check: the module is reachable and imports compile.
    }
}
