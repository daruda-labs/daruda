//! Confirm-first entry points for the Settings actions that cannot be undone.
//!
//! Each button calls a `request_*` here instead of the action itself; the
//! action runs only from the dialog's OK. See
//! [`crate::workspace::dialog_helpers::confirm_destructive`] for the rule.

use gpui::{Context, Window};

use super::SettingsView;
use crate::surface::strings as s;
use crate::workspace::dialog_helpers::confirm_destructive;

impl SettingsView {
    pub(super) fn request_clear_telegram_token(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_remove_telegram_token_title(),
            s::settings_confirm_remove_token_body(),
            s::settings_confirm_ok_remove_token(),
            |this, _window, cx| this.clear_telegram_token(cx),
            window,
            cx,
        );
    }

    pub(super) fn request_unpair_telegram(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_unpair_telegram_title(),
            s::settings_confirm_unpair_body(),
            s::settings_confirm_ok_unpair(),
            |this, _window, cx| this.unpair_telegram(cx),
            window,
            cx,
        );
    }

    pub(super) fn request_remove_session_host_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_remove_session_host_title(),
            s::settings_confirm_remove_from_config_body(),
            s::settings_confirm_ok_remove(),
            move |this, _window, cx| this.remove_session_host_row(index, cx),
            window,
            cx,
        );
    }

    pub(super) fn request_remove_agent_catalog_item(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        confirm_destructive(
            cx.weak_entity(),
            s::settings_confirm_remove_agent_title(),
            s::settings_confirm_remove_from_config_body(),
            s::settings_confirm_ok_remove(),
            move |this, _window, cx| this.remove_agent_catalog_item(index, cx),
            window,
            cx,
        );
    }
}
