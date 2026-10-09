//! Widget subscriptions and value synchronization.

use super::*;

impl SettingsView {
    pub(super) fn subscribe_draft_input(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
            },
        )
    }

    pub(super) fn subscribe_sidebar_search(
        state: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(state, window, |_this, _, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
    }

    pub(super) fn subscribe_text_setting(
        state: &Entity<InputState>,
        setting: TextSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            move |this, state, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.persist_text_setting(state, setting, cx);
                }
                InputEvent::Change => {
                    if this.error.is_some() {
                        this.error = None;
                        cx.notify();
                    }
                }
                InputEvent::Focus => {}
            },
        )
    }

    pub(super) fn subscribe_select_setting(
        state: &Entity<SelectState>,
        setting: SelectSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            move |this, state, ev: &select::ConfirmEvent, _window, cx| {
                if matches!(ev, select::SelectEvent::Confirm(_)) {
                    this.persist_select_setting(state, setting, cx);
                }
            },
        )
    }

    /// Build one text setting's input from its [`spec`] row: the row supplies
    /// the placeholder, the value to seed from `config`, and the section whose
    /// tab cycle this handle joins. Call order is tab order, so the push
    /// happens here rather than at the call site.
    pub(super) fn new_text_field(
        setting: TextSetting,
        config: &daruda_config::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let row = spec::text_spec(setting);
        let state = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder((row.placeholder)())
                .default_value((row.show)(config))
        });
        subs.push(Self::subscribe_text_setting(&state, setting, window, cx));
        state
    }

    pub(super) fn set_select_value(
        select: &Entity<SelectState>,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        select.update(cx, |select, cx| {
            select.set_selected_value(&value, window, cx);
        });
    }

    /// Rebuild the orchestrator agent picker from the live catalog.
    pub(super) fn refresh_orchestrator_agent_select(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let live = crate::settings_store::SettingsStore::global(cx).user_arc();
        let options = sections::orchestrator::agent_options(&live);
        let value = sections::orchestrator::agent_select_value(&live);
        self.orchestrator_agent_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            select.set_selected_value(&value, window, cx);
        });
    }

    /// Rebuild the orchestrator account picker from live account state.
    pub(super) fn refresh_orchestrator_account_select(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let live = crate::settings_store::SettingsStore::global(cx).user_arc();
        let options = sections::orchestrator::account_options(&self.accounts.accounts);
        let value = sections::orchestrator::account_select_value(&live);
        self.orchestrator_account_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            select.set_selected_value(&value, window, cx);
        });
    }

    pub(super) fn set_font_select_value(
        select: &Entity<SelectState>,
        value: impl Into<SharedString>,
        current: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        let options = font_select_options(cx, current);
        select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            select.set_selected_value(&value, window, cx);
        });
    }

    pub(super) fn set_input_value(
        input: &Entity<InputState>,
        value: impl ToString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.to_string();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}
