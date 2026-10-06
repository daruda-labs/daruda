//! Fold-mode summary and the shared rule editor bound to this
//! pane's own mode.

use std::rc::Rc;

use gpui::{AnyElement, Context};

use super::axis_chip::axis_chip_label;
use crate::surface::strings as s;
use crate::transcript::editor::fold::{FoldEditorActions, fold_editor, mode_value};
use crate::transcript::editor::state::FoldEditorState;
use crate::transcript::editor::{ResetSpec, SaveSpec};
use crate::transcript::fold_mode::FoldMode;
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// The chip's full text, overridden mark included. Also the fold axis's slot
/// in the compact bar's tooltip, so the two readings of the same setting
/// cannot diverge.
pub(super) fn fold_mode_chip_label(mode: PaneChoice<FoldMode>) -> String {
    axis_chip_label(
        s::agent_chat::fold_mode_chip(mode_value(mode.value())),
        mode.is_following(),
    )
}

/// The footer's source line for one pane axis: following the agent's stated
/// value, or set on this chat alone. Shared by the three axis panels.
pub(super) fn pane_source(following: bool) -> String {
    if following {
        s::agent_chat::source_following_agent()
    } else {
        s::agent_chat::source_this_chat()
    }
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
    let history_rules_view = view.clone();
    let tools_view = view.clone();
    let reset_view = view.clone();
    let save_view = view.clone();
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
            on_history_rules: Rc::new(move |app| {
                if let Some(view) = history_rules_view.upgrade() {
                    view.update(app, |v, cx| v.toggle_fold_editor_history_rules(cx));
                }
            }),
            reset: Some(ResetSpec {
                label: s::agent_chat::use_agent_defaults(),
                source: pane_source(mode_choice.is_following()),
                save: Some(SaveSpec {
                    label: s::agent_chat::save_as_agent_default(),
                    on_save: Rc::new(move |_window, app| {
                        if let Some(view) = save_view.upgrade() {
                            view.update(app, |v, cx| v.save_fold_mode_as_agent_default(cx));
                        }
                    }),
                }),
                // Offered on a value that already equals the default: what the
                // button undoes is the *override*, not the value.
                overridden: !mode_choice.is_following(),
                on_reset: Rc::new(move |window, app| {
                    if let Some(view) = reset_view.upgrade() {
                        view.update(app, |v, cx| v.reset_fold_mode(window, cx));
                    }
                }),
            }),
            on_tools: Rc::new(move |turn, app| {
                if let Some(view) = tools_view.upgrade() {
                    view.update(app, |v, cx| v.toggle_fold_editor_tools(turn, cx));
                }
            }),
        },
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::fold_mode::FoldPreset;

    /// A pane that chose the very value it would follow is still an override:
    /// its footer says so and offers the hand-back.
    #[test]
    fn a_chosen_value_equal_to_the_default_still_reads_as_this_chat() {
        let default = FoldPreset::Auto.mode();
        let seeded = PaneChoice::Seeded(default);
        let chosen = PaneChoice::Chosen(default);
        assert_eq!(seeded.value(), chosen.value());
        assert_eq!(
            pane_source(seeded.is_following()),
            s::agent_chat::source_following_agent()
        );
        assert_eq!(
            pane_source(chosen.is_following()),
            s::agent_chat::source_this_chat()
        );
        assert_ne!(
            s::agent_chat::source_following_agent(),
            s::agent_chat::source_this_chat()
        );
    }
}
