//! The editor's subtask list — the section frame and one row per subtask,
//! with inline editing.

use daruda_store::tasks::SubTask;
use gpui::{Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use crate::surface::strings;
use crate::ui::checkbox;
use crate::ui::{self, theme};
use crate::workspace::Workspace;

use super::TaskEditorId;
use super::field;
use super::state::TaskEditContent;

pub(super) fn subtasks_section(
    editor_id: TaskEditorId,
    te: &TaskEditContent,
    subtasks: Vec<SubTask>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let done = subtasks.iter().filter(|subtask| subtask.completed).count();
    let title = strings::task::edit_subtasks_progress(done, subtasks.len());
    let mut list = div().flex().flex_col().gap(px(theme::GAP_LG));
    for subtask in subtasks {
        let editing = te.editing_subtask.as_deref() == Some(subtask.id.as_str());
        list = list.child(subtask_row(editor_id, subtask, editing, te, cx));
    }
    list = list.child(
        div()
            .flex()
            .items_center()
            .gap(px(theme::GAP_SM))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(ui::input(&te.new_subtask_input, cx, 8)),
            )
            .child(
                ui::button_icon(
                    ("task-edit-add-subtask", editor_id.0 as usize),
                    ui::icons::ADD,
                    cx,
                )
                .tooltip(strings::task::subtask_add_placeholder())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.submit_new_subtask(editor_id, window, cx)
                })),
            ),
    );
    field(title, list.into_any_element(), cx)
}

fn subtask_row(
    editor_id: TaskEditorId,
    sub: SubTask,
    is_editing: bool,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let sub_id_for_toggle = sub.id.clone();
    // Subtask checkboxes are mouse-targets in a dynamic list — Tab
    // shouldn't traverse N row checkboxes between fields.
    let check = checkbox(
        SharedString::from(format!("subtask-check-{}", sub.id)),
        "",
        (),
    )
    .checked(sub.completed)
    .on_click(cx.listener(move |this, _checked: &bool, _w, cx| {
        this.toggle_editor_subtask(editor_id, &sub_id_for_toggle, cx);
    }));

    let t = theme::current(cx);
    let muted_color = t.text_muted;
    let strong_color = t.text_primary;

    let title_body: gpui::AnyElement = if is_editing {
        div()
            .flex_1()
            .child(crate::ui::input(&te.editing_subtask_input, cx, ()))
            .into_any_element()
    } else {
        let sub_id_for_rename = sub.id.clone();
        let title_text = sub.title.clone();
        div()
            .id(SharedString::from(format!("subtask-title-{}", sub.id)))
            .flex_1()
            .min_w_0()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .text_color(if sub.completed {
                muted_color
            } else {
                strong_color
            })
            .child(SharedString::from(title_text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev, window, cx| {
                    this.handle_task_subtask_mouse_down(
                        editor_id,
                        &sub_id_for_rename,
                        ev,
                        window,
                        cx,
                    )
                }),
            )
            .into_any_element()
    };

    let auto_manual_label = if sub.source_session_id.is_some() {
        strings::task::subtask_auto_label()
    } else {
        strings::task::subtask_manual_label()
    };

    let sub_id_for_remove = sub.id.clone();
    let remove = crate::ui::button_delete_glyph(
        SharedString::from(format!("subtask-remove-{}", sub.id)),
        cx,
    )
    .on_click(cx.listener(move |this, _ev: &gpui::ClickEvent, _w, cx| {
        this.delete_editor_subtask(editor_id, &sub_id_for_remove, cx);
    }));

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(check)
        .child(title_body)
        .child(
            div()
                .flex_none()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(muted_color)
                .child(auto_manual_label),
        )
        .child(remove)
}
