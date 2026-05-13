//! Renderer for the TaskEdit pane content variant (R-19c).
//!
//! Builds the form body for `PaneContent::TaskEdit`: title input,
//! branch input (with auto-derive + git ref validation), markdown
//! prompt editor, subtask placeholder, notes editor, auto-execute
//! checkbox, and the trailing [Discard] / [Save Draft] / [Start]
//! footer.
//!
//! The form is rendered from a `&TaskEditContent` snapshot held on
//! `Pane`; mutations (save, branch override, etc.) flow back through
//! `Workspace::*` methods so this module stays free of business
//! logic.

use crate::ui::theme;
use daruda_store::tasks::SubTask;
use gpui::{Context, IntoElement, KeyDownEvent, MouseButton, SharedString, div, prelude::*, px};

use super::super::Workspace;
use super::super::layout::PaneId;
use super::super::pane::{BranchValidation, TaskEditContent};
use crate::surface::strings;
use crate::ui::select::select;
use crate::ui::{
    ButtonVariants as _, Disableable as _, ScrollableElement as _, Sizable as _, button,
    button_close, button_danger, button_primary, checkbox, markdown_editor,
};

/// Reserved footer height (px). Used to position the absolute scroll
/// area's `bottom` offset and the absolute footer bar's `h` — the
/// `flex_1 + overflow_y_scroll` combination silently collapses
/// `scroll_max` to 0 in GPUI/Taffy, so we use the absolute-positioning
/// pattern from `render_file_viewer/mod.rs` instead. Value covers an
/// `xsmall` Button row (~28px) plus the surrounding `MODAL_PANEL_GAP`
/// vertical padding (~10px top + 10px bottom).
const TASK_EDIT_FOOTER_H_PX: f32 = 48.0;

/// Right-side padding reserved for the overlay vertical scrollbar so
/// text inputs and other content don't sit underneath the thumb. The
/// gpui-component thumb is 6 / 8px wide depending on hover state — 14px
/// covers the active width plus a small visual gap.
const TASK_EDIT_SCROLLBAR_GUTTER_PX: f32 = 14.0;

