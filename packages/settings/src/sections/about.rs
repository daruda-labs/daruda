//! About section of Settings — app version plus the
//! self-update check / download / restart controls.
//!
//! The page shows the [`UpdateState`] its host pushes and asks for each step
//! with [`SettingsEvent::Update`]; the updater itself is the host's.
//!
//! `render_about` uses `pub(crate)` so the
//! `settings::render` dispatcher can call it, matching the
//! sibling `render_plugin` pattern.

use crate::strings as s;
use daruda_ui::Disableable as _;
use daruda_ui::theme;
use gpui::{ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::{
    SettingsEvent, SettingsView, UpdateCommand, UpdateInstall, UpdateState,
    settings_button as button,
};

/// Two-column meta row: muted label on the left, primary value on the
/// right. Used for the current-version line.
fn about_row(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .gap(px(theme::SKILL_HEADER_GAP))
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(t.text_muted)
                .child(label.into()),
        )
        .child(
            div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(t.text_primary)
                .child(value.into()),
        )
}

/// A single muted status/notes line that wraps on long content.
fn muted_line(text: impl Into<SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .whitespace_normal()
        .child(text.into())
}

/// "Check for updates" button, wrapped in a row so it hugs its label.
/// Disabled while a check is in flight.
fn check_button(disabled: bool, cx: &mut gpui::Context<SettingsView>) -> impl IntoElement {
    let btn = button("settings-update-check", s::settings::button_check_updates())
        .disabled(disabled)
        .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
            cx.emit(SettingsEvent::Update(UpdateCommand::Check));
        }));
    div().flex().flex_row().child(btn)
}

/// "Update" button — download + install the available release.
fn update_button(cx: &mut gpui::Context<SettingsView>) -> impl IntoElement {
    let btn = button("settings-update-install", s::settings::button_update()).on_click(
        cx.listener(|_this, _: &ClickEvent, _window, cx| {
            cx.emit(SettingsEvent::Update(UpdateCommand::DownloadAndInstall));
        }),
    );
    div().flex().flex_row().child(btn)
}

/// "Restart" button — relaunch into the swapped bundle.
fn restart_button(cx: &mut gpui::Context<SettingsView>) -> impl IntoElement {
    let btn = button("settings-update-restart", s::settings::button_restart()).on_click(
        cx.listener(|_this, _: &ClickEvent, _window, cx| {
            cx.emit(SettingsEvent::Update(UpdateCommand::Restart));
        }),
    );
    div().flex().flex_row().child(btn)
}

impl SettingsView {
    /// The version line and the updater's current state, as one block.
    pub(crate) fn about_version(&self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let state = self.update_state.clone();

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::SETTINGS_CARD_PAD))
            .bg(theme::current(cx).dock_bg)
            .border_1()
            .border_color(theme::current(cx).border)
            .rounded(px(theme::RADIUS_SM))
            .child(about_row(
                s::settings::label_current_version(),
                SharedString::from(env!("CARGO_PKG_VERSION")),
                cx,
            ));

        match state {
            UpdateState::Idle => {
                col = col.child(check_button(false, cx));
            }
            UpdateState::UpToDate => {
                col = col
                    .child(check_button(false, cx))
                    .child(muted_line(s::settings::update_up_to_date(), cx));
            }
            UpdateState::Errored(msg) => {
                col = col
                    .child(check_button(false, cx))
                    .child(muted_line(s::settings::update_error(&msg), cx));
            }
            UpdateState::Checking => {
                col = col
                    .child(check_button(true, cx))
                    .child(muted_line(s::settings::update_checking(), cx));
            }
            UpdateState::Available {
                version,
                notes,
                install,
            } => {
                col = col
                    .child(muted_line(s::settings::update_available(version), cx))
                    .child(muted_line(SharedString::from(notes), cx));
                col = match install {
                    UpdateInstall::InPlace => col.child(update_button(cx)),
                    UpdateInstall::ManualPackage => {
                        col.child(muted_line(s::settings::update_manual_package(), cx))
                    }
                    UpdateInstall::Unpackaged => {
                        col.child(muted_line(s::settings::update_dev_build(), cx))
                    }
                };
            }
            UpdateState::Downloading => {
                col = col.child(muted_line(s::settings::update_downloading(), cx));
            }
            UpdateState::Installing => {
                col = col.child(muted_line(s::settings::update_installing(), cx));
            }
            UpdateState::ReadyToRestart { uses_installer } => {
                let message = if uses_installer {
                    s::settings::update_ready_installer()
                } else {
                    s::settings::update_ready()
                };
                col = col.child(muted_line(message, cx)).child(restart_button(cx));
            }
        }

        col
    }
}
