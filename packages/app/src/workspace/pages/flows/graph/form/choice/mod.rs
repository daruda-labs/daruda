//! A suggested-value picker backed by the form's lossless text value.

mod render;
#[cfg(test)]
mod tests;

use crate::surface::strings as s;
use crate::ui::select::{SelectEvent, SelectOption, SelectState, state_with_options};
use crate::ui::{InputEvent, InputState};
use gpui::{AppContext as _, Context, Entity, Subscription, Window};

const CUSTOM: &str = "\0custom";

pub(in crate::workspace) struct Choice {
    pub(super) input: Entity<InputState>,
    pub(super) select: Entity<SelectState>,
    options: Vec<SelectOption>,
    tab: isize,
    _subscriptions: Vec<Subscription>,
}

impl Choice {
    pub(super) fn new(
        input: Entity<InputState>,
        options: Vec<SelectOption>,
        tab: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = input.read(cx).value().trim().to_string();
        let items = with_saved(&options, &value);
        let selected = value.into();
        let select =
            cx.new(|cx| state_with_options(items, Some(&selected), window, cx).searchable(true));
        let subscriptions = vec![
            cx.subscribe_in(&select, window, |this, _, event, window, cx| {
                this.choose(event, window, cx)
            }),
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                this.edited(event, window, cx)
            }),
        ];
        Self {
            input,
            select,
            options,
            tab,
            _subscriptions: subscriptions,
        }
    }

    fn choose(
        &mut self,
        event: &crate::ui::select::ConfirmEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            if value.as_ref() == CUSTOM {
                self.input.update(cx, |input, cx| input.focus(window, cx));
            } else {
                self.input
                    .update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
            cx.notify();
        }
    }

    fn edited(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::Change) {
            self.refresh(window, cx);
        }
    }

    pub(super) fn set_options(
        &mut self,
        options: Vec<SelectOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.options = options;
        self.refresh(window, cx);
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let custom = self
            .select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.as_ref() == CUSTOM);
        let value = self.input.read(cx).value().trim().to_string();
        let items = with_saved(&self.options, &value);
        let selected = if custom { CUSTOM.to_string() } else { value }.into();
        self.select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&selected, window, cx);
            cx.notify();
        });
        cx.notify();
    }
}

fn with_saved(options: &[SelectOption], saved: &str) -> Vec<SelectOption> {
    let mut items = options.to_vec();
    if !items.iter().any(|item| item.value.as_ref() == saved) {
        items.push(SelectOption::new(
            saved.to_string(),
            s::flow::form_saved_choice(saved),
        ));
    }
    items.push(SelectOption::new(CUSTOM, s::flow::form_custom_choice()));
    items
}
