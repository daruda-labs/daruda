//! Display-filter chip, persistent popover, and filtered-row placeholder.

use std::rc::Rc;

use gpui::{Anchor, AnyElement, Context, IntoElement, SharedString, prelude::*, px};

use super::options_panel::axis_chip_label;
use crate::surface::strings as s;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::editor::ResetSpec;
use crate::transcript::editor::filter::{FilterEditorActions, filter_editor, filter_value};
use crate::transcript::editor::panel_root;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{Popover, Selectable as _, button_chip_on_surface};
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::FilteredAway;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

pub(super) fn display_filter_chip(
    pane_id: PaneId,
    filter: PaneChoice<DisplayFilter>,
    default_open: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let view = cx.entity().downgrade();
    Popover::new(SharedString::from(format!(
        "agent-chat-display-filter-popover-{pane_id}"
    )))
    .default_open(default_open)
    .anchor(Anchor::TopRight)
    .trigger(
        button_chip_on_surface(
            ("agent-chat-display-filter", pane_id as usize),
            SharedString::from(display_filter_chip_label(filter)),
            surface,
            cx,
        )
        .selected(!filter.is_following())
        .tooltip(SharedString::from(s::agent_chat_filter_tooltip())),
    )
    .content(move |_, window, cx| {
        panel_root(theme::AGENT_CHAT_OPTIONS_PANEL_W, window)
            .child(filter_panel(&view, filter, pane_id, cx))
            .into_any_element()
    })
}

/// The chip's full text, overridden mark included. Also the filter axis's slot
/// in the compact bar's tooltip, so the two readings of the same setting
/// cannot diverge.
pub(super) fn display_filter_chip_label(filter: PaneChoice<DisplayFilter>) -> String {
    axis_chip_label(
        s::agent_chat_filter_chip(&filter_value(filter.value())),
        filter,
    )
}

/// The shared editor, bound to this pane: every click is a one-line dispatch to
/// an `AgentChatView` method, and the footer hands the axis back to the agent's
/// stated value rather than setting one.
pub(super) fn filter_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    choice: PaneChoice<DisplayFilter>,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let toggle_view = view.clone();
    let section_view = view.clone();
    let reset_view = view.clone();
    filter_editor(
        choice.value(),
        &format!("agent-chat-{pane_id}"),
        theme::agent_chat_font_size(cx),
        FilterEditorActions {
            on_toggle: Rc::new(move |facet, app| {
                if let Some(view) = toggle_view.upgrade() {
                    view.update(app, |v, cx| v.toggle_display_facet(facet, cx));
                }
            }),
            on_section: Rc::new(move |parent, on, app| {
                if let Some(view) = section_view.upgrade() {
                    view.update(app, |v, cx| v.set_filter_section(parent, on, cx));
                }
            }),
            reset: Some(ResetSpec {
                // Offered on a value that already equals the default: what the
                // button undoes is the *override*, not the value.
                disabled: choice.is_following(),
                on_reset: Rc::new(move |app| {
                    if let Some(view) = reset_view.upgrade() {
                        view.update(app, |v, cx| v.reset_display_filter(cx));
                    }
                }),
            }),
        },
        cx,
    )
}

/// The reveal control's copy.
///
/// Collapsed, the number states exactly what clicking does — nothing more. A
/// count that also named what a fold held back read as "1 now, 6 on the next
/// click" when the other six were somewhere this control cannot reach.
///
/// Revealed, there is no number at all. Those rows are on screen, so a count
/// restates them, and `Hide 12 filtered rows` is readable as a description of
/// the current state precisely when the state is the opposite.
fn filtered_chip_label(filtered: FilteredAway, revealed: bool) -> String {
    if revealed {
        return s::agent_chat_filtered_hide_again();
    }
    s::agent_chat_filtered_show(filtered.revealable)
}

/// The filter's reveal control, riding the response bar's trailing slot.
///
/// The tally is one per run and the bar is the run's one header, so this is
/// where it belongs. As a row of its own it sat at the group bars' indent and
/// wore their chevron while carrying no count — it read as one more group.
pub(super) fn filtered_chip(
    run_start: usize,
    filtered: FilteredAway,
    revealed: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    button_chip_on_surface(
        ("agent-chat-filtered", run_start),
        SharedString::from(filtered_chip_label(filtered, revealed)),
        surface,
        cx,
    )
    // The row's own gap is tuned for text meeting text; a bordered chip needs
    // its own breathing room or it reads as fused to the count beside it.
    .mr(px(theme::GAP_STANDARD))
    .selected(revealed)
    .on_click(cx.listener(move |this, _ev, window, cx| {
        // The bar itself toggles the response fold, so the chip has to keep its
        // own click from reaching it.
        cx.stop_propagation();
        this.toggle_fold(FoldKey::Filtered(run_start), window, cx);
    }))
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(revealable: usize) -> FilteredAway {
        FilteredAway { revealable }
    }

    /// Unrevealed the chip promises a count; revealed it promises none, and the
    /// revealed string does not vary with either number. That invariance is the
    /// point: a count beside `Hide` reads as "still hidden" exactly when the
    /// rows are on screen.
    #[test]
    fn the_revealed_label_carries_no_count_at_all() {
        let revealed: Vec<String> = [12, 1, 30]
            .into_iter()
            .map(|r| filtered_chip_label(cut(r), true))
            .collect();
        assert!(
            revealed.iter().all(|l| l == &revealed[0]),
            "one string for every count: {revealed:?}"
        );
        assert!(
            !revealed[0].chars().any(char::is_numeric),
            "no digits in the revealed label: {}",
            revealed[0]
        );
    }

    /// The unrevealed half names what clicking reveals, and nothing else. The
    /// number is the reachable count, so the same count reads the same way no
    /// matter how much the filter took from inside a fold.
    #[test]
    fn the_unrevealed_label_names_only_what_clicking_reveals() {
        let one = filtered_chip_label(cut(1), false);
        assert!(one.contains('1'), "{one}");
        let many = filtered_chip_label(cut(12), false);
        assert!(many.contains("12"), "{many}");
        assert_ne!(one, many, "singular and plural differ");
        // No second number to disagree with the first.
        assert_eq!(
            many.matches(char::is_numeric).count(),
            2,
            "only the reachable count appears: {many}"
        );
    }
}
