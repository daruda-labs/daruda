//! Bottom dock UI for user-managed panel tabs.
//!
//! Composition mirrors the left dock's dock:
//!   * `tab_strip::render` returns the header (one tab per
//!     `panels.tabs` entry, click switches the active tab).
//!   * `render_body` returns the body — fixed-column grid of widget
//!     renderers (`grid_columns` from config, default 5) plus an `[+]`
//!     button at the end of the sequence. Click handlers on widgets
//!     call back into `Workspace::run_widget`.

pub(in crate::workspace) mod macro_edit_modal;
pub(in crate::workspace) mod macro_key;
pub(in crate::workspace) mod macro_ops;
pub(in crate::workspace) mod queue_strip;
pub(in crate::workspace) mod slash_command;
pub(in crate::workspace) mod tab_strip;
pub(in crate::workspace) mod terminal_input;

use crate::{surface::strings as s, ui::theme};
use daruda_store::panels::{MacroKey, TabId};
use gpui::{AnyElement, ClickEvent, Context, Div, IntoElement, div, prelude::*, px};

use self::macro_edit_modal::MacroEditModal;
use crate::ui::button_add_tile;
use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::layout::Dock;

/// Shared body for both bottom-dock content modes: a `flex_1` vertical
/// flex column with the dock's standard padding, inter-row gap, and
/// overflow clipping. The macro grid fills it with tile rows; the
/// terminal-input mode adds its drag-drop handlers and single input cell
/// on top of the identical container — the column direction and gap are
/// inert for that single child, and the cell sits inside the padding so
/// `overflow_hidden` never clips it. The centered empty-state
/// `placeholder` is deliberately separate.
pub(in crate::workspace) fn bottom_panel_body() -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(theme::PANEL_BODY_GAP))
        .px(px(theme::PANEL_BODY_PAD_X))
        .py(px(theme::PANEL_BODY_PAD_Y))
        .overflow_hidden()
}

/// Build the bottom dock body for the active tab. Active tab's widgets
/// render in a fixed-column grid (`snap.grid_columns`, mirrored from
/// `daruda_config::PanelsConfig::grid_columns`); the `[+]` button is
/// appended at the end of the tile sequence and flows onto the next row
/// when the last data row is full. Per-tab `height` (`Some(px)` = fixed)
/// is enforced via the dock's outer container; auto (`None`) lets the
/// grid (`flex_col` of N-column rows) size the body to fit content.
/// When `snap.terminal_input_visible` is set the built-in Input panel is
/// rendered instead of any macro tab.
pub(in crate::workspace) fn render_body(
    snap: &BottomDockSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    if snap.terminal_input_visible {
        let input = terminal_input::render_body(snap, cx);
        // The queued-prompt strip (if any) sits ABOVE the input panel; both
        // stack in a full-height column so the input keeps its `flex_1` growth
        // and the fixed-height strip rides on top.
        return match queue_strip::render(snap, cx) {
            Some(strip) => div()
                .flex_1()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(strip)
                .child(input)
                .into_any_element(),
            None => input,
        };
    }

    let active_tab_id = snap.active_tab_id.as_ref();
    let has_tabs = !snap.tab_summaries.is_empty();
    let active_widgets = &snap.active_tab_widgets;

    let Some(active_tab_id) = active_tab_id else {
        let message = if !has_tabs {
            s::bottom_dock_no_tabs()
        } else {
            s::bottom_dock_no_active_tab()
        };
        return placeholder(message, cx).into_any_element();
    };
    let tab_id = active_tab_id.clone();

    // Collect renderable tiles in order (MacroKey::Unknown is skipped
    // entirely — no placeholder cell), then append the trailing `[+]`
    // button so it always lands at the end of the tile sequence.
    let mut tiles: Vec<AnyElement> = Vec::new();
    for widget in active_widgets {
        match widget {
            MacroKey::Button(btn) => {
                tiles.push(
                    macro_key::button::render(tab_id.clone(), btn, snap, cx).into_any_element(),
                );
            }
            MacroKey::Unknown(_) => {}
        }
    }
    tiles.push(add_widget_button(tab_id, snap, cx).into_any_element());

    let cols = snap.grid_columns.max(1) as usize;
    let mut body = bottom_panel_body();

    let mut iter = tiles.into_iter();
    loop {
        let row_tiles: Vec<AnyElement> = iter.by_ref().take(cols).collect();
        if row_tiles.is_empty() {
            break;
        }
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::PANEL_BODY_GAP));
        for tile in row_tiles {
            row = row.child(tile);
        }
        body = body.child(row);
    }

    body.into_any_element()
}

/// `[+]` button at the end of the panel body — opens the
/// MacroEditModal in Create mode for the active tab.
fn add_widget_button(
    tab_id: TabId,
    snap: &BottomDockSnapshot,
    cx: &mut Context<Dock>,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    button_add_tile("panel-widget-add", cx).on_click(cx.listener(
        move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                let tab_id = tab_id.clone();
                let ws_for_modal = workspace.clone();
                ws.update(cx, |_, cx| {
                    crate::workspace::dialog_helpers::open_form_modal(
                        s::macro_new_title(),
                        None,
                        move |window, cx| {
                            MacroEditModal::new(
                                ws_for_modal.clone(),
                                tab_id.clone(),
                                None,
                                window,
                                cx,
                            )
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    ))
}

fn placeholder(message: impl Into<gpui::SharedString>, cx: &mut Context<Dock>) -> impl IntoElement {
    let text_color = theme::current(cx).text_subtle;
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(text_color)
        .child(crate::ui::placeholder_text(message))
}
