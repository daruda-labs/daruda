//! Renderer for the TaskEdit pane content variant (R-19c).
//!
//! Builds the form body for `PaneContent::TaskEditPane`: title input,
//! branch input (with auto-derive + git ref validation), markdown
//! prompt editor, subtask placeholder, notes editor, auto-execute
//! checkbox, and the trailing [Discard] / [Save Draft] / [Start]
//! footer.
//!
//! The form is rendered from a `&TaskEditContent` snapshot held on
//! `Pane`; mutations (save, branch override, etc.) flow back through
//! `Workspace::*` methods so this module stays free of business
//! logic.

pub(in crate::workspace) mod task_edit_ops;

use crate::ui::theme;
use daruda_store::tasks::SubTask;
use gpui::{Context, IntoElement, KeyDownEvent, MouseButton, SharedString, div, prelude::*, px};

use super::super::Workspace;
use super::pane::{BranchValidation, TaskEditContent};
use super::pane_tree::PaneId;
use crate::surface::strings;
use crate::ui::select::select;
use crate::ui::{
    ButtonVariants as _, Disableable as _, Sizable as _, button, button_close, button_primary,
    checkbox, markdown_editor, radio,
};
use daruda_store::tasks::TaskAgentSurface;

/// Reserved footer height (px). Used to position the absolute scroll
/// area's `bottom` offset and the absolute footer bar's `h` — the
/// `flex_1 + overflow_y_scroll` combination silently collapses
/// `scroll_max` to 0 in GPUI/Taffy, so we use the absolute-positioning
/// pattern from `render_file_viewer/mod.rs` instead. Value covers an
/// `xsmall` Button row (~28px) plus the surrounding `MODAL_PANEL_GAP`
/// vertical padding (~10px top + 10px bottom).
const TASK_EDIT_FOOTER_H_PX: f32 = 48.0;

pub(in crate::workspace) fn render(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let theme_t = theme::current(cx);
    let pane_bg = theme_t.task_edit_bg;
    let scrollbar_thumb = theme_t.scrollbar_thumb;
    let scrollbar_thumb_hover = theme_t.task_edit_scrollbar_thumb_hover;
    let pane_text = theme::current(cx).text_primary;
    let label_color = theme_t.text_muted;

    // Title field doubles as the pane's discard affordance: the
    // destructive close sits at the far right of the "Title" label row
    // (a `×`, like a tab/pane close) rather than as a footer button, so
    // the footer carries only the two save actions and the close reads
    // as "dismiss this editor". `button_close` is always visible (not
    // hover-gated) and stays out of the Tab cycle.
    let title_block = div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(field_label_with_color(
                    strings::task_edit_title_label(),
                    label_color,
                ))
                .child(
                    button_close(("task-edit-discard", pane_id as usize), cx).on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.discard_task_edit_pane(pane_id, window, cx);
                        }),
                    ),
                ),
        )
        .child(
            div()
                .w_full()
                .child(crate::ui::input(&te.title_input, cx, 0)),
        );

    let branch_block = labeled_field(
        strings::task_edit_branch_label(),
        branch_field(te, cx),
        label_color,
    );

    let base_block = labeled_field(
        strings::task_edit_base_label(),
        div()
            .w_full()
            .child(
                select(&te.base_select, cx, 3).placeholder(strings::task_edit_base_active_label()),
            )
            .into_any_element(),
        label_color,
    );

    let subtasks_block = subtasks_section(pane_id, te, cx);

    let editor_column = editor_column(pane_id, te, cx);

    let surface_block = surface_selector(pane_id, te, cx, label_color);

    let auto_execute = te.auto_execute;
    // `auto_execute` has no effect on the AgentChat surface: an ACP `submit`
    // turn runs automatically and there is no dangerous-skip flag, so that
    // surface is inherently auto-execute. Disable the toggle there so its
    // affordance matches reality (the field/state is kept — only interactivity
    // is gated).
    let auto_execute_disabled = matches!(te.agent_surface, TaskAgentSurface::AgentChat);
    let auto_row = checkbox_row(
        checkbox(
            "task-edit-auto-execute",
            strings::task_edit_auto_execute_label(),
            0,
        )
        .checked(auto_execute)
        .disabled(auto_execute_disabled)
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
    // larger click target. Discard lives as the `×` on the Title label
    // row, so the footer carries only the two save actions. Save Draft
    // is the emphasized accent CTA (saving the task is the primary
    // intent); Start is the green "go/run" action, distinct in colour
    // so the two aren't the same accent tone side-by-side.
    //   Save Draft — primary (accent), the emphasized save.
    //   Start — success (green), the "go" / launch action.
    let footer = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap(px(theme::MODAL_FOOTER_GAP))
        .child(
            button_primary("task-edit-save-draft", strings::task_edit_save())
                .small()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_task_edit_pane(pane_id, false, window, cx);
                })),
        )
        .child(
            button("task-edit-start", strings::task_action_start())
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
        // The thin 4px overlay thumb (see `body_scrollbar_thumb`) sits
        // on top of the content with only a 2px right margin, so no
        // dedicated gutter is reserved — the form uses the same
        // symmetric `RIGHT_PANEL_PAD_X` padding as its sibling panels.
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(title_block)
        .child(branch_block)
        .child(base_block)
        .child(surface_block)
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
        .children(body_scrollbar_thumb(
            &scroll_handle,
            scrollbar_thumb,
            scrollbar_thumb_hover,
        ))
}

