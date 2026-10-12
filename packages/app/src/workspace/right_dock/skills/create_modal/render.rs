use super::*;

impl Render for CreateSkillModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pasting {
            return self.render_paste(cx);
        }
        let panel_focus = self.panel_focus_handle.clone();
        let user_invocable = self.user_invocable;
        let disable_model = self.disable_model_invocation;
        let submitting = self.submitting;
        let t = crate::ui::theme::current(cx).clone();
        let banner = self
            .error
            .as_ref()
            .filter(|_| self.error_field.is_none())
            .map(|msg| crate::ui::alert::error("create-skill-error", msg.clone()));

        // Scope segmented selector. Children align with `scope_values`
        // by index; the click callback maps the selected index back to
        // its `SkillScope`. `scope_options` only ever holds writable
        // scopes (Project / Personal) — Plugin is read-only and cannot
        // reach the Create modal.
        let scope_pairs: Vec<(SkillScope, SharedString)> = self
            .scope_options
            .iter()
            .filter_map(|scope| match scope {
                SkillScope::Project => Some((*scope, strings::common::section_project().into())),
                SkillScope::Personal => Some((*scope, strings::common::section_personal().into())),
                SkillScope::Plugin => None,
            })
            .collect();
        let scope_values: Vec<SkillScope> = scope_pairs.iter().map(|(scope, _)| *scope).collect();
        let scope_chip = button_group("skill-scope-group", cx)
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

        let mut left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::skills::folder_name(), &t))
            .child(super::super::super::form_state::field_input(
                &self.name_input,
                cx,
                0,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::common::field_scope(), &t))
            .child(scope_chip)
            .child(field_label(strings::skills::field_description(), &t))
            .child(super::super::super::form_state::field_input(
                &self.description_input,
                cx,
                2,
                self.error_field,
                self.error.as_ref(),
            ));
        left = left.child(
            button("skill-advanced", strings::common::field_advanced())
                .disabled(submitting)
                .tab_stop(true)
                .tab_index(3)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced = !this.advanced;
                    cx.notify();
                })),
        );
        if self.advanced {
            left = left
                .child(field_label(strings::skills::invocation_name(), &t))
                .child(crate::ui::input(&self.invocation_input, cx, 4))
                .child(field_label(strings::skills::field_when_to_use(), &t))
                .child(super::super::super::form_state::field_input(
                    &self.when_to_use_input,
                    cx,
                    5,
                    self.error_field,
                    self.error.as_ref(),
                ))
                .child(field_label(strings::skills::field_allowed_tools(), &t))
                .child(super::super::super::form_state::field_input(
                    &self.allowed_tools_input,
                    cx,
                    6,
                    self.error_field,
                    self.error.as_ref(),
                ))
                .child(field_label(strings::skills::field_arg_hint(), &t))
                .child(super::super::super::form_state::field_input(
                    &self.argument_hint_input,
                    cx,
                    7,
                    self.error_field,
                    self.error.as_ref(),
                ))
                .child(field_label(strings::skills::field_paths(), &t))
                .child(super::super::super::form_state::field_input(
                    &self.paths_input,
                    cx,
                    10,
                    self.error_field,
                    self.error.as_ref(),
                ))
                .child(field_label(strings::skills::field_model(), &t))
                .child(super::super::super::form_state::field_input(
                    &self.model_input,
                    cx,
                    11,
                    self.error_field,
                    self.error.as_ref(),
                ))
                .child(
                    checkbox(
                        "skill-user-invocable",
                        strings::skills::toggle_user_invocable(),
                        10,
                    )
                    .checked(user_invocable)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        if this.submitting {
                            return;
                        }
                        this.user_invocable = *checked;
                        cx.notify();
                    })),
                )
                .child(
                    checkbox(
                        "skill-disable-model",
                        strings::skills::toggle_disable_model(),
                        11,
                    )
                    .checked(disable_model)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        if this.submitting {
                            return;
                        }
                        this.disable_model_invocation = *checked;
                        cx.notify();
                    })),
                );
        }

        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::skills::field_body(), &t))
            .child(crate::ui::input(&self.body_editor, cx, -1));

        let body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(right)
            .child(left)
            .when(
                self.body_editor.read(cx).value().trim().is_empty(),
                |body| {
                    body.child(crate::ui::alert::warning(
                        "empty-body",
                        strings::skills::empty_body_warning(),
                    ))
                },
            );

        let save_label = if submitting {
            strings::skills::saving_label()
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
                button("paste-back", strings::common::btn_back())
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(12)
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.pasting = true;
                        w.focus(&this.paste_input.focus_handle(cx), cx);
                        cx.notify();
                    })),
            )
            .child(
                button("create-skill-cancel", strings::common::btn_cancel())
                    .tab_stop(true)
                    .tab_index(13)
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("create-skill-save", save_label)
                    .tab_stop(true)
                    .tab_index(14)
                    .disabled(submitting || self.name_input.read(cx).value().trim().is_empty())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("CreateSkillModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
            .when(self.imported.is_some(), |body| {
                body.child(field_label(strings::skills::duplicate_hint(), &t))
            });
        if let Some(b) = banner {
            p = p.child(b);
        }
        p.child(footer).into_any_element()
    }
}