pub(in crate::workspace) fn render(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let theme_t = theme::current(cx);
    let pane_bg = theme_t.terminal_bg;
    let pane_text = theme_t.modal_text_primary;
    let label_color = theme_t.muted_text;

    let title_block = labeled_field(
        "Title",
        div()
            .w_full()
            .child(crate::ui::input(&te.title_input, cx, 0))
            .into_any_element(),
        label_color,
    );

    let branch_block = labeled_field("Branch", branch_field(te, cx), label_color);

    let base_block = labeled_field(
        strings::TASK_EDIT_BASE_LABEL,
        div()
            .w_full()
            .child(select(&te.base_select, cx, 3).placeholder(strings::TASK_EDIT_BASE_ACTIVE_LABEL))
            .into_any_element(),
        label_color,
    );

    let subtasks_block = subtasks_section(pane_id, te, cx);

    let editor_column = editor_column(pane_id, te, cx);

    let auto_execute = te.auto_execute;
    let auto_row = checkbox_row(
        checkbox("task-edit-auto-execute", "Auto-execute", 0)
            .checked(auto_execute)
            .on_click(cx.listener(move |this, checked: &bool, _w, cx| {
                if let Some(te) = this.task_edit_content_mut_for_pane(pane_id) {
                    te.auto_execute = *checked;
                }
                cx.notify();
            })),
    );

    let can_save = !matches!(te.branch_validation, BranchValidation::Invalid { .. });

    // Action-bar footer overrides the wrapper-default `xsmall()` to
    // `small()` per CLAUDE.md §10 (size override justified inline):
    // these are top-level "what now?" buttons that benefit from a
    // larger click target. Variants are picked so the three actions
    // are visually distinguishable at a glance:
    //   Discard — danger (red), destructive.
    //   Save Draft — primary (blue), the emphasized save target.
    //   Start — success (green), the "go" action; green also reads
    //   as run/launch and stops the two CTAs from being the same
    //   blue tone side-by-side.
    let footer = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(px(theme::MODAL_FOOTER_GAP))
        .child(
            button_danger("task-edit-discard", "Discard")
                .small()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.discard_task_edit_pane(pane_id, window, cx);
                })),
        )
        .child(
            button_primary("task-edit-save-draft", "Save Draft")
                .small()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_task_edit_pane(pane_id, false, window, cx);
                })),
        )
        .child(
            button("task-edit-start", "Start")
                .small()
                .success()
                .disabled(!can_save)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_task_edit_pane(pane_id, true, window, cx);
                })),
        );

    // Absolute-positioned layers — the only reliable way to give an
    // `overflow_y_scroll` container a definite bounded height in
    // GPUI/Taffy. The same pattern lives in `render_file_viewer/mod.rs`
    // and is documented there: `flex_1 + overflow_y_scroll` silently
    // collapses `scroll_max` to 0, so the pane never actually scrolls
    // even when its content overflows.
    //
    // Layout:
    // - scroll_area: top 0, bottom TASK_EDIT_FOOTER_H — form content
    // - footer_bar: bottom 0, h TASK_EDIT_FOOTER_H — action buttons,
    //   pinned regardless of scroll position
    let scroll_handle = te.body_scroll_handle.clone();
    let scroll_area = div()
        .id(("task-edit-body", pane_id as usize))
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom(px(TASK_EDIT_FOOTER_H_PX))
        .overflow_y_scroll()
        .track_scroll(&scroll_handle)
        .pt(px(theme::RIGHT_PANEL_PAD_X))
        .pb(px(theme::RIGHT_PANEL_PAD_X))
        // Symmetric horizontal padding — even though only the right
        // edge actually overlaps with the overlay scrollbar, matching
        // the left side keeps the form visually centered when the
        // scrollbar is hidden (short content) or fading out.
        .pl(px(TASK_EDIT_SCROLLBAR_GUTTER_PX + theme::RIGHT_PANEL_PAD_X))
        .pr(px(TASK_EDIT_SCROLLBAR_GUTTER_PX + theme::RIGHT_PANEL_PAD_X))
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(title_block)
        .child(branch_block)
        .child(base_block)
        .child(auto_row)
        .child(subtasks_block)
        .child(editor_column);

    let footer_bar = div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(TASK_EDIT_FOOTER_H_PX))
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::MODAL_PANEL_GAP))
        .child(footer);

    div()
        .key_context("TaskEditPane")
        // `.tab_group()` keeps Tab cycling inside this pane's
        // labelled inputs (title=0, branch=1, prompt=2, base=3,
        // notes=4, new_subtask=5). `editing_subtask_input` is a
        // dynamic-list row outside the main cycle.
        .tab_group()
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _window, cx| {
            // Escape while an inline subtask rename is in progress
            // cancels it. `gpui_component::Input` doesn't expose
            // Escape via `InputEvent`, so the per-pane handler
            // intercepts it here. Outside rename mode we let the
            // key fall through to the workspace's global handler.
            if ev.keystroke.key.as_str() == "escape" {
                let in_rename = this
                    .task_edit_content_for_pane(pane_id)
                    .is_some_and(|te| te.editing_subtask.is_some());
                if in_rename {
                    this.cancel_rename_subtask(pane_id, cx);
                    cx.stop_propagation();
                }
            }
        }))
        .relative()
        .size_full()
        .bg(pane_bg)
        .text_color(pane_text)
        .child(scroll_area)
        .child(footer_bar)
        .vertical_scrollbar(&scroll_handle)
}

