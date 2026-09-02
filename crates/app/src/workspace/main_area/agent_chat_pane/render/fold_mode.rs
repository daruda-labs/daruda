//! Fold-mode chip, and the popover that opens the shared rule editor on this
//! pane's own mode.

use std::rc::Rc;

use gpui::{Anchor, AnyElement, Context, IntoElement, SharedString, prelude::*};

use super::axis_chip::axis_chip_label;
use crate::surface::strings as s;
use crate::transcript::editor::ResetSpec;
use crate::transcript::editor::fold::{FoldEditorActions, fold_editor, mode_value};
use crate::transcript::editor::panel_root;
use crate::transcript::editor::state::FoldEditorState;
use crate::transcript::fold_mode::FoldMode;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{Popover, Selectable as _, button_chip_on_surface};
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Activity-bar chip for the pane's transcript fold rules.
pub(super) fn fold_mode_chip(
    pane_id: PaneId,
    mode: PaneChoice<FoldMode>,
    editor_state: FoldEditorState,
    default_open: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let view = cx.entity().downgrade();
    Popover::new(SharedString::from(format!(
        "agent-chat-fold-mode-popover-{pane_id}"
    )))
    .default_open(default_open)
    .anchor(Anchor::TopRight)
    .trigger(
        button_chip_on_surface(
            ("agent-chat-fold-mode", pane_id as usize),
            SharedString::from(fold_mode_chip_label(mode)),
            surface,
            cx,
        )
        .selected(!mode.is_following())
        .tooltip(SharedString::from(s::agent_chat_fold_mode_tooltip())),
    )
    .content(move |_, window, cx| {
        panel_root(theme::TRANSCRIPT_EDITOR_RULES_PANEL_W, window)
            .child(fold_mode_panel(&view, mode, editor_state, pane_id, cx))
            .into_any_element()
    })
}

/// The chip's full text, overridden mark included. Also the fold axis's slot
/// in the compact bar's tooltip, so the two readings of the same setting
/// cannot diverge.
pub(super) fn fold_mode_chip_label(mode: PaneChoice<FoldMode>) -> String {
    axis_chip_label(
        s::agent_chat_fold_mode_chip(&mode_value(mode.value())),
        mode,
    )
}

/// The shared editor, bound to this pane: every click is a one-line dispatch to
/// an `AgentChatView` method, and the footer hands the axis back to the agent's
/// stated value rather than setting one.
pub(super) fn fold_mode_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    mode_choice: PaneChoice<FoldMode>,
    editor_state: FoldEditorState,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let change_view = view.clone();
    let preset_view = view.clone();
    let turn_view = view.clone();
    let reset_view = view.clone();
    fold_editor(
        mode_choice.value(),
        editor_state,
        &format!("agent-chat-{pane_id}"),
        theme::agent_chat_font_size(cx),
        FoldEditorActions {
            on_change: Rc::new(move |mode, window, app| {
                if let Some(view) = change_view.upgrade() {
                    view.update(app, |v, cx| v.set_fold_mode(mode, window, cx));
                }
            }),
            on_preset: Rc::new(move |preset, window, app| {
                if let Some(view) = preset_view.upgrade() {
                    view.update(app, |v, cx| v.select_fold_preset(preset, window, cx));
                }
            }),
            on_turn: Rc::new(move |turn, app| {
                if let Some(view) = turn_view.upgrade() {
                    view.update(app, |v, cx| v.set_fold_editor_turn(turn, cx));
                }
            }),
            reset: Some(ResetSpec {
                // Offered on a value that already equals the default: what the
                // button undoes is the *override*, not the value.
                disabled: mode_choice.is_following(),
                on_reset: Rc::new(move |window, app| {
                    if let Some(view) = reset_view.upgrade() {
                        view.update(app, |v, cx| v.reset_fold_mode(window, cx));
                    }
                }),
            }),
        },
        cx,
    )
}
