use super::*;
use crate::workspace::right_dock::form_layout;

impl Render for AddMcpServerModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pasting {
            return self.render_paste(cx);
        }
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let has_advanced = super::super::modal_shared::has_advanced_settings(
            self.initial.config.as_ref().map(|config| config.provider),
            self.transport,
        );

        let scope_chip =
            self.scope_choice
                .render(submitting || self.scope_options.len() == 1, 0, cx);
        let transport_chip = self.transport_choice.render(submitting, 2, cx);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .child(form_layout::source(self.target_label(), cx))
            .when(self.duplicate, |body| {
                body.child(field_label(strings::mcp::duplicate_hint(), &t))
            })
            .child(form_layout::pair(
                form_layout::field(field_label(strings::common::field_scope(), &t), scope_chip),
                form_layout::field(
                    field_label(strings::common::field_name(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.name_input,
                        cx,
                        0,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ),
                window,
            ))
            .child(form_layout::field(
                field_label(strings::mcp::field_transport(), &t),
                transport_chip,
            ));

        if self.scope == McpScope::Project
            && self
                .initial
                .config
                .as_ref()
                .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
        {
            body = body.child(field_label(strings::library::project_load_hint(), &t));
        }

        let connection = if self.transport.is_remote() {
            &self.url_input
        } else {
            &self.command_input
        };
        body = body.child(form_layout::field(
            field_label(
                if self.transport.is_remote() {
                    strings::mcp::field_url()
                } else {
                    strings::mcp::field_command()
                },
                &t,
            ),
            super::super::super::form_state::ordered_field_input(
                connection,
                cx,
                1,
                3,
                self.error_field,
                self.error.as_ref(),
            ),
        ));
        if self.transport.is_remote() {
            body = body.child(self.auth.render(submitting, |this| &mut this.auth, cx));
        } else {
            body = body.child(form_layout::field(
                field_label(strings::mcp::field_args(), &t),
                self.args_rows.clone(),
            ));
        }
        body = body.child(form_layout::field(
            field_label(strings::mcp::field_env(), &t),
            self.env_rows.clone(),
        ));
        body = body.when(has_advanced, |body| {
            body.child(
                form_layout::advanced("advanced", self.advanced)
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(7)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
        });
        if self.advanced && has_advanced {
            if self.transport.is_remote() {
                body = body
                    .child(field_label(strings::mcp::field_headers(), &t))
                    .child(form_layout::field(
                        field_label(strings::mcp::auth_headers_hint(), &t),
                        self.header_rows.clone(),
                    ));
            }
            if self
                .initial
                .config
                .as_ref()
                .is_none_or(|c| c.provider == crate::agent::configuration::Provider::Codex)
            {
                body = body.child(
                    checkbox("mcp-disabled", strings::mcp::field_disabled(), 12)
                        .checked(self.disabled)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if !this.submitting {
                                this.disabled = *checked;
                                cx.notify();
                            }
                        })),
                );
            }
            if self
                .initial
                .config
                .as_ref()
                .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex)
            {
                body = body.child(self.codex_options.render(cx));
            }
        }

        if let Some(draft) = &self.imported {
            if draft.transport != self.transport {
                body = body.child(crate::ui::alert::warning(
                    "transport-warning",
                    strings::mcp::transport_warning(),
                ));
            }
            if !draft.extra.is_empty() {
                body = body.child(field_label(
                    strings::mcp::extra_preserved(
                        draft.extra.keys().cloned().collect::<Vec<_>>().join(", "),
                    ),
                    &t,
                ));
            }
        }
        let p = div()
            .flex()
            .flex_col()
            .key_context("AddMcpServerModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        p.into_any_element()
    }
}

impl AddMcpServerModal {
    pub(super) fn form_footer(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.pasting {
            return self.paste_footer(cx);
        }
        let submitting = self.submitting;
        let save_label = if submitting {
            strings::mcp::saving_label()
        } else {
            strings::common::btn_add()
        };
        let footer = form_layout::footer(cx)
            .child(
                form_layout::back(cx)
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(20)
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.pasting = true;
                        w.focus(&this.paste_input.focus_handle(cx), cx);
                        cx.notify();
                    })),
            )
            .child(
                button("add-mcp-cancel", strings::common::btn_cancel())
                    .disabled(self.submitting)
                    .tab_stop(true)
                    .tab_index(21)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("add-mcp-save", save_label)
                    .tab_stop(true)
                    .tab_index(22)
                    .disabled(submitting || self.name_input.read(cx).value().trim().is_empty())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        form_layout::actions(footer, self.error.as_ref()).into_any_element()
    }
}
