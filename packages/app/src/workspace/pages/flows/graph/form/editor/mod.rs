//! An expanded prose draft; Apply updates the inspector, Cancel leaves it alone.

mod render;
use super::super::FlowGraphView;
use super::prompt_edit::{PromptEdit, PromptField};
use crate::ui::{InputState, WindowExt as _, make_markdown_prose_state, theme};
use gpui::{App, Context, Entity, FocusHandle, Focusable, Subscription, WeakEntity, Window};

pub(super) struct Editor {
    input: Entity<InputState>,
    target: WeakEntity<FlowGraphView>,
    edit: PromptEdit,
    error: Option<String>,
    _submit: Subscription,
}

impl Editor {
    fn on_input(
        &mut self,
        event: &crate::ui::InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, crate::ui::InputEvent::PressEnter { secondary: true }) {
            self.apply(window, cx);
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let applied = self.target.upgrade().is_some_and(|view| {
            view.update(cx, |view, cx| {
                view.apply_prompt_edit(&self.edit, value.to_string(), window, cx)
            })
        });
        if applied {
            window.close_dialog(cx);
        } else {
            self.error = Some(crate::surface::strings::flow::form_prompt_conflict());
            cx.notify();
        }
    }
}
impl Focusable for Editor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
impl crate::workspace::modal_view::ModalView for Editor {}

impl super::super::FlowGraphView {
    pub(in crate::workspace) fn expand_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.form else {
            return;
        };
        let value = form.body.prompt.inline.read(cx).value();
        let Some(edit) = self.begin_prompt_edit(PromptField::Inline, cx) else {
            return;
        };
        let target = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            crate::surface::strings::flow::form_prompt_label(),
            Some(gpui::px(theme::FORM_MODAL_WIDE)),
            move |window, cx| {
                let input = make_markdown_prose_state(
                    value.as_ref(),
                    "",
                    theme::TASK_EDIT_PROMPT_ROWS,
                    window,
                    cx,
                );
                let submit =
                    cx.subscribe_in(&input, window, |this: &mut Editor, _, event, window, cx| {
                        this.on_input(event, window, cx)
                    });
                Editor {
                    input,
                    target,
                    edit,
                    error: None,
                    _submit: submit,
                }
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    // The modal needs a window; the workspace authoring tests open, apply and
    // dismiss it.
}
