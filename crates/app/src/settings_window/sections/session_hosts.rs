//! Session Hosts page: the `[[session_hosts]]` registry editor.
//!
//! Model closely mirrors [`super::agent`]'s catalog editor (row
//! add/edit/delete over a `Vec`, staged in memory until Save), minus the
//! preset concept agents have — every row here is a plain user-entered
//! `{label, kind, target|container}`. See `SettingsWindow::validate` for
//! the label-uniqueness check and the tombstone/redirect bookkeeping this
//! page's Save triggers.

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{button, button_danger, field_row};
use gpui::{AnyElement, ClickEvent, IntoElement, div, prelude::*, px};

use super::super::{SessionHostRow, SettingsWindow};

impl SettingsWindow {
    pub(in crate::settings_window) fn render_session_hosts(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let description_color = theme::current(cx).text_muted;

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_session_hosts(), cx))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_session_hosts_description()),
            )
            .child(div().flex().flex_row().child(
                button("settings-session-host-add", s::settings_session_host_add()).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_session_host_row(window, cx);
                    }),
                ),
            ));

        if self.session_host_rows().next().is_none() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(description_color)
                    .child(s::settings_session_hosts_empty()),
            );
        }

        for (index, row) in self.session_host_rows() {
            body = body.child(Self::render_session_host_row(index, row, cx));
        }

        body.into_any_element()
    }

    fn render_session_host_row(
        index: usize,
        row: &SessionHostRow,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let remove_id = format!("settings-session-host-remove-{index}");
        let is_docker = row.is_docker(cx);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::MODAL_PANEL_GAP))
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_MD))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_primary)
                            .child(s::settings_session_host_row_label(index + 1)),
                    )
                    .child(
                        button_danger(remove_id, s::settings_session_host_remove()).on_click(
                            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.remove_session_host_row(index, cx);
                            }),
                        ),
                    ),
            )
            .child(field_row(
                s::settings_session_host_field_label(),
                crate::ui::input(&row.label_input, cx, ()),
            ))
            .child(field_row(
                s::settings_session_host_field_kind(),
                crate::ui::select::select(&row.kind_select, cx, ()),
            ));

        // Only one of target/container is meaningful per kind — show just
        // that field, mirroring the agent catalog row's ssh/docker split.
        if is_docker {
            body = body.child(field_row(
                s::settings_session_host_field_container(),
                crate::ui::input(&row.container_input, cx, ()),
            ));
        } else {
            body = body.child(field_row(
                s::settings_session_host_field_target(),
                crate::ui::input(&row.target_input, cx, ()),
            ));
        }

        body.into_any_element()
    }
}
