//! Prompt-first task form with contextual execution controls.

mod editor_ops;
mod prompt_file_ops;
pub(in crate::workspace) mod run_in_ops;
mod save_ops;
pub(in crate::workspace) mod state;
pub(super) mod task_edit_ops;

use daruda_store::tasks::{SubTask, TaskAgentSurface, TaskExecution, TaskState};
use gpui::{Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use super::super::Workspace;
use super::pane_tree::PaneId;
use crate::agent::tasks_global::GlobalTasks;
use crate::surface::strings;
use crate::ui::{self, ButtonVariants as _, Disableable as _, theme};
use crate::ui::{button, checkbox};
use run_in_ops::task_running_in;
use state::{BranchValidation, RunInChoice, TaskEditContent};

pub(in crate::workspace) fn render(
    pane_id: PaneId,
    te: &TaskEditContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let (border, muted, background, foreground, thumb, thumb_hover) = {
        let t = theme::current(cx);
        (
            t.border,
            t.text_muted,
            t.task_edit_bg,
            t.text_primary,
            t.scrollbar_thumb,
            t.task_edit_scrollbar_thumb_hover,
        )
    };
    let task = te
        .task_id
        .as_deref()
        .and_then(|id| cx.global::<GlobalTasks>().get(id));
    let state = task.map(|task| task.state.clone());
    let can_start = state
        .as_ref()
        .is_none_or(|state| matches!(state, TaskState::Backlog));
    let has_worktree = state.as_ref().and_then(TaskState::worktree_path).is_some();
    let can_open_chat = task.is_some_and(|task| {
        task.execution
            .as_ref()
            .is_some_and(TaskExecution::chat_available)
    });
    let subtasks = task
        .map(|task| task.subtasks.clone())
        .unwrap_or_else(|| te.draft_subtasks.clone());
    let status = match state {
        None => strings::task_edit_new(),
        Some(TaskState::Backlog) => strings::right_panel_task_backlog(),
        Some(TaskState::Running { .. }) => strings::right_panel_task_running(),
        Some(TaskState::Done { .. }) => strings::right_panel_task_done_prefix(),
        Some(TaskState::Error { .. }) => strings::task_edit_error(),
        Some(TaskState::Cancelled { .. }) => strings::task_edit_cancelled(),
    };
    let header =
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(theme::TASK_EDIT_HEADER_H))
            .px(px(theme::PAD_LG))
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::GAP_LG))
                    .child(
                        ui::button_icon(("task-edit-back", pane_id as usize), ui::icons::BACK, cx)
                            .tooltip(strings::task_edit_back())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_page(crate::workspace::pages::Page::Tasks, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(theme::FONT_SIZE_SM))
                            .text_color(muted)
                            .child(status),
                    ),
            )
            .child(
                ui::button_close(("task-edit-close", pane_id as usize), cx).on_click(cx.listener(
                    move |this, _, window, cx| this.request_close_pane(pane_id, window, cx),
                )),
            );

    let body = div()
        .w_full()
        .max_w(px(theme::TASK_EDIT_MAX_WIDTH))
        .mx_auto()
        .flex()
        .flex_col()
        .gap(px(theme::PAD_XL))
        .p(px(theme::PAD_XL))
        .child(field(
            strings::task_edit_title_label(),
            ui::input(&te.title_input, cx, 0).into_any_element(),
            cx,
        ))
        .child(prompt(pane_id, te, te._prompt_watcher.is_some(), cx))
        .child(settings(pane_id, te, can_start, cx))
        .child(subtasks_section(pane_id, te, subtasks, cx))
        .child(notes(pane_id, te, cx));

    let footer = div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(px(theme::TASK_EDIT_FOOTER_H))
        .border_t_1()
        .border_color(border)
        .child(
            div()
                .w_full()
                .max_w(px(theme::TASK_EDIT_MAX_WIDTH))
                .mx_auto()
                .h_full()
                .px(px(theme::PAD_XL))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(theme::GAP_LG))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(muted)
                        .child(if te.is_dirty(cx) {
                            strings::task_edit_unsaved()
                        } else {
                            String::new()
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(theme::GAP_LG))
                        .when(can_open_chat, |row| {
                            row.child(
                                ui::button_icon(
                                    ("task-edit-chat", pane_id as usize),
                                    ui::icons::AGENT,
                                    cx,
                                )
                                .tooltip(strings::task_action_open_chat())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_editor_task_chat(pane_id, window, cx)
                                    },
                                )),
                            )
                        })
                        .when(has_worktree, |row| {
                            row.child(
                                ui::button_icon(
                                    ("task-edit-worktree", pane_id as usize),
                                    ui::icons::FOLDER_OPEN,
                                    cx,
                                )
                                .tooltip(strings::task_action_open())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_editor_task_worktree(pane_id, window, cx)
                                    },
                                )),
                            )
                        })
                        .child(
                            ui::button_primary(
                                ("task-edit-save", pane_id as usize),
                                strings::task_edit_save(),
                            )
                            .disabled(!te.can_save(cx))
                            .tab_stop(true)
                            .tab_index(10)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.save_task_editor(pane_id, false, window, cx)
                                },
                            )),
                        )
                        .when(can_start, |row| {
                            row.child(
                                button(
                                    ("task-edit-start", pane_id as usize),
                                    strings::task_edit_save_start(),
                                )
                                .disabled(!te.can_save(cx))
                                .tab_stop(true)
                                .tab_index(11)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.save_task_editor(pane_id, true, window, cx)
                                    },
                                )),
                            )
                        }),
                ),
        );

    let scroll = te.body_scroll_handle.clone();
    div()
        .key_context("TaskEditPane")
        .tab_group()
        .relative()
        .size_full()
        .bg(background)
        .text_color(foreground)
        .on_key_down(cx.listener(move |this, event, window, cx| {
            this.handle_task_edit_key(pane_id, event, window, cx)
        }))
        .on_action(cx.listener(
            move |this, _: &crate::workspace::SaveFilePane, window, cx| {
                this.save_task_editor(pane_id, false, window, cx)
            },
        ))
        .child(header)
        .child(
            div()
                .id(("task-edit-body", pane_id as usize))
                .absolute()
                .top(px(theme::TASK_EDIT_HEADER_H))
                .bottom(px(theme::TASK_EDIT_FOOTER_H))
                .left_0()
                .right_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .child(body),
        )
        .child(footer)
        .children(ui::scrollbar::vertical_thumb(
            ("task-edit-scrollbar", pane_id as usize),
            scroll.bounds().size.height,
            scroll.bounds().size.height + scroll.max_offset().y,
            scroll.offset().y,
            px(theme::TASK_EDIT_HEADER_H),
            thumb,
            thumb_hover,
        ))
}