/// Vertical stack containing the prompt editor on top and the notes
/// editor on the bottom. Both use their `InputState::rows` baseline
/// for natural height (prompt = 20 rows, notes = 4 rows) and let the
/// surrounding `overflow_y_scroll` body handle any window-too-short
/// overflow. The earlier draggable divider was dropped because fixed
/// sizes plus the outer scroll cover the same use case without the
/// event-routing complexity.
fn editor_column(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> gpui::AnyElement {
    let prompt_pane = div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(prompt_header(pane_id, te, cx))
        .child(markdown_editor(&te.prompt_state, cx).tab_index(2));
    let notes_pane = div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(notes_header(cx))
        .child(markdown_editor(&te.notes_state, cx).tab_index(4));

    div()
        .flex()
        .flex_col()
        .w_full()
        .flex_none()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(prompt_pane)
        .child(notes_pane)
        .into_any_element()
}

/// SubtaskListView (R-21). Shows the section title with the current
/// `(done/total done)` progress counter, then one row per subtask
/// (checkbox + title + `auto`/`manual` label + `[×]`), then the
/// inline `[+ Add subtask…]` input. Draft panes (no `task_id`) get a
/// muted hint in place of the list — subtasks attach to persisted
/// tasks only.
fn subtasks_section(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> gpui::AnyElement {
    let Some(task_id) = te.task_id.clone() else {
        return draft_subtasks_hint(te, cx);
    };

    let (subtasks, done, total) = match cx
        .global::<crate::agent::tasks_global::GlobalTasks>()
        .get(&task_id)
    {
        Some(task) => {
            let (d, t) = task.subtask_progress();
            (task.subtasks.clone(), d, t)
        }
        None => (Vec::new(), 0, 0),
    };

    let header = SharedString::from(format!(
        "{} ({}/{}{})",
        strings::TASK_SUBTASK_SECTION_TITLE,
        done,
        total,
        strings::TASK_SUBTASK_PROGRESS_SUFFIX,
    ));

    let editing_id = te.editing_subtask.clone();
    let mut list = div().flex().flex_col().gap(px(theme::MODAL_PANEL_GAP));
    for sub in subtasks.into_iter() {
        let is_editing = editing_id.as_deref() == Some(sub.id.as_str());
        list = list.child(subtask_row(
            pane_id,
            task_id.clone(),
            sub,
            is_editing,
            te,
            cx,
        ));
    }
    list = list.child(add_subtask_row(te, cx));

    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(field_label_shared(header, cx))
        .child(list)
        .into_any_element()
}

fn draft_subtasks_hint(_te: &TaskEditContent, cx: &gpui::App) -> gpui::AnyElement {
    let muted = theme::current(cx).muted_text;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(field_label(strings::TASK_SUBTASK_SECTION_TITLE, cx))
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(muted)
                .child(strings::TASK_SUBTASK_DRAFT_HINT),
        )
        .into_any_element()
}

fn subtask_row(
    pane_id: PaneId,
    task_id: String,
    sub: SubTask,
    is_editing: bool,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let sub_id_for_toggle = sub.id.clone();
    let task_id_for_toggle = task_id.clone();
    // Subtask checkboxes are mouse-targets in a dynamic list — Tab
    // shouldn't traverse N row checkboxes between fields.
    let check = checkbox(
        SharedString::from(format!("subtask-check-{}", sub.id)),
        "",
        (),
    )
    .checked(sub.completed)
    .on_click(cx.listener(move |this, _checked: &bool, _w, cx| {
        this.toggle_subtask(&task_id_for_toggle, &sub_id_for_toggle, cx);
    }));

    let t = theme::current(cx);
    let muted_color = t.muted_text;
    let strong_color = t.modal_text_primary;

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
            .text_color(if sub.completed {
                muted_color
            } else {
                strong_color
            })
            .child(SharedString::from(title_text))
            .on_mouse_down(MouseButton::Left, {
                let sub_id = sub_id_for_rename.clone();
                cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
                    if ev.click_count >= 2 {
                        this.enter_rename_subtask(pane_id, sub_id.clone(), window, cx);
                        cx.stop_propagation();
                    }
                })
            })
            .into_any_element()
    };

    let auto_manual_label = if sub.source_session_id.is_some() {
        strings::TASK_SUBTASK_AUTO_LABEL
    } else {
        strings::TASK_SUBTASK_MANUAL_LABEL
    };

    let sub_id_for_remove = sub.id.clone();
    let task_id_for_remove = task_id.clone();
    let remove = button_close(
        SharedString::from(format!("subtask-remove-{}", sub.id)),
        SharedString::from(format!("subtask-row-{}", sub.id)),
        cx,
    )
    .on_click(cx.listener(move |this, _ev: &gpui::ClickEvent, _w, cx| {
        this.delete_subtask(&task_id_for_remove, &sub_id_for_remove, cx);
    }));

    div()
        .group(SharedString::from(format!("subtask-row-{}", sub.id)))
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

