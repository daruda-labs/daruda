use super::*;
use crate::workspace::right_dock::form_layout;
use crate::workspace::right_dock::form_layout::{input, readonly_value};

impl Render for EditSkillModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::ui::theme::current(cx).clone();
        let codex = self
            .config
            .as_ref()
            .is_some_and(|c| c.provider == crate::agent::configuration::Provider::Codex);
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

        let mut left = div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .child(form_layout::pair(
                form_layout::field(
                    field_label(strings::common::field_scope(), &t),
                    readonly_value(scope_label.to_string(), cx),
                ),
                form_layout::field(
                    field_label(strings::common::field_name(), &t),
                    readonly_value(name_label, cx),
                ),
                window,
            ))
            .child(form_layout::field(
                field_label(strings::skills::invocation_name(), &t),
                input(&self.invocation_input, cx, 0),
            ))
            .child(form_layout::field(
                field_label(strings::skills::field_description(), &t),
                input(&self.description_input, cx, 1),
            ))
            .child(form_layout::field(
                field_label(strings::skills::field_body(), &t),
                input(&self.body_editor, cx, 2),
            ));
        left = left.when(!codex, |left| {
            left.child(
                form_layout::advanced("skill-advanced", self.advanced)
                    .disabled(self.submitting)
                    .tab_stop(true)
                    .tab_index(3)
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
                    input(&self.when_to_use_input, cx, 4),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_allowed_tools(), &t),
                    input(&self.allowed_tools_input, cx, 5),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_arg_hint(), &t),
                    input(&self.argument_hint_input, cx, 6),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_paths(), &t),
                    input(&self.paths_input, cx, 7),
                ))
                .child(form_layout::field(
                    field_label(strings::skills::field_model(), &t),
                    input(&self.model_input, cx, 8),
                ))
                .child(
                    checkbox(
                        "edit-skill-user-invocable",
                        strings::skills::toggle_user_invocable(),
                        9,
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
                        10,
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
        }

        let companion = self.companion_path.clone();
        let companion_workspace = self.workspace.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_SPLIT_GAP))
            .child(form_layout::source(
                strings::library::source(self.dir.display()),
                cx,
            ))
            .when_some(companion, |body, path| {
                body.child(
                    button("skill-companion", strings::library::companion()).on_click(
                        move |_, window, cx| {
                            if let Some(ws) = companion_workspace.upgrade() {
                                ws.update(cx, |ws, cx| {
                                    ws.open_reference_file(path.clone(), window, cx)
                                });
                            }
                        },
                    ),
                )
            })
            .child(left);

        div()
            .flex()
            .flex_col()
            .key_context("EditSkillModal")
            .track_focus(&panel_focus)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body)
    }
}

impl EditSkillModal {
    pub(super) fn form_footer(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let dir_for_finder = self.dir.clone();
        let workspace_for_finder = self.workspace.clone();
        let footer = form_layout::footer(cx)
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
                button("duplicate", strings::skills::duplicate_instructions())
                    .disabled(self.submitting || !self.body_loaded)
                    .tab_stop(true)
                    .tab_index(20)
                    .on_click(cx.listener(|this, _, w, cx| this.duplicate(w, cx))),
            )
            .child(
                button("edit-skill-rename", strings::common::btn_rename())
                    .tab_stop(true)
                    .tab_index(21)
                    .disabled(self.submitting || !self.body_loaded)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, w, cx| this.request_rename(w, cx)),
                    ),
            )
            .child(
                button("edit-skill-finder", strings::skills::button_open_finder())
                    .tab_stop(true)
                    .tab_index(22)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _w, cx| {
                        if let Some(ws) = workspace_for_finder.upgrade() {
                            let dir = dir_for_finder.clone();
                            ws.update(cx, |ws, cx| ws.open_skill_dir_in_finder(&dir, cx));
                        }
                    })),
            )
            .child(
                button("edit-skill-cancel", strings::common::btn_cancel())
                    .disabled(self.submitting)
                    .tab_stop(true)
                    .tab_index(23)
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
                .disabled(
                    self.submitting || !self.body_loaded || !self.form.is_dirty(&self.values(cx)),
                )
                .tab_stop(true)
                .tab_index(24)
                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| this.submit(w, cx))),
            );

        form_layout::actions(footer, self.error.as_ref()).into_any_element()
    }
}
