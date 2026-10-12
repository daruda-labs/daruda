use super::*;

impl Render for EditSkillModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let panel_focus = self.panel_focus_handle.clone();
        let name_label = self.name.clone();
        let scope_label = match self.scope {
            SkillScope::Project => strings::common::section_project(),
            SkillScope::Personal => strings::common::section_personal(),
            // Plugin scope is read-only — `open_edit_skill_modal` (the
            // free fn this modal opens through) refuses to open for
            // plugin skills, so this arm is unreachable in practice.
            // The value is purely defensive.
            SkillScope::Plugin => strings::skills::section_plugin(),
        };
        let user_invocable = self.user_invocable;
        let disable_model = self.disable_model_invocation;

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(strings::common::field_name(), &t))
            .child(readonly_value(name_label, cx))
            .child(field_label(strings::common::field_scope(), &t))
            .child(readonly_value(scope_label.to_string(), cx))
            .child(field_label(strings::skills::field_description(), &t))
            .child(input(&self.description_input, cx, 0))
            .child(field_label(strings::skills::field_when_to_use(), &t))
            .child(input(&self.when_to_use_input, cx, 1))
            .child(field_label(strings::skills::field_allowed_tools(), &t))
            .child(input(&self.allowed_tools_input, cx, 2))
            .child(field_label(strings::skills::field_arg_hint(), &t))
            .child(input(&self.argument_hint_input, cx, 3))
            .child(field_label(strings::skills::field_paths(), &t))
            .child(input(&self.paths_input, cx, 4))
            .child(field_label(strings::skills::field_model(), &t))
            .child(input(&self.model_input, cx, 5))
            .child(
                checkbox(
                    "edit-skill-user-invocable",
                    strings::skills::toggle_user_invocable(),
                    7,
                )
                .checked(user_invocable)
                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    if this.submitting || !this.body_loaded {
                        return;
                    }
                    this.user_invocable = *checked;
                    cx.notify();
                })),
            )
            .child(
                checkbox(
                    "edit-skill-disable-model",
                    strings::skills::toggle_disable_model(),
                    8,
                )
                .checked(disable_model)
                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                    if this.submitting || !this.body_loaded {
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
            // two toggles so Tab moves directly from the last metadata
            // field into the markdown body — toggles come last as
            // tail-of-form tweaks.
            .child(input(&self.body_editor, cx, 6));

        let body = div()
            .flex()
            .flex_row()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(left)
            .child(right);

        let dir_for_finder = self.dir.clone();
        let workspace_for_finder = self.workspace.clone();
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("edit-skill-rename", strings::common::btn_rename())
                    .disabled(self.submitting || !self.body_loaded)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, w, cx| this.request_rename(w, cx)),
                    ),
            )
            .child(
                button("edit-skill-finder", strings::skills::button_open_finder()).on_click(
                    cx.listener(move |_, _: &ClickEvent, _w, cx| {
                        if let Some(ws) = workspace_for_finder.upgrade() {
                            let dir = dir_for_finder.clone();
                            ws.update(cx, |ws, cx| ws.open_skill_dir_in_finder(&dir, cx));
                        }
                    }),
                ),
            )
            .child(
                button("edit-skill-cancel", strings::common::btn_cancel())
                    .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.leave(w, cx))),
            )
            .child(
                button_primary(
                    "edit-skill-save",
                    if self.submitting {
                        strings::skills::saving_label()
                    } else {
                        strings::common::btn_save()
                    },
                )
                .disabled(self.submitting || !self.body_loaded)
                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        let banner = self
            .error
            .as_ref()
            .map(|msg| crate::ui::alert::error("edit-skill-error", msg.clone()));

        let mut p = div()
            .flex()
            .flex_col()
            .key_context("EditSkillModal")
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

fn readonly_value(value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .px(px(theme::MODAL_INPUT_PAD))
        .py(px(theme::MODAL_INPUT_PAD))
        .rounded(px(theme::MODAL_BUTTON_RADIUS))
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.border)
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(t.text_primary)
        .child(value.into())
}