fn add_subtask_row(te: &TaskEditContent, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .child(
            div()
                .w_full()
                .child(crate::ui::input(&te.new_subtask_input, cx, 5)),
        )
}

fn field_label_shared(text: SharedString, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).muted_text)
        .child(text)
}

fn labeled_field(
    label: &'static str,
    body: gpui::AnyElement,
    label_color: gpui::Hsla,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(field_label_with_color(label, label_color))
        .child(body)
}

fn field_label(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    field_label_with_color(text, theme::current(cx).muted_text)
}

fn field_label_with_color(text: &'static str, color: gpui::Hsla) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(color)
        .child(text)
}

/// Smaller, dimmer caption rendered inline with a `field_label` —
/// used for contractual notes the user benefits from seeing inline
/// (e.g. "Not included in the agent prompt." beside the Notes
/// section). One size step below `field_label` so it reads as a
/// subtitle rather than a peer.
fn field_hint(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).muted_text)
        .child(text)
}

/// Header row above the Notes markdown editor: "Notes" label on the
/// left, the agent-prompt-exclusion hint inline to its right. Mirrors
/// `prompt_header` shape so both editor sections present their
/// section title + auxiliary metadata on a single row.
fn notes_header(cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(field_label("Notes", cx))
        .child(field_hint(strings::TASK_EDIT_NOTES_HINT, cx))
}

/// Header row above the Prompt markdown editor: "Prompt" label on the
/// left, `[📄 Open file]` button on the right (R-20 follow-up). The
/// button is disabled for drafts and Backlog tasks since neither has
/// the on-disk `<wt>/.daruda/task-<branch>.md` file yet — running
/// `[Start]` materialises both the worktree and the file in one step.
fn prompt_header(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let has_prompt_file = te.task_id.as_deref().is_some_and(|id| {
        cx.global::<crate::agent::tasks_global::GlobalTasks>()
            .get(id)
            .and_then(|t| {
                let wt = t.state.worktree_path()?;
                let path = wt
                    .join(".daruda")
                    .join(format!("task-{}.md", t.branch_name));
                path.exists().then_some(())
            })
            .is_some()
    });
    let open_btn = button(
        ("task-edit-open-file", pane_id as usize),
        strings::TASK_EDIT_OPEN_FILE_BUTTON,
    )
    .disabled(!has_prompt_file)
    .on_click(cx.listener(move |this, _, window, cx| {
        let Some(id) = this
            .task_edit_content_for_pane(pane_id)
            .and_then(|te| te.task_id.clone())
        else {
            return;
        };
        this.open_task_prompt_file(&id, window, cx);
    }));
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(field_label("Prompt", cx))
        .child(open_btn)
}

fn branch_field(te: &TaskEditContent, cx: &gpui::App) -> gpui::AnyElement {
    // Wrap the input in a frame so the red error border can ride on
    // the outer div without `gpui_component::Input` needing an "error"
    // builder (which it doesn't expose). The wrapper is invisible
    // when validation is `Valid` or `Empty`, matching the
    // pre-validation baseline.
    let invalid = te.branch_validation.is_invalid();
    let invalid_border = theme::current(cx).task_edit_branch_invalid_border;
    let input_frame = div()
        .rounded(px(theme::TASK_EDIT_BRANCH_INVALID_RADIUS))
        .when(invalid, |f| {
            f.border(px(theme::TASK_EDIT_BRANCH_INVALID_BORDER_W))
                .border_color(invalid_border)
        })
        .child(crate::ui::input(&te.branch_input, cx, 1));

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(input_frame);
    if let BranchValidation::Invalid { reason } = &te.branch_validation {
        col = col.child(branch_error(reason.clone(), cx));
    }
    col.into_any_element()
}

fn branch_error(reason: SharedString, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).right_panel_task_error_color)
        .child(reason)
}

fn checkbox_row(widget: impl IntoElement) -> impl IntoElement {
    div().flex().flex_row().items_center().child(widget)
}
