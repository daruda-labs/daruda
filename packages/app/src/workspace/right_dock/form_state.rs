//! Shared draft comparison and leave-confirmation for right-dock forms.

use crate::surface::strings;
use gpui::{App, Context, Entity, Window};

#[derive(Default)]
pub(super) struct FormState {
    pub baseline: Vec<String>,
    pub asking: bool,
}

impl FormState {
    pub fn is_dirty(&self, values: &[String]) -> bool {
        self.baseline != values
    }

    /// The first value is the paste buffer, not an already reviewed field.
    pub fn has_fields(&self, values: &[String]) -> bool {
        self.baseline.get(1..) != values.get(1..)
    }
}

pub(super) fn confirm_replace<E: 'static>(
    state: &mut FormState,
    window: &mut Window,
    cx: &mut Context<E>,
    next: impl FnOnce(&mut E, bool, &mut Window, &mut Context<E>) + 'static,
) {
    if state.asking {
        return;
    }
    state.asking = true;
    let receiver = window.prompt(
        gpui::PromptLevel::Warning,
        &strings::common::replace_form_title(),
        Some(&strings::common::replace_form_body()),
        &[
            strings::common::btn_cancel().as_str(),
            strings::common::btn_replace_fields().as_str(),
        ],
        cx,
    );
    cx.spawn_in(window, async move |this, cx| {
        let replace = matches!(receiver.await, Ok(1));
        // SILENT-OK: a closed window has no remaining draft to replace.
        let _ = this.update_in(cx, |this, window, cx| next(this, replace, window, cx));
    })
    .detach();
}

#[derive(Clone, Copy)]
pub(super) enum LeaveChoice {
    Save,
    Discard,
    Continue,
}

/// A native prompt leaves the form alive, so failed saves retain its draft.
pub(super) fn ask_to_leave<E: 'static>(
    state: &mut FormState,
    dirty: bool,
    busy: bool,
    window: &mut Window,
    cx: &mut Context<E>,
    next: impl Fn(&mut E, LeaveChoice, &mut Window, &mut Context<E>) + 'static,
) {
    if busy || state.asking {
        return;
    }
    if !dirty {
        return;
    }
    state.asking = true;
    let receiver = window.prompt(
        gpui::PromptLevel::Warning,
        &strings::common::unsaved_form_title(),
        None,
        &[
            strings::common::btn_save().as_str(),
            strings::common::btn_discard().as_str(),
            strings::common::btn_cancel().as_str(),
        ],
        cx,
    );
    cx.spawn_in(window, async move |this, cx| {
        let choice = match receiver.await {
            Ok(0) => LeaveChoice::Save,
            Ok(1) => LeaveChoice::Discard,
            _ => LeaveChoice::Continue,
        };
        // SILENT-OK: a closed window has no remaining draft to leave.
        let _ = this.update_in(cx, |this, window, cx| next(this, choice, window, cx));
    })
    .detach();
}

pub(super) fn freeze_inputs(inputs: &[Entity<crate::ui::InputState>], frozen: bool, cx: &mut App) {
    for input in inputs {
        input.update(cx, |input, cx| input.set_disabled(frozen, cx));
    }
}

/// A field keeps its validation identity when optional controls alter tab order.
pub(super) fn ordered_field_input(
    input: &Entity<crate::ui::InputState>,
    cx: &App,
    index: isize,
    tab: isize,
    error_field: Option<isize>,
    error: Option<&gpui::SharedString>,
) -> impl gpui::IntoElement {
    use gpui::{div, prelude::*};
    div()
        .flex()
        .flex_col()
        .child(crate::ui::input(input, cx, tab))
        .when(error_field == Some(index), |field| {
            field.children(error.map(|message| {
                crate::ui::alert::error(
                    gpui::SharedString::from(format!("field-error-{index}")),
                    message.clone(),
                )
            }))
        })
}
