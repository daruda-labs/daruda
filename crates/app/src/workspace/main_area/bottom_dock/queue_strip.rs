//! Bottom-dock queued-prompt strip.
//!
//! When the focused Agent chat pane has prompts buffered behind the running
//! turn (submitted while the agent was busy), this strip renders above the
//! terminal-input panel: a header row with the queued count and a "clear all"
//! button, then one row per queued prompt (text truncated to a single line)
//! with a per-item × remove button.
//!
//! MVU: this is a pure view builder. The buttons dispatch one-line into
//! `Workspace::remove_queued_prompt` / `clear_queued_prompts` (one-way data
//! flow — the queue is single-source on the `AgentChatView`; this strip renders
//! a read-only projection carried by [`BottomDockSnapshot::queued_prompts`]).
//!
//! Rendered only when `snap.queued_prompts` is `Some` (a non-empty queue on the
//! focused agent pane); `None` otherwise — the caller (`render_body`) skips it.

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, SharedString, StatefulInteractiveElement as _,
    div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Sizable as _};
use crate::workspace::layout::{BottomDockSnapshot, Dock};

/// Build the queued-prompt strip, or `None` when the focused pane has no queued
/// prompts (the strip is then not rendered).
pub(in crate::workspace) fn render(
    snap: &BottomDockSnapshot,
    cx: &mut Context<Dock>,
) -> Option<AnyElement> {
    let (pane_id, prompts) = snap.queued_prompts.as_ref()?;
    let pane_id = *pane_id;
    let t = theme::current(cx);
    let border = t.border;
    let bg = theme::agent_chat_tint(cx);
    let header_color = t.text_subtle;
    let item_color = t.text_body;
    let item_bg = t.overlay_hover;
    // The row pulled into the composer for editing gets a distinct fill + a
    // brighter text tone so the edit target reads clearly against its siblings.
    let editing_bg = t.overlay_selected;
    let editing_color = t.text_primary;

    // Header: "N queued" + Resume (only when a Stop parked prompts) + clear-all.
    let clear_all = {
        let workspace = snap.workspace.clone();
        crate::ui::button("agent-queue-clear-all", s::bottom_input_queue_clear_all())
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.clear_queued_prompts(pane_id, window, cx));
                }
            }))
    };
    // A parked queue (kept by a Stop) shows a Resume button that drains it back
    // into the live queue. Absent when nothing is parked (normal live queue).
    let has_paused = prompts.iter().any(|qp| qp.paused);
    let resume = has_paused.then(|| {
        let workspace = snap.workspace.clone();
        crate::ui::button("agent-queue-resume", s::bottom_input_queue_resume())
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.resume_queued_prompts(pane_id, cx));
                }
            }))
    });
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(header_color)
                .child(SharedString::from(s::bottom_input_queued_count(
                    prompts.len(),
                ))),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
                .children(resume)
                .child(clear_all),
        );

    // One row per queued prompt: single-line text + × remove button.
    let mut list = div()
        .id(("agent-queue-list", pane_id as usize))
        .flex()
        .flex_col()
        .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
        .max_h(px(theme::AGENT_QUEUE_STRIP_MAX_H))
        .overflow_y_scroll();
    for qp in prompts {
        let id = qp.id;
        // Text cell — the edit target reads brightest; a parked row is dimmed
        // and marked "Paused"; a live queued row uses the normal body tone. The
        // trailing marker (editing / paused) reuses one slot: editing wins when
        // both apply (the composer is actively holding that row).
        let text_color = if qp.editing {
            editing_color
        } else if qp.paused {
            header_color
        } else {
            item_color
        };
        let marker = if qp.editing {
            Some(s::bottom_input_queue_editing())
        } else if qp.paused {
            Some(s::bottom_input_queue_paused())
        } else {
            None
        };
        let mut text_cell = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_1()
            .min_w_0()
            .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::AGENT_QUEUE_STRIP_FONT_SIZE))
                    .text_color(text_color)
                    .child(SharedString::from(qp.text.clone())),
            );
        if let Some(marker) = marker {
            text_cell = text_cell.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::FONT_SIZE_SM))
                    .text_color(header_color)
                    .child(SharedString::from(marker)),
            );
        }

        // Actions — while editing, a single cancel (↩); otherwise edit (✎) + × .
        let actions = if qp.editing {
            let workspace = snap.workspace.clone();
            let cancel = crate::ui::button_edit_cancel_glyph(
                SharedString::from(format!("agent-queue-edit-cancel-{}", qp.id)),
                cx,
            )
            .tooltip(s::bottom_input_queue_edit_cancel())
            .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.cancel_edit_queued_prompt(pane_id, window, cx)
                    });
                }
            }));
            div().flex().flex_row().items_center().child(cancel)
        } else {
            let edit_ws = snap.workspace.clone();
            let edit = crate::ui::button_edit_glyph(
                SharedString::from(format!("agent-queue-edit-{}", qp.id)),
                cx,
            )
            .tooltip(s::bottom_input_queue_edit())
            .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                if let Some(ws) = edit_ws.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.begin_edit_queued_prompt(pane_id, id, window, cx)
                    });
                }
            }));
            let remove_ws = snap.workspace.clone();
            let remove = crate::ui::button_delete_glyph(
                SharedString::from(format!("agent-queue-remove-{}", qp.id)),
                cx,
            )
            .tooltip(s::bottom_input_queue_remove())
            .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                if let Some(ws) = remove_ws.upgrade() {
                    ws.update(cx, |ws, cx| ws.remove_queued_prompt(pane_id, id, cx));
                }
            }));
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
                .child(edit)
                .child(remove)
        };

        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
            .px(px(theme::AGENT_QUEUE_STRIP_ROW_PAD_X))
            .py(px(theme::AGENT_QUEUE_STRIP_ROW_PAD_Y))
            .rounded(px(theme::AGENT_QUEUE_STRIP_ROW_RADIUS))
            .bg(if qp.editing { editing_bg } else { item_bg })
            .child(text_cell)
            .child(actions);
        list = list.child(row);
    }

    Some(
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
            .px(px(theme::PANEL_BODY_PAD_X))
            .py(px(theme::PANEL_BODY_PAD_Y))
            .border_b_1()
            .border_color(border)
            .bg(bg)
            .child(header)
            .child(list)
            .into_any_element(),
    )
}
