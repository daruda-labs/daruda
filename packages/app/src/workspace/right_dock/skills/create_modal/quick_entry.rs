use super::*;
use crate::agent::skills::import;

impl CreateSkillModal {
    pub(super) fn review_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match import::parse(&self.paste_input.read(cx).value()) {
            Ok(imported) => self.apply_import(imported, window, cx),
            Err(()) => {
                self.error_field = None;
                self.error = Some(strings::skills::import_error().into());
                window.focus(&self.paste_input.focus_handle(cx), cx);
                cx.notify();
            }
        }
    }

    pub(super) fn apply_import(
        &mut self,
        imported: import::ImportedSkill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fm = &imported.frontmatter;
        let suggested = fm
            .name
            .clone()
            .filter(|n| validate_name(n).is_ok())
            .unwrap_or_default();
        for (input, value) in [
            (&self.name_input, suggested),
            (&self.invocation_input, fm.name.clone().unwrap_or_default()),
            (
                &self.description_input,
                fm.description.clone().unwrap_or_default(),
            ),
            (
                &self.when_to_use_input,
                fm.when_to_use.clone().unwrap_or_default(),
            ),
            (
                &self.argument_hint_input,
                fm.argument_hint.clone().unwrap_or_default(),
            ),
            (
                &self.allowed_tools_input,
                fm.allowed_tools.clone().unwrap_or_default(),
            ),
            (&self.paths_input, fm.paths.clone().unwrap_or_default()),
            (&self.model_input, fm.model.clone().unwrap_or_default()),
            (&self.body_editor, imported.body.clone()),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.user_invocable = fm.user_invocable;
        self.disable_model_invocation = fm.disable_model_invocation;
        self.imported = Some(imported);
        self.pasting = false;
        self.error = None;
        self.error_field = None;
        window.focus(&self.name_input.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn render_paste(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .tab_group()
            .gap(px(theme::FORM_MODAL_SECTION_GAP))
            .child(field_label(
                strings::skills::quick_hint(),
                theme::current(cx),
            ))
            .child(crate::ui::input(&self.paste_input, cx, 0));
        if let Some(error) = &self.error {
            body = body.child(crate::ui::alert::error("skill-import-error", error.clone()));
        }
        body.child(
            div()
                .flex()
                .justify_end()
                .gap(px(theme::MODAL_FOOTER_GAP))
                .child(
                    button("paste-cancel", strings::common::btn_cancel())
                        .tab_stop(true)
                        .tab_index(1)
                        .on_click(cx.listener(|this, _, w, cx| this.leave(w, cx))),
                )
                .child(
                    button("paste-manual", strings::common::btn_manual())
                        .tab_stop(true)
                        .tab_index(2)
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.pasting = false;
                            this.error = None;
                            w.focus(&this.body_editor.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
                .child(
                    button_primary("paste-review", strings::common::btn_review())
                        .tab_stop(true)
                        .tab_index(3)
                        .disabled(self.paste_input.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|this, _, w, cx| this.review_paste(w, cx))),
                ),
        )
        .into_any_element()
    }
}
