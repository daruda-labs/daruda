//! `impl Render` for the merge-into modal — target list, remove-after-merge
//! checkbox, conflicts state, and footer buttons.

use crate::ui::theme;
use gpui::{
    ClickEvent, Context, IntoElement, KeyDownEvent, MouseDownEvent, Render, SharedString, Window,
    div, prelude::*, px,
};

use crate::ui::Disableable as _;
use crate::ui::{button, button_danger, button_primary, checkbox};

use crate::surface::strings as surface_strings;

use super::{MergeModal, MergeState};

impl Render for MergeModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Title is provided by the Dialog wrapper at the entry point.
        let is_merging = matches!(self.state, MergeState::Merging);
        let is_conflicts = matches!(self.state, MergeState::Conflicts(_));
        let show_remove_checkbox = self.source_is_removable();

        // Snapshot selected-target info for footer closures (must happen
        // before any borrow of self.state).
        let sel_branch = self.target_options[self.selected_idx].branch.clone();

        let t = theme::current(cx);
        let strong_text = t.text_primary;
        let muted_text = t.text_muted;
        let row_hover_bg = t.lane_row_hover_bg;
        let radio_dot_color = t.text_primary;

        // ---- branch list ----
        let mut branch_list = div().flex().flex_col().gap(px(theme::MODAL_BUTTON_PAD_Y));
        for (idx, opt) in self.target_options.iter().enumerate() {
            let is_selected = idx == self.selected_idx;
            let branch_label = SharedString::from(opt.branch.clone());
            let row = div()
                .id(("merge-target", idx))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::LANE_LABEL_GAP))
                .px(px(theme::MODAL_BUTTON_PAD_X))
                .py(px(theme::MODAL_BUTTON_PAD_Y))
                .rounded(px(theme::MODAL_BUTTON_RADIUS))
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(if is_selected { strong_text } else { muted_text })
                .bg(if is_selected {
                    row_hover_bg
                } else {
                    theme::TRANSPARENT
                })
                // Branch selection is only interactive before merge starts.
                .when(!is_conflicts && !is_merging, |d| {
                    d.cursor_pointer()
                        .hover(move |d| d.bg(row_hover_bg))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.selected_idx = idx;
                                cx.notify();
                            }),
                        )
                })
                .child(
                    div()
                        .flex_none()
                        .w(px(theme::MODAL_RADIO_W))
                        .text_color(radio_dot_color)
                        .child(crate::ui::icons::icon(if is_selected {
                            crate::ui::icons::RADIO_ON
                        } else {
                            crate::ui::icons::RADIO_OFF
                        })),
                )
                .child(branch_label);
            branch_list = branch_list.child(row);
        }

        // Merging / Conflicts state surfaces an inline note inside the
        // body. Error state goes through `crate::ui::alert::error` so it reads
        // at a glance.
        let inline_status = match &self.state {
            MergeState::Idle | MergeState::Error(_) => None,
            MergeState::Merging => Some(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(surface_strings::modal::merge_merging())
                    .into_any_element(),
            ),
            MergeState::Conflicts(files) => {
                let mut content = div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::MODAL_BUTTON_PAD_Y))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(strong_text)
                            .child(surface_strings::modal::merge_conflicts_note()),
                    );
                if !files.is_empty() {
                    let mut file_list = div().flex().flex_col().pl(px(theme::MODAL_BUTTON_PAD_X));
                    for f in files {
                        file_list = file_list.child(
                            div()
                                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                                .text_color(muted_text)
                                .child(SharedString::from(f.clone())),
                        );
                    }
                    content = content.child(file_list);
                }
                Some(content.into_any_element())
            }
        };

        let error_banner = match &self.state {
            MergeState::Error(msg) => Some(crate::ui::alert::error(
                "merge-error",
                SharedString::from(msg.clone()),
            )),
            _ => None,
        };

        // ---- body ----
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(surface_strings::modal::merge_branch_label()),
            )
            .child(branch_list);

        if show_remove_checkbox && !is_conflicts {
            body = body.child(
                checkbox(
                    "merge-remove-after",
                    surface_strings::modal::merge_remove_after(),
                    0,
                )
                .checked(self.remove_after_merge)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.remove_after_merge = *checked;
                    cx.notify();
                })),
            );
        }

        if let Some(status) = inline_status {
            body = body.child(status);
        }

        // ---- footer ----
        let mut footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP));

        if is_conflicts {
            footer = footer
                .child(
                    button_danger("merge-abort", surface_strings::modal::merge_abort()).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.abort_merge(window, cx);
                        }),
                    ),
                )
                .child(
                    button_primary(
                        "merge-goto",
                        surface_strings::modal::merge_goto_target(&sel_branch),
                    )
                    .child(crate::ui::icons::icon(crate::ui::icons::FORWARD))
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.go_to_target(window, cx);
                        },
                    )),
                );
        } else {
            footer = footer
                .child(
                    button("merge-cancel", surface_strings::common::btn_cancel())
                        .disabled(is_merging)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.dismiss(window, cx);
                        })),
                )
                .child(
                    button_primary("merge-confirm", surface_strings::modal::merge_confirm())
                        .disabled(is_merging)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.submit(window, cx);
                        })),
                );
        }

        let mut panel = div()
            .flex()
            .flex_col()
            .key_context("MergeModal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key(ev, window, cx);
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = error_banner {
            panel = panel.child(b);
        }
        panel.child(footer)
    }
}
