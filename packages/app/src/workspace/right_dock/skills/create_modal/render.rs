use super::*;

impl Render for CreateSkillModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
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
            .child(field_label(strings::skills::field_description(), &t))
            .child(super::super::super::form_state::field_input(
                &self.description_input,
                cx,
                1,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::skills::field_when_to_use(), &t))
            .child(super::super::super::form_state::field_input(
                &self.when_to_use_input,
                cx,
                2,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::skills::field_allowed_tools(), &t))
            .child(super::super::super::form_state::field_input(
                &self.allowed_tools_input,
                cx,
                3,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::skills::field_arg_hint(), &t))
            .child(super::super::super::form_state::field_input(
                &self.argument_hint_input,
                cx,
                4,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::skills::field_paths(), &t))
            .child(super::super::super::form_state::field_input(
                &self.paths_input,
                cx,
                5,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(field_label(strings::skills::field_model(), &t))
            .child(super::super::super::form_state::field_input(
                &self.model_input,
                cx,
                6,
                self.error_field,
                self.error.as_ref(),
            ))
            .child(
                checkbox(
                    "skill-user-invocable",
                    strings::skills::toggle_user_invocable(),
                    8,
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
                    9,
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

        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::skills::field_body(), &t))
            // body_editor sits between the left-column inputs and the
            // toggles so Tab flows from the last metadata field into
            // the markdown body before reaching the tail-of-form
            // toggles.
            .child(super::super::super::form_state::field_input(
                &self.body_editor,
                cx,
                7,
                self.error_field,
                self.error.as_ref(),
            ));

        let body = div()
            .flex()
            .flex_row()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(left)
            .child(right);

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
                button("create-skill-cancel", strings::common::btn_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary("create-skill-save", save_label)
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
            .child(body);
        if let Some(b) = banner {
            p = p.child(b);
        }
        p.child(footer)
    }
}
