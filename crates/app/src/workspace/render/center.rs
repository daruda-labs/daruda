//! Main-area center content: the element shown between the tab bar and
//! the bottom dock. Either the active tab's pane layout (via
//! `render_layout`), the inaccessible-lane empty state, or a blank
//! fallback. Extracted from the `Workspace::render` body so that large
//! method stays focused on top-level layout assembly.
//!
//! Named `center` (not `main_area`) to avoid confusion with the
//! sibling `crate::workspace::main_area` runtime module.

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

use crate::lane::availability::LaneAvailability;
use crate::ui::theme;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneLayout;
use crate::workspace::main_area::render_layout;

/// Build the center content element for the active lane.
///
/// The walker (`render_layout`) dispatches per-pane on `PaneContent`:
/// terminal panes embed `TerminalView`, file panes embed the file
/// viewer driven by their own per-pane scroll handle and find input.
/// When a pane is zoomed, only that leaf renders at full size with
/// `has_splits = true` so the pane header (and its right-click Unzoom
/// menu) stays available. Inactive-pane dim is driven separately by
/// `refresh_pane_dimming`, which skips dimming while a pane is zoomed.
///
/// Reads only `&Workspace` (the `cx` is for the embedded views'
/// listeners), so it stays render-pure.
pub(in crate::workspace) fn render_center_content(
    ws: &Workspace,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    // Availability gate runs FIRST, before any tab lookup. An
    // inaccessible active lane must show the empty-state even when
    // `tabs` is non-empty — a lane whose runtime entry still holds tabs
    // from when it was last `Present`, or a mid-session vanish that
    // never cleared tabs, would otherwise render a stale terminal
    // against a dead cwd. `availability` is precomputed (recompute on
    // restore / activate / load-failure); reading it here keeps
    // `render()` pure.
    if let Some(availability) = ws
        .active_lane()
        .map(|l| l.availability)
        .filter(|a| *a != LaneAvailability::Present)
    {
        inaccessible_empty_state(availability, cx)
    } else if let Some(tab) = ws
        .active_runtime()
        .tabs
        .get(ws.active_runtime().active_tab_index)
    {
        let actual_has_splits = tab.layout.leaf_count() > 1;
        if let Some(zoomed_id) = ws.main_area.zoomed_pane_id {
            if tab.layout.pane_ids().contains(&zoomed_id) {
                let leaf = PaneLayout::Pane(zoomed_id);
                render_layout(
                    &leaf,
                    &ws.active_runtime().panes,
                    zoomed_id,
                    true,
                    SharedString::from(ws.font_family.clone()),
                    None,
                    ws.main_area.pane_drop_hover,
                    cx,
                )
            } else {
                render_layout(
                    &tab.layout,
                    &ws.active_runtime().panes,
                    ws.active_runtime().focused_pane_id,
                    actual_has_splits,
                    SharedString::from(ws.font_family.clone()),
                    ws.main_area.zoomed_pane_id,
                    ws.main_area.pane_drop_hover,
                    cx,
                )
            }
        } else {
            render_layout(
                &tab.layout,
                &ws.active_runtime().panes,
                ws.active_runtime().focused_pane_id,
                actual_has_splits,
                SharedString::from(ws.font_family.clone()),
                None,
                ws.main_area.pane_drop_hover,
                cx,
            )
        }
    } else {
        // No tab for the active lane, and the lane is `Present` (the
        // inaccessible case is handled by the availability gate above).
        // The legitimate "Present lane with no tabs" case is healed by
        // the restore fallback / `add_tab`; a truly empty workspace (no
        // projects, no active lane) also lands here. Either way, fall
        // through to a blank element.
        div().flex_1().w_full().into_any_element()
    }
}

/// Centered main-area placeholder shown when the active lane's root
/// directory is inaccessible (Missing / AccessDenied). Fills the center
/// with the state message and a Remove affordance; the button is a
/// one-line dispatch into `request_remove_inaccessible_active` (one-way
/// data flow).
fn inaccessible_empty_state(
    availability: LaneAvailability,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    use crate::surface::strings as s;
    use crate::ui::{Icon, IconName, Sizable as _, button_danger};

    let t = theme::current(cx);
    let (icon, title, body) = match availability {
        // The caller filters `Present` before reaching here. Render
        // nothing rather than panic if that invariant ever slips — an
        // explicit arm keeps the match exhaustive so a future variant
        // is a compile error, not a silent fall-through.
        LaneAvailability::Present => return div().into_any_element(),
        LaneAvailability::Missing => (
            IconName::TriangleAlert,
            s::projects_empty_missing_title(),
            s::projects_empty_missing_body(),
        ),
        LaneAvailability::AccessDenied => (
            IconName::EyeOff,
            s::projects_empty_denied_title(),
            s::projects_empty_denied_body(),
        ),
    };

    div()
        .flex_1()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::MAIN_EMPTY_STATE_GAP))
        .bg(t.file_viewer_bg)
        .child(
            Icon::new(icon)
                .with_size(px(theme::MAIN_EMPTY_STATE_ICON_SIZE))
                .text_color(theme::WARNING),
        )
        .child(
            div()
                .text_size(px(theme::MAIN_EMPTY_STATE_TITLE_FONT_SIZE))
                .text_color(t.text_primary)
                .child(SharedString::from(title)),
        )
        .child(
            div()
                .max_w(px(theme::MAIN_EMPTY_STATE_BODY_MAX_W))
                .text_size(px(theme::MAIN_EMPTY_STATE_BODY_FONT_SIZE))
                .text_color(t.text_muted)
                .text_center()
                .child(SharedString::from(body)),
        )
        .child(
            button_danger("inaccessible-remove", s::ctx_remove()).on_click(cx.listener(
                |this, _, window, cx| {
                    this.request_remove_inaccessible_active(window, cx);
                },
            )),
        )
        .into_any_element()
}
