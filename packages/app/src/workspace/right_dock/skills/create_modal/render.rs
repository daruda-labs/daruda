use super::*;
use crate::workspace::right_dock::form_layout;

impl Render for CreateSkillModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pasting {
            return self.render_paste(cx);
        }
        let codex = self
            .config
            .as_ref()
            .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex);
        let panel_focus = self.panel_focus_handle.clone();
        let user_invocable = self.user_invocable;
        let disable_model = self.disable_model_invocation;
        let submitting = self.submitting;
        let t = crate::ui::theme::current(cx).clone();

        let scope_chip =
            self.scope_choice
                .render(submitting || self.scope_options.len() == 1, 0, cx);

        let mut left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .child(form_layout::pair(
                form_layout::field(field_label(strings::common::field_scope(), &t), scope_chip),
                form_layout::field(
                    field_label(strings::skills::folder_name(), &t),
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
                field_label(
                    if codex {
                        strings::skills::invocation_name_required()
                    } else {
                        strings::skills::invocation_name()
                    },
                    &t,
                ),
                super::super::super::form_state::ordered_field_input(
                    &self.invocation_input,
                    cx,
                    1,
                    2,
                    self.error_field,
                    self.error.as_ref(),
                ),
            ))
            .child(form_layout::field(
                field_label(
                    if codex {
                        strings::skills::description_required()
                    } else {
                        strings::skills::field_description()
                    },
                    &t,
                ),
                super::super::super::form_state::ordered_field_input(
                    &self.description_input,
                    cx,
                    2,
                    3,
                    self.error_field,
                    self.error.as_ref(),
                ),
            ))
            .child(form_layout::field(
                field_label(strings::skills::field_body(), &t),
                crate::ui::form_input(&self.body_editor, cx, 4),
            ));
        left = left.when(!codex, |left| {
            left.child(
                form_layout::advanced("skill-advanced", self.advanced)
                    .disabled(submitting)
                    .tab_stop(true)
                    .tab_index(5)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
        });
        if self.advanced && !codex {
            left = left
                .child(form_layout::field(
                    field_label(strings::skills::field_when_to_use(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.when_to_use_input,
                        cx,
                        5,
                        6,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_allowed_tools(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.allowed_tools_input,
                        cx,
                        6,
                        7,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_arg_hint(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.argument_hint_input,
                        cx,
                        7,
                        8,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_paths(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.paths_input,
                        cx,
                        10,
                        9,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_model(), &t),
                    super::super::super::form_state::ordered_field_input(
                        &self.model_input,
                        cx,
                        11,
                        10,
                        self.error_field,
                        self.error.as_ref(),
                    ),
                ))
                .child(
                    checkbox(
                        "skill-user-invocable",
                        strings::skills::toggle_user_invocable(),
                        11,
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
                        12,
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

        let body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .children(self.config.as_ref().map(|c| {
                form_layout::source(
                    strings::library::source(
                        match self.scope {
                            SkillScope::Personal => c.personal_skills.clone(),
                            SkillScope::Project => c
                                .cwd
                                .as_ref()
                                .map(|p| c.project_skills(p))
                                .unwrap_or_default(),
                            SkillScope::Plugin => Default::default(),
                        }
                        .display(),
                    ),
                    cx,
                )
            }))
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

        let p = div()
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
        p.into_any_element()
    }
}

impl CreateSkillModal {
    pub(super) fn form_footer(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.pasting {
            return self.paste_footer(cx);
        }
        let submitting = self.submitting;
        let save_label = if submitting {
            strings::skills::saving_label()
        } else {
            strings::common::btn_save()
        };
        let footer = form_layout::footer(cx)
            .child(
                form_layout::back(cx)
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
                    .disabled(self.submitting)
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

        form_layout::actions(footer, self.error.as_ref()).into_any_element()
    }
}