fn field(label: String, body: gpui::AnyElement, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::GAP_LG))
        .child(
            div()
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(theme::current(cx).text_muted)
                .child(label),
        )
        .child(body)
}

fn prompt(
    pane_id: PaneId,
    te: &TaskEditContent,
    has_file: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .child(
            ui::tab_bar(("task-edit-prompt-mode", pane_id as usize))
                .selected_index(usize::from(te.preview_prompt))
                .child(ui::tab(strings::task_edit_prompt_label()))
                .child(ui::tab(strings::file_viewer_tab_preview()))
                .on_click(cx.listener(move |this, index: &usize, _, cx| {
                    this.set_task_prompt_preview(pane_id, *index, cx)
                })),
        )
        .when(has_file, |row| {
            row.child(
                ui::button_icon(
                    ("task-edit-open-file", pane_id as usize),
                    ui::icons::FOLDER_OPEN,
                    cx,
                )
                .tooltip(strings::task_edit_open_file_button())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor_prompt_file(pane_id, window, cx)
                })),
            )
        });
    let body = if te.preview_prompt {
        div()
            .min_h(px(theme::TASK_EDIT_PREVIEW_MIN_H))
            .w_full()
            .child(
                ui::markdown(
                    ("task-edit-preview", pane_id as usize),
                    te.prompt_state.read(cx).value(),
                )
                .text_size(px(theme::editor_font_size(cx)))
                .selectable(true)
                .full_width(true),
            )
            .into_any_element()
    } else {
        ui::markdown_editor(&te.prompt_state, cx)
            .tab_index(1)
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::GAP_LG))
        .child(header)
        .child(body)
}