/// Overlay scrollbar thumb for the task-edit body. Reuses the shared
/// `vertical_thumb` chrome (thin 4px overlay, 2px right margin) so the
/// pane scrolls and looks identical to the file viewer and right dock,
/// instead of the heavier gpui_component scrollbar. The scroll area is
/// pinned to the container's top edge, so the thumb takes a `px(0.)`
/// top offset; its track ends above the absolute footer bar because the
/// measured viewport (`scroll_handle.bounds()`) excludes it. Returns
/// `None` when the content fits without scrolling.
fn body_scrollbar_thumb(
    scroll_handle: &gpui::ScrollHandle,
    thumb: gpui::Hsla,
    thumb_hover: gpui::Hsla,
) -> Option<gpui::AnyElement> {
    let viewport_h = scroll_handle.bounds().size.height;
    let max_offset = scroll_handle.max_offset().y;
    crate::ui::scrollbar::vertical_thumb(
        "task-edit-scrollbar-thumb",
        viewport_h,
        viewport_h + max_offset,
        scroll_handle.offset().y,
        px(0.),
        thumb,
        thumb_hover,
    )
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

/// SubtaskListView. Shows the section title with the current
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
        strings::task_subtask_section_title(),
        done,
        total,
        strings::task_subtask_progress_suffix(),
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
    let muted = theme::current(cx).text_muted;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .child(field_label(strings::task_subtask_section_title(), cx))
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(muted)
                .child(strings::task_subtask_draft_hint()),
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
        strings::task_subtask_auto_label()
    } else {
        strings::task_subtask_manual_label()
    };

    let sub_id_for_remove = sub.id.clone();
    let task_id_for_remove = task_id.clone();
    let remove = button_close(SharedString::from(format!("subtask-remove-{}", sub.id)), cx)
        .on_click(cx.listener(move |this, _ev: &gpui::ClickEvent, _w, cx| {
            this.delete_subtask(&task_id_for_remove, &sub_id_for_remove, cx);
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
        .text_color(theme::current(cx).text_muted)
        .child(text)
}

fn labeled_field(
    label: impl Into<gpui::SharedString>,
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

fn field_label(text: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    field_label_with_color(text, theme::current(cx).text_muted)
}

fn field_label_with_color(
    text: impl Into<gpui::SharedString>,
    color: gpui::Hsla,
) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(color)
        .child(text.into())
}

/// Smaller, dimmer caption rendered inline with a `field_label` —
/// used for contractual notes the user benefits from seeing inline
/// (e.g. "Not included in the agent prompt." beside the Notes
/// section). One size step below `field_label` so it reads as a
/// subtitle rather than a peer.
fn field_hint(text: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .child(text.into())
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
        .child(field_label(strings::task_edit_notes_label(), cx))
        .child(field_hint(strings::task_edit_notes_hint(), cx))
}

/// Header row above the Prompt markdown editor: "Prompt" label on the
/// left, `[📄 Open file]` button on the right. The
/// button is disabled for drafts and Backlog tasks since neither has
/// the on-disk `<wt>/.daruda/task-<branch>.md` file yet — running
/// `[Start]` materialises both the lane and the file in one step.
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
        strings::task_edit_open_file_button(),
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
        .child(field_label(strings::task_edit_prompt_label(), cx))
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
        col = col.child(branch_error(reason.clone()));
    }
    col.into_any_element()
}

fn branch_error(reason: SharedString) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::ERROR)
        .child(reason)
}

fn checkbox_row(widget: impl IntoElement) -> impl IntoElement {
    div().flex().flex_row().items_center().child(widget)
}

/// Execution-surface selector: two radios (Terminal | Agent Chat) bound
/// to `te.agent_surface`. Mirrors the `auto_execute` checkbox's
/// plain-data flip — the click listener sets the enum on the focused
/// pane's content and re-renders. Radios sit outside the Tab cycle
/// (`()`), matching the subtask-row checkboxes, since the surrounding
/// form fields own the numbered slots.
fn surface_selector(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
    label_color: gpui::Hsla,
) -> gpui::AnyElement {
    let current = te.agent_surface;
    let terminal_radio = radio(
        ("task-edit-surface-terminal", pane_id as usize),
        strings::task_edit_surface_terminal(),
        (),
    )
    .checked(matches!(current, TaskAgentSurface::Terminal))
    .on_click(cx.listener(move |this, _checked: &bool, _w, cx| {
        if let Some(te) = this.task_edit_content_mut_for_pane(pane_id) {
            te.agent_surface = TaskAgentSurface::Terminal;
        }
        cx.notify();
    }));
    let agent_radio = radio(
        ("task-edit-surface-agent-chat", pane_id as usize),
        strings::task_edit_surface_agent_chat(),
        (),
    )
    .checked(matches!(current, TaskAgentSurface::AgentChat))
    .on_click(cx.listener(move |this, _checked: &bool, _w, cx| {
        if let Some(te) = this.task_edit_content_mut_for_pane(pane_id) {
            te.agent_surface = TaskAgentSurface::AgentChat;
        }
        cx.notify();
    }));

    labeled_field(
        strings::task_edit_surface_label(),
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::RIGHT_PANEL_ROW_GAP))
            .child(terminal_radio)
            .child(agent_radio)
            .into_any_element(),
        label_color,
    )
    .into_any_element()
}
