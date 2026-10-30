//! The refusal line under a validated single-field dialog's input.

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, px,
};

use crate::ui::theme;
use crate::ui::{InputEvent, InputState};

/// Its own entity so a refused submit re-renders the line without asking the
/// dialog's Root to rebuild; the input subscription clears it on the next edit.
pub(super) struct FieldError {
    message: Option<SharedString>,
    _input: Subscription,
}

impl FieldError {
    pub(super) fn new(input: &Entity<InputState>, cx: &mut Context<Self>) -> Self {
        let _input = cx.subscribe(input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && this.message.take().is_some() {
                cx.notify();
            }
        });
        Self {
            message: None,
            _input,
        }
    }

    pub(super) fn show(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.message = Some(message);
        cx.notify();
    }
}

impl Render for FieldError {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // The spacing rides the message: an empty line must add no height, or
        // every dialog without an error grows a blank band under its field.
        let mut line = div();
        if let Some(message) = &self.message {
            line = line.pt(px(theme::MODAL_PANEL_GAP)).child(
                div().debug_selector(|| "single-modal-error".into()).child(
                    crate::ui::alert::error("single-modal-error", message.clone()),
                ),
            );
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    #[gpui::test]
    fn an_edit_clears_the_reason_and_nothing_else_does(cx: &mut gpui::TestAppContext) {
        let (window, _) = crate::workspace::tests::build_workspace(cx);
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        let (input, error) = vcx.update(|window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx));
            let error = cx.new(|cx| FieldError::new(&input, cx));
            (input, error)
        });
        vcx.update(|_, cx| error.update(cx, |e, cx| e.show("refused".into(), cx)));
        vcx.update(|_, cx| input.update(cx, |_, cx| cx.emit(InputEvent::Focus)));
        assert!(
            vcx.update(|_, cx| error.read(cx).message.is_some()),
            "focus keeps it"
        );
        vcx.update(|_, cx| input.update(cx, |_, cx| cx.emit(InputEvent::Change)));
        assert!(
            vcx.update(|_, cx| error.read(cx).message.is_none()),
            "an edit clears it"
        );
    }
}
