use super::*;

impl Render for AddMcpServerModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pasting {
            return self.render_paste(cx);
        }
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let submitting = self.submitting;
        let banner = self
            .error
            .as_ref()
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
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(1)
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
        let options: Vec<_> =
            transport_options()
                .into_iter()
                .filter(|(transport, _)| {
                    *transport != McpTransport::Sse
                        || self.initial.config.as_ref().is_none_or(|c| {
                            c.provider == crate::agent::configuration::Provider::Claude
                        })
                })
                .collect();
        let transport_values: Vec<McpTransport> = options.iter().map(|(tp, _)| *tp).collect();
        let transport_chip = button_group("transport-group", cx)
            .children(options.into_iter().map(|(tp, label)| {
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
            .child(field_label(self.target_label(), &t))
            .when(self.duplicate, |body| {
                body.child(field_label(strings::mcp::duplicate_hint(), &t))
            })
            .child(field_label(strings::common::field_name(), &t))
            .child(super::super::super::form_state::ordered_field_input(
                &self.name_input,
                cx,
                0,
                0,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::common::field_scope(), &t))
            .child(scope_chip)
            .child(field_label(strings::mcp::field_transport(), &t))
            .child(transport_chip);

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
                1,
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
            if !self.transport.is_remote() {
                body = body
                    .child(field_label(strings::mcp::field_args(), &t))
                    .child(self.args_rows.clone());
            }
            if self.transport.is_remote() {
                body = body
                    .child(field_label(strings::mcp::field_headers(), &t))
                    .child(field_label(strings::mcp::auth_headers_hint(), &t))
                    .child(self.header_rows.clone());
            }
            body = body
                .child(field_label(strings::mcp::field_env(), &t))
                .child(self.env_rows.clone());
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
                button("paste-back", strings::common::btn_back())
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
        p.child(footer).into_any_element()
    }
}
