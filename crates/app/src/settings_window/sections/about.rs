//! About section of the Settings window — app version plus the
//! self-update check / download / restart controls.
//!
//! The page reads the process-wide [`crate::update::Updater`] entity
//! (registered as a global at init) and dispatches to its methods on
//! button click. The window's `_updater_subscription` observes that
//! entity, so every status transition re-renders this page reactively.
//!
//! `render_about` uses `pub(in crate::settings_window)` so the
//! `settings_window::render` dispatcher can call it, matching the
//! sibling `render_plugin` pattern.

use crate::surface::strings as s;
use crate::ui::Disableable as _;
use crate::ui::theme;
use crate::update::AutoUpdateStatus;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::{SettingsWindow, settings_button as button};

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
fn check_button(disabled: bool, cx: &mut gpui::Context<SettingsWindow>) -> impl IntoElement {
    let btn = button("settings-update-check", s::settings_button_check_updates())
        .disabled(disabled)
        .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
            if let Some(e) = crate::update::Updater::get(cx) {
                e.update(cx, |u, cx| u.check(cx));
            }
        }));
    div().flex().flex_row().child(btn)
}

/// "Update" button — download + install the available release.
fn update_button(cx: &mut gpui::Context<SettingsWindow>) -> impl IntoElement {
    let btn = button("settings-update-install", s::settings_button_update()).on_click(cx.listener(
        |_this, _: &ClickEvent, _window, cx| {
            if let Some(e) = crate::update::Updater::get(cx) {
                e.update(cx, |u, cx| u.download_and_install(cx));
            }
        },
    ));
    div().flex().flex_row().child(btn)
}

/// "Restart" button — relaunch into the swapped bundle.
fn restart_button(cx: &mut gpui::Context<SettingsWindow>) -> impl IntoElement {
    let btn = button("settings-update-restart", s::settings_button_restart()).on_click(
        cx.listener(|_this, _: &ClickEvent, _window, cx| {
            if let Some(e) = crate::update::Updater::get(cx) {
                e.update(cx, |u, cx| u.restart(cx));
            }
        }),
    );
    div().flex().flex_row().child(btn)
}

impl SettingsWindow {
    pub(in crate::settings_window) fn render_about(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Snapshot the updater state once so the mutable `cx` borrow is
        // free for the button listeners below.
        let entity = crate::update::Updater::get(cx);
        let status = entity.as_ref().map(|e| e.read(cx).status().clone());
        let can_install = entity
            .as_ref()
            .map(|e| e.read(cx).can_install())
            .unwrap_or(false);

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_about(), cx))
            .child(about_row(
                s::settings_label_current_version(),
                SharedString::from(env!("CARGO_PKG_VERSION")),
                cx,
            ));

        match status {
            None | Some(AutoUpdateStatus::Idle) => {
                col = col.child(check_button(false, cx));
            }
            Some(AutoUpdateStatus::UpToDate) => {
                col = col
                    .child(check_button(false, cx))
                    .child(muted_line(s::settings_update_up_to_date(), cx));
            }
            Some(AutoUpdateStatus::Errored(msg)) => {
                col = col
                    .child(check_button(false, cx))
                    .child(muted_line(s::settings_update_error(&msg), cx));
            }
            Some(AutoUpdateStatus::Checking) => {
                col = col
                    .child(check_button(true, cx))
                    .child(muted_line(s::settings_update_checking(), cx));
            }
            Some(AutoUpdateStatus::Available(info)) => {
                col = col
                    .child(muted_line(
                        s::settings_update_available(&info.version.to_string()),
                        cx,
                    ))
                    .child(muted_line(SharedString::from(info.notes.clone()), cx));
                // Install gate (C4): a dev / `cargo run` build has no
                // `.app` bundle to swap, so offer no Update button —
                // just an explanation.
                if can_install {
                    col = col.child(update_button(cx));
                } else {
                    col = col.child(muted_line(s::settings_update_dev_build(), cx));
                }
            }
            Some(AutoUpdateStatus::Downloading) => {
                col = col.child(muted_line(s::settings_update_downloading(), cx));
            }
            Some(AutoUpdateStatus::Installing) => {
                col = col.child(muted_line(s::settings_update_installing(), cx));
            }
            Some(AutoUpdateStatus::ReadyToRestart(_)) => {
                col = col
                    .child(muted_line(s::settings_update_ready(), cx))
                    .child(restart_button(cx));
            }
        }

        col.into_any_element()
    }
}
