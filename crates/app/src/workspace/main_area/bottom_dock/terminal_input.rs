//! Bottom dock body for the built-in "Terminal Input" panel.
//!
//! Renders the `crate::ui::input_with_action` wrapper — a chrome cell
//! (`MODAL_INPUT_BG` + border + radius) with the Submit button in its
//! own column beside the text, so the text uses the cell's full height
//! at every dock size. Enter or the button click forwards the current
//! text to the focused terminal pane followed by `\r`, then clears the
//! field. The send key depends on context: a focused agent chat pane
//! sends on Enter (Shift+Enter for a newline) unless the
//! `agent.use_modifier_to_send` config is on, in which case — and for
//! terminal panes — Cmd+Enter sends. While an agent chat pane that
//! advertises ≥2 session modes is focused, Shift+Tab cycles the mode
//! (Claude Code's permission-mode cycle) instead of outdenting. The
//! keyboard path lives in `workspace::mod.rs` via `subscribe_in` on the
//! `InputState`; this file owns the visual layout + the button-click handler.
//!
//! Layout sketch (every dock height). The mode chip only appears when the
//! focused pane is an Agent chat pane that advertises session modes; a
//! terminal-pane focus shows the Submit button alone:
//!
//! ```text
//! ┌──────────────────────────────┐
//! │ text                         │
//! │ ...        [Mode ▾][Submit]  │
//! └──────────────────────────────┘
//! ```
//!
//! Drop handling — dropped paths are quoted with [`shell_quote`] using
//! the focused pane's shell flavour so a path like
//! `/Users/me/My File.txt` reaches the shell as a single token. Two
//! sources are handled:
//!
//! - [`PathDrag`] — internal left-dock (Files / Git Changes) row drag.
//! - [`gpui::ExternalPaths`] — Finder / desktop / other-app file drops.

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
    // Cmd+C / Cmd+X / Cmd+V / Cmd+A on the input are handled by
    // `gpui_component::Input`'s own action handlers (key context
    // `"Input"`). The user must have keyboard focus on the input for
    // the shortcuts to fire — the same constraint every other modal
    // input has.
    // While the focused pane is an Agent chat pane mid-turn, the button
    // reads "Stop" and cancels that pane's turn; otherwise it reads "Send"
    // and forwards the input (to the agent session or the terminal PTY,
    // resolved inside `send_terminal_input`).
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
    // When the focused pane is an Agent chat pane that advertises modes, a
    // mode-selector chip sits to the left of the Submit button (both in the
    // input's right-hand action column). It is agent-only: a terminal-pane
    // focus carries `None`, so only Submit shows. Selecting a mode dispatches
    // through `Workspace::set_agent_mode` (one-way data flow).
    let action: AnyElement = match &snap.agent_mode {
        Some((pane_id, modes)) => {
            let chip = super::super::agent_chat_pane::mode_chip::mode_chip(
                *pane_id,
                modes,
                snap.workspace.clone(),
            );
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(gpui::px(theme::AGENT_CHAT_MSG_GAP))
                .child(chip)
                .child(submit)
                .into_any_element()
        }
        None => submit.into_any_element(),
    };
    // The action column sits beside the text in its own column (see
    // `input_with_action_grow`). In auto-grow mode the editor self-sizes
    // to content (up to `max_rows` configured via `AgentConfig`); the
    // outer dock height is driven by `adapt_dock_to_input_lines` on every
    // `InputEvent::Change`. In fill mode (fallback) the editor fills the
    // dock's fixed height and scrolls internally.
    let max_rows = usize::from(snap.input_max_rows);
    let cell = div()
        .flex_1()
        .flex()
        .child(crate::ui::input_with_action_grow(
            &state,
            action,
            cx,
            0_isize,
            crate::ui::InputGrowMode::AutoGrow { max_rows },
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
