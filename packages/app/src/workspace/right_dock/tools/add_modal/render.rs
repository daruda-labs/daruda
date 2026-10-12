use super::*;

impl Render for AddMcpServerModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
            .filter(|_| self.error_field.is_none())
            .map(|msg| crate::ui::alert::error("add-tool-error", msg.clone()));

        // Scope segmented selector. Children align with `scope_values`
        // by index, so the click callback maps the selected index back
        // to its `McpScope`.
        let scope_pairs: Vec<(McpScope, String)> = scope_options()
            .into_iter()
            .filter(|(scope, _)| self.scope_options.contains(scope))
            .collect();
        let scope_values: Vec<McpScope> = scope_pairs.iter().map(|(scope, _)| *scope).collect();
        let scope_chip = button_group("scope-group", cx)
            .children(scope_pairs.into_iter().map(|(scope, label)| {
                button(SharedString::from(format!("scope-{}", scope.slug())), label)
                    .selected(self.scope == scope)
            }))
            .on_click(cx.listener(move |this, ixs: &Vec<usize>, _w, cx| {
                if this.submitting {
                    return;
                }
                if let Some(&ix) = ixs.first() {
                    this.scope = scope_values[ix];
                    this.clear_error(cx);
                    cx.notify();
                }
            }));

        // Transport segmented selector — same index→value mapping.
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
            .child(super::super::super::form_state::field_input(
                &self.name_input,
                cx,
                0,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::common::field_scope(), &t))
            .child(scope_chip)
            .child(field_label(strings::mcp::field_transport(), &t))
            .child(transport_chip);

        match self.transport {
            McpTransport::Stdio => {
                body = body
                    .child(field_label(strings::mcp::field_command(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.command_input,
                        cx,
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_args(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.args_input,
                        cx,
                        2,
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
                        1,
                        self.error_field,
                        self.error.as_ref(),
                    ))
                    .child(field_label(strings::mcp::field_headers(), &t))
                    .child(super::super::super::form_state::field_input(
                        &self.headers_input,
                        cx,
                        2,
                        self.error_field,
                        self.error.as_ref(),
                    ));
            }
        }

        // Disabled checkbox slot — its tab_index sits one past the last
        // transport-specific input so it lands at the end of the cycle
        // regardless of the active transport (Stdio uses 1,2; remote
        // uses 1,2 for url+headers — env uses 3 across both branches).
        body = body
            .child(field_label(strings::mcp::field_env(), &t))
            .child(super::super::super::form_state::field_input(
                &self.env_input,
                cx,
                3,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(
                checkbox("mcp-disabled", strings::mcp::field_disabled(), 4)
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
            strings::common::btn_add()
        };
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("add-mcp-cancel", strings::common::btn_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("add-mcp-save", save_label)
                    .disabled(submitting || self.name_input.read(cx).value().trim().is_empty())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("AddMcpServerModal")
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
