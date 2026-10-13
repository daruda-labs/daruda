use super::*;
use crate::workspace::right_dock::form_layout;

impl Render for EditMcpServerModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let has_advanced = super::super::modal_shared::has_advanced_settings(
            self.initial.config.as_ref().map(|config| config.provider),
            self.transport,
        );

        let transport_chip = self.transport_choice.render(submitting, 2, cx);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .children(
                self.initial
                    .target
                    .as_ref()
                    .map(|target| form_layout::source(target.label(), cx)),
            )
            .child(form_layout::pair(
                form_layout::field(
                    field_label(strings::common::field_scope(), &t),
                    crate::ui::readonly_form_input(scope_label(self.initial.scope), cx),
                ),
                form_layout::field(
                    field_label(strings::common::field_name(), &t),
                    crate::ui::readonly_form_input(self.initial.name.clone(), cx),
                ),
                window,
            ))
            .child(form_layout::field(
                field_label(strings::mcp::field_transport(), &t),
                transport_chip,
            ));

        if self.initial.scope == McpScope::Project
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
                0,
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
        if self.transport != self.initial.transport {
            body = body.child(crate::ui::alert::warning(
                "transport-warning",
                strings::mcp::transport_warning(),
            ));
        }

        div()
            .flex()
            .flex_col()
            .key_context("EditMcpServerModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
    }
}

fn scope_label(scope: McpScope) -> String {
    match scope {
        McpScope::Project => strings::mcp::scope_project(),
        McpScope::Local => strings::mcp::scope_local(),
        McpScope::User => strings::mcp::scope_user(),
    }
}

impl EditMcpServerModal {
    pub(super) fn form_footer(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let submitting = self.submitting;
        let save_label = if submitting {
            strings::mcp::saving_label()
        } else {
            strings::common::btn_save()
        };
        let footer = form_layout::footer(cx)
            .child(
                super::super::saved::guide_button(
                    self.initial.config.as_ref().map(|c| c.provider),
                    cx,
                )
                .disabled(submitting),
            )
            .when(self.error.is_some(), |footer| {
                footer.child(
                    button("reload-source", strings::library::reload())
                        .disabled(self.submitting)
                        .tab_stop(true)
                        .tab_index(19)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.request_reload(window, cx)),
                        ),
                )
            })
            .child(
                button("duplicate", strings::common::btn_duplicate())
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(20)
                    .on_click(cx.listener(|this, _, w, cx| this.duplicate(w, cx))),
            )
            .child(
                button("edit-mcp-cancel", strings::common::btn_cancel())
                    .disabled(self.submitting)
                    .tab_stop(true)
                    .tab_index(21)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("edit-mcp-save", save_label)
                    .tab_stop(true)
                    .tab_index(22)
                    .disabled(submitting || !self.form.is_dirty(&self.values(cx)))
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        form_layout::actions(footer, self.error.as_ref()).into_any_element()
    }
}
