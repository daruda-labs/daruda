use super::*;

impl Render for EditMcpServerModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .filter(|_| self.error_field.is_none())
            .map(|msg| crate::ui::alert::error("edit-tool-error", msg.clone()));

        // Read-only summary line: "Name @ Scope"
        let summary_text = t.text_body;
        let summary = div()
            .flex()
            .flex_row()
            .gap(px(theme::MCP_HEADER_GAP))
            .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
            .text_color(summary_text)
            .child(SharedString::from(self.initial.name.clone()))
            .child(SharedString::from(format!(
                "@ {}",
                scope_label(self.initial.scope)
            )));

        // Transport segmented selector. Children align with
        // `transport_values` by index; the click callback maps the
        // selected index back to its `McpTransport`.
        let transport_values: Vec<McpTransport> =
            transport_options().iter().map(|(tp, _)| *tp).collect();
        let transport_chip = button_group("transport-group", cx)
            .children(transport_options().into_iter().map(|(tp, label)| {
                button(
                    SharedString::from(format!("transport-{}", tp.slug())),
                    label,
                )
                .selected(self.transport == tp)
                .disabled(submitting)
                .tab_stop(true)
                .tab_index(2)
            }))
            .on_click(cx.listener(move |this, ixs: &Vec<usize>, _w, cx| {
                if this.submitting {
                    return;
                }
                if let Some(&ix) = ixs.first() {
                    this.transport = transport_values[ix];
                    this.clear_error(cx);
                    cx.notify();
                }
            }));

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::common::field_name(), &t))
            .child(summary)
            .child(field_label(strings::mcp::field_transport(), &t))
            .child(transport_chip);

        let connection = if self.transport.is_remote() {
            &self.url_input
        } else {
            &self.command_input
        };
        body = body
            .child(field_label(
                if self.transport.is_remote() {
                    strings::mcp::field_url()
                } else {
                    strings::mcp::field_command()
                },
                &t,
            ))
            .child(super::super::super::form_state::ordered_field_input(
                connection,
                cx,
                0,
                3,
                self.error_field,
                self.error.as_ref(),
            ));
        if self.transport.is_remote() {
            body = body.child(self.auth.render(submitting, |this| &mut this.auth, cx));
        }
        body = body.child(
            button("advanced", strings::common::field_advanced())
                .disabled(submitting)
                .tab_stop(true)
                .tab_index(7)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced = !this.advanced;
                    cx.notify();
                })),
        );
        if self.advanced {
            let extra_input = if self.transport.is_remote() {
                &self.headers_input
            } else {
                &self.args_input
            };
            if !self.transport.is_remote() {
                body = body
                    .child(field_label(strings::mcp::field_args(), &t))
                    .child(super::super::super::form_state::ordered_field_input(
                        extra_input,
                        cx,
                        1,
                        8,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
            body = body.child(
                button("reveal-values", strings::mcp::reveal_values())
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(9)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reveal_values = !this.reveal_values;
                        cx.notify();
                    })),
            );
            if self.reveal_values {
                if self.transport.is_remote() {
                    body = body
                        .child(field_label(strings::mcp::field_headers(), &t))
                        .child(field_label(strings::mcp::auth_headers_hint(), &t))
                        .child(super::super::super::form_state::ordered_field_input(
                            extra_input,
                            cx,
                            1,
                            10,
                            self.error_field,
                            self.error.as_ref(),
                        ));
                }
                body = body
                    .child(field_label(strings::mcp::field_env(), &t))
                    .child(super::super::super::form_state::ordered_field_input(
                        &self.env_input,
                        cx,
                        2,
                        11,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
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
        if self.transport != self.initial.transport {
            body = body.child(crate::ui::alert::warning(
                "transport-warning",
                strings::mcp::transport_warning(),
            ));
        }

        let save_label = if submitting {
            strings::mcp::saving_label()
        } else {
            strings::common::btn_save()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("duplicate", strings::common::btn_duplicate())
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(13)
                    .on_click(cx.listener(|this, _, w, cx| this.duplicate(w, cx))),
            )
            .child(
                button("edit-mcp-cancel", strings::common::btn_cancel())
                    .tab_stop(true)
                    .tab_index(14)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("edit-mcp-save", save_label)
                    .tab_stop(true)
                    .tab_index(15)
                    .disabled(submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("EditMcpServerModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = banner {
            p = p.child(b);
        }
        p.child(footer)
    }
}

fn scope_label(scope: McpScope) -> String {
    match scope {
        McpScope::Project => strings::mcp::scope_project(),
        McpScope::Local => strings::mcp::scope_local(),
        McpScope::User => strings::mcp::scope_user(),
    }
}