fn settings(
    pane_id: PaneId,
    te: &TaskEditContent,
    editable: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .border_t_1()
        .border_color(theme::current(cx).border)
        .child(
            button(
                ("task-edit-settings", pane_id as usize),
                strings::task_edit_settings(),
            )
            .ghost()
            .justify_start()
            .w_full()
            .tab_stop(true)
            .tab_index(2)
            .icon(ui::icons::icon(if te.settings_open {
                ui::icons::EXPAND_MORE
            } else {
                ui::icons::CHEVRON_RIGHT
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_task_settings(pane_id, cx))),
        );
    if !te.settings_open {
        return section;
    }
    // A Backlog task has not created its lane yet, so its branch is still
    // free to change; once started, it names that lane.
    let branch = if !editable {
        div()
            .text_size(px(theme::FONT_SIZE_SM))
            .child(
                ui::selectable_text(
                    ("task-edit-branch-value", pane_id as usize),
                    te.branch_input.read(cx).value(),
                )
                .selectable(true),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_SM))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::GAP_SM))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(ui::input(&te.branch_input, cx, 3)),
                    )
                    .child(
                        ui::button_icon(
                            ("task-edit-regenerate-branch", pane_id as usize),
                            ui::icons::REFRESH,
                            cx,
                        )
                        .tooltip(strings::task_edit_branch_regenerate())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.regenerate_task_branch(pane_id, window, cx)
                            },
                        )),
                    ),
            )
            .when(te.branch_validation.is_invalid(), |column| {
                let message = match &te.branch_validation {
                    BranchValidation::Invalid { reason } => reason.clone(),
                    BranchValidation::Exists => strings::task_edit_branch_exists().into(),
                    _ => SharedString::default(),
                };
                column.child(
                    div()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(theme::ERROR)
                        .child(message),
                )
            })
            .into_any_element()
    };
    section = section.child(field(
        strings::task_edit_run_in_label(),
        run_in(pane_id, te, editable, cx).into_any_element(),
        cx,
    ));
    section = match te.run_in {
        RunInChoice::NewWorktree => section
            .child(field(strings::task_edit_branch_label(), branch, cx))
            .child(field(
                strings::task_edit_base_label(),
                ui::select::select(&te.base_select, cx, 4)
                    .disabled(!editable)
                    .placeholder(strings::task_edit_base_active_label())
                    .into_any_element(),
                cx,
            )),
        RunInChoice::ExistingLane => section,
    };
    section = section.child(field(
        strings::task_edit_surface_label(),
        div()
            .flex()
            .flex_wrap()
            .gap(px(theme::GAP_LG))
            .child(
                ui::radio(
                    ("task-edit-surface-terminal", pane_id as usize),
                    strings::task_edit_surface_terminal(),
                    5,
                )
                .disabled(!editable)
                .checked(te.agent_surface == TaskAgentSurface::Terminal)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_task_surface(pane_id, TaskAgentSurface::Terminal, cx)
                })),
            )
            .child(
                ui::radio(
                    ("task-edit-surface-chat", pane_id as usize),
                    strings::task_edit_surface_agent_chat(),
                    6,
                )
                .disabled(!editable)
                .checked(te.agent_surface == TaskAgentSurface::AgentChat)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_task_surface(pane_id, TaskAgentSurface::AgentChat, cx)
                })),
            )
            .into_any_element(),
        cx,
    ));
    if te.agent_surface == TaskAgentSurface::Terminal {
        section = section.child(
            checkbox(
                ("task-edit-auto", pane_id as usize),
                strings::task_edit_auto_execute_label(),
                7,
            )
            .checked(te.auto_execute)
            .disabled(!editable)
            .on_click(cx.listener(move |this, enabled: &bool, _, cx| {
                this.set_task_auto_execute(pane_id, *enabled, cx)
            })),
        );
    }
    section
}

