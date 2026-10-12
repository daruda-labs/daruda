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

        match self.transport {
            McpTransport::Stdio => {
                body = body
                    .child(field_label(strings::mcp::field_command(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.command_input,
                        cx,
                        0,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_args(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.args_input,
                        cx,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
            McpTransport::Sse | McpTransport::Http => {
                body = body
                    .child(field_label(strings::mcp::field_url(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.url_input,
                        cx,
                        0,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_headers(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.headers_input,
                        cx,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
        }

        // Env sits one past the last transport-specific input and the
        // disabled checkbox one past env, so the cycle ends at the
        // checkbox regardless of transport. Name is read-only here, so
        // indices start at 0 (both branches use 0,1 for their inputs).
        body = body
            .child(field_label(strings::mcp::field_env(), &t))
            .child(super::super::super::form_state::field_input(
                &self.env_input,
                cx,
                2,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(
                checkbox("mcp-edit-disabled", strings::mcp::field_disabled(), 3)
                    .checked(self.disabled)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        if this.submitting {
                            return;
                        }
                        this.disabled = *checked;
                        cx.notify();
                    })),
            );

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
                button("edit-mcp-cancel", strings::common::btn_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("edit-mcp-save", save_label)
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
