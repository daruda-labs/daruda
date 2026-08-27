//! Fold-mode chip and menu.

use gpui::{Context, IntoElement, SharedString};

use crate::surface::strings as s;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, Sizable as _, button_on_surface};
use crate::workspace::main_area::agent_chat_pane::fold_mode::{FoldMode, FoldPreset};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Activity-bar chip for the pane's fold mode.
pub(super) fn fold_mode_chip(
    pane_id: PaneId,
    mode: FoldMode,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let label = SharedString::from(s::agent_chat_fold_mode_chip(&mode_value(mode)));
    let view = cx.entity().downgrade();
    button_on_surface(
        ("agent-chat-fold-mode", pane_id as usize),
        label,
        surface,
        cx,
    )
    .xsmall()
    .tooltip(SharedString::from(s::agent_chat_fold_mode_tooltip()))
    .dropdown_menu(move |menu, _window, _cx| build_fold_mode_menu(&view, mode, menu))
}

fn mode_value(mode: FoldMode) -> String {
    match mode.preset() {
        Some(preset) => preset_label(preset),
        None => s::agent_chat_fold_mode_custom(),
    }
}

fn build_fold_mode_menu(
    view: &gpui::WeakEntity<AgentChatView>,
    current: FoldMode,
    menu: PopupMenu,
) -> PopupMenu {
    FoldPreset::ALL.into_iter().fold(menu, |menu, preset| {
        let view = view.clone();
        menu.item(
            PopupMenuItem::new(SharedString::from(preset_label(preset)))
                .checked(current.preset() == Some(preset))
                .on_click(move |_, _window, app| {
                    if let Some(view) = view.upgrade() {
                        view.update(app, |v, cx| v.set_fold_mode(preset.mode(), cx));
                    }
                }),
        )
    })
}

fn preset_label(preset: FoldPreset) -> String {
    match preset {
        FoldPreset::Auto => s::agent_chat_fold_mode_auto(),
        FoldPreset::Summary => s::agent_chat_fold_mode_summary(),
        FoldPreset::Expanded => s::agent_chat_fold_mode_expanded(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_label() {
        for preset in FoldPreset::ALL {
            assert!(!preset_label(preset).is_empty(), "{preset:?}");
        }
        assert!(!s::agent_chat_fold_mode_custom().is_empty());
    }

    #[test]
    fn the_chip_names_the_preset_and_falls_back_to_custom() {
        assert_eq!(
            mode_value(FoldMode::default()),
            preset_label(FoldPreset::Auto)
        );
        assert_eq!(
            mode_value(FoldPreset::Summary.mode()),
            preset_label(FoldPreset::Summary)
        );
        let custom = FoldMode::from_tokens(["auto", "last.tool=expanded"]);
        assert_eq!(mode_value(custom), s::agent_chat_fold_mode_custom());
    }
}