/// New / Existing worktree radios; under Existing, the lane picker and a
/// warning when another task already runs in the picked lane.
fn run_in(
    pane_id: PaneId,
    te: &TaskEditContent,
    editable: bool,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let radios = div().flex().flex_wrap().gap(px(theme::GAP_LG)).children(
        [
            (
                RunInChoice::NewWorktree,
                strings::task_edit_run_in_new(),
                "task-edit-run-in-new",
            ),
            (
                RunInChoice::ExistingLane,
                strings::task_edit_run_in_existing(),
                "task-edit-run-in-existing",
            ),
        ]
        .map(|(choice, label, id)| {
            ui::radio((id, pane_id as usize), label, 3)
                .disabled(!editable)
                .checked(te.run_in == choice)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_task_run_in(pane_id, choice, cx)),
                )
        }),
    );
    let mut column = div().flex().flex_col().gap(px(theme::GAP_SM)).child(radios);
    if te.run_in == RunInChoice::ExistingLane {
        column = column.child(
            ui::select::select(&te.lane_select, cx, 4)
                .disabled(!editable)
                .placeholder(strings::task_edit_run_in_lane_placeholder()),
        );
        let lane = te.lane_value(cx);
        let busy = (!lane.is_empty())
            .then(|| {
                task_running_in(
                    cx.global::<GlobalTasks>(),
                    std::path::Path::new(&lane),
                    te.task_id.as_deref(),
                )
            })
            .flatten()
            .map(|task| strings::task_edit_run_in_lane_busy(&task.title));
        column = column.children(busy.map(|message| {
            div()
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(theme::WARNING)
                .child(message)
        }));
    }
    column
}

fn notes(pane_id: PaneId, te: &TaskEditContent, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .border_t_1()
        .border_color(theme::current(cx).border)
        .child(
            button(
                ("task-edit-notes", pane_id as usize),
                strings::task_edit_notes_label(),
            )
            .ghost()
            .justify_start()
            .w_full()
            .tab_stop(true)
            .tab_index(9)
            .icon(ui::icons::icon(if te.notes_open {
                ui::icons::EXPAND_MORE
            } else {
                ui::icons::CHEVRON_RIGHT
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_task_notes(pane_id, cx))),
        )
        .when(te.notes_open, |section| {
            section
                .child(
                    div()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(theme::current(cx).text_muted)
                        .child(strings::task_edit_notes_hint()),
                )
                .child(ui::markdown_editor(&te.notes_state, cx).tab_index(9))
        })
}

fn subtasks_section(
    pane_id: PaneId,
    te: &TaskEditContent,
    subtasks: Vec<SubTask>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let done = subtasks.iter().filter(|subtask| subtask.completed).count();
    let title = strings::task_edit_subtasks_progress(done, subtasks.len());
    let mut list = div().flex().flex_col().gap(px(theme::GAP_LG));
    for subtask in subtasks {
        let editing = te.editing_subtask.as_deref() == Some(subtask.id.as_str());
        list = list.child(subtask_row(pane_id, subtask, editing, te, cx));
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
                    ("task-edit-add-subtask", pane_id as usize),
                    ui::icons::ADD,
                    cx,
                )
                .tooltip(strings::task_subtask_add_placeholder())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.submit_new_subtask(pane_id, window, cx)
                })),
            ),
    );
    field(title, list.into_any_element(), cx)
}

fn subtask_row(
    pane_id: PaneId,
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
        this.toggle_editor_subtask(pane_id, &sub_id_for_toggle, cx);
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
                    this.handle_task_subtask_mouse_down(pane_id, &sub_id_for_rename, ev, window, cx)
                }),
            )
            .into_any_element()
    };

    let auto_manual_label = if sub.source_session_id.is_some() {
        strings::task_subtask_auto_label()
    } else {
        strings::task_subtask_manual_label()
    };

    let sub_id_for_remove = sub.id.clone();
    let remove = crate::ui::button_delete_glyph(
        SharedString::from(format!("subtask-remove-{}", sub.id)),
        cx,
    )
    .on_click(cx.listener(move |this, _ev: &gpui::ClickEvent, _w, cx| {
        this.delete_editor_subtask(pane_id, &sub_id_for_remove, cx);
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
