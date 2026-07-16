//! Bottom dock body for the built-in Terminal Input panel.
//!
//! Owns the visual layout and button-click handler; keyboard handling lives in
//! `workspace::mod.rs` on the shared `InputState`. Dropped paths are quoted via
//! [`shell_quote`] using the focused pane's shell flavour before insertion.

use crate::ui::theme;
use gpui::{AnyElement, ClickEvent, Context, ExternalPaths, IntoElement, div, prelude::*, px};

use crate::shell_quote::{format_paths_for_drop, quote_path};
use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::layout::Dock;
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
    // Input edit shortcuts are handled by gpui_component's `"Input"` context.
    // Mid-turn agent panes show Stop; otherwise the button sends input.
    // DESIGN.md: Submit button — height 28px, radius md (6px). The primary / danger
    // factory applies the accent / danger background; height and radius are pinned
    // here to match the spec's fixed heights table (Button: 28px) and radius scale
    // (md: 6px), overriding `Button::small()`'s 24px default.
    let submit = match snap.agent_stop_pane {
        Some(pane_id) => {
            crate::ui::button_danger("send", crate::surface::strings::bottom_input_stop_button())
                .h(px(theme::BUTTON_HEIGHT))
                .rounded(px(theme::RADIUS_MD))
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.cancel_agent_turn(pane_id, cx));
                    }
                }))
        }
        None => {
            crate::ui::button_primary("send", crate::surface::strings::bottom_input_send_button())
                .h(px(theme::BUTTON_HEIGHT))
                .rounded(px(theme::RADIUS_MD))
                .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.send_terminal_input(window, cx));
                    }
                }))
        }
    };
    // When the focused pane is an Agent chat pane, selector chips sit to the
    // left of the Submit button (all in the input's right-hand action column):
    // the mode chip (permission mode), then one chip per Model / ThoughtLevel
    // config option (model, effort). All agent-only: a terminal-pane focus
    // carries `None`, so only Submit shows. Selecting dispatches through
    // `Workspace::set_agent_mode` / `set_agent_config_option` (one-way data flow).
    let mut chips: Vec<AnyElement> = Vec::new();
    if let Some((pane_id, modes)) = &snap.agent_mode {
        chips.push(
            super::super::agent_chat_pane::mode_chip::mode_chip(
                *pane_id,
                modes,
                snap.workspace.clone(),
            )
            .into_any_element(),
        );
    }
    if let Some((pane_id, options)) = &snap.agent_config_options {
        for opt in options {
            chips.push(
                super::super::agent_chat_pane::config_chip::config_chip(
                    *pane_id,
                    opt,
                    snap.workspace.clone(),
                )
                .into_any_element(),
            );
        }
    }
    let action: AnyElement = if chips.is_empty() {
        submit.into_any_element()
    } else {
        let mut row = div()
            .flex()
            .flex_row()
            .items_end()
            .gap(gpui::px(theme::AGENT_CHAT_MSG_GAP));
        for chip in chips {
            row = row.child(chip);
        }
        row.child(submit).into_any_element()
    };
    // The action column sits beside the text in its own column (see
    // `input_with_action_grow`). In auto-grow mode the editor self-sizes
    // to content (the cap is owned by `InputState` — set at construction
    // via `auto_grow(1, max_rows)` and kept in sync on live config reload
    // via `set_auto_grow`); the outer dock height is driven by
    // `adapt_dock_to_input_lines` on every `InputEvent::Change`. In fill
    // mode (fallback) the editor fills the dock's fixed height and scrolls.
    let cell = div()
        .flex_1()
        .flex()
        .child(crate::ui::input_with_action_grow(
            &state,
            action,
            cx,
            0_isize,
            crate::ui::InputGrowMode::AutoGrow,
        ));
    super::bottom_panel_body()
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
