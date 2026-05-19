//! Modal asking how to open a folder picked via File > Open… when
//! the workspace's `WindowOpenPolicy` is `Ask`.
//!
//! Two mutually-exclusive choices (radio): add the folder to the
//! current window vs. open it in a fresh window. A "Don't ask again"
//! checkbox folds the picked choice into the workspace's
//! [`WindowOpenPolicy`] so subsequent Open Project actions bypass
//! this modal.
//!
//! The modal owns no business logic — it just collects the user's
//! choice and hands it to [`OpenProjectChoice`]'s submit callback,
//! which threads back into [`crate::windows`] to execute the picked
//! action against the captured `path`.

use std::path::PathBuf;
use std::rc::Rc;

use daruda_store::project::WindowOpenPolicy;
use gpui::{
    ClickEvent, Context, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*, px,
};

use crate::ui::theme;
use crate::ui::{ActiveTheme, WindowExt as _, button, button_primary, checkbox, radio};
use crate::workspace::ModalView;
use crate::workspace::dialog_helpers::open_form_modal;

/// User's pick on the Open Project chooser. Mirrors the runtime
/// [`WindowOpenPolicy`] variants that the modal can produce — `Ask`
/// is not a valid outcome (cancel/escape just dismisses without
/// emitting a choice).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenProjectChoice {
    AddHere,
    NewWindow,
}

impl OpenProjectChoice {
    /// Map the modal's pick onto its persisted [`WindowOpenPolicy`]
    /// counterpart. Used when "Don't ask again" is ticked.
    #[allow(dead_code)]
    pub(crate) fn as_policy(self) -> WindowOpenPolicy {
        match self {
            Self::AddHere => WindowOpenPolicy::AddHere,
            Self::NewWindow => WindowOpenPolicy::NewWindow,
        }
    }
}

/// Submit callback shape — runs with `(choice, dont_ask, picked_path)`
/// inside the live [`gpui::App`] after the user clicks [Open]. Boxed +
/// `Rc<…>` so the modal owner doesn't need to name a concrete closure
/// type and so the modal can keep one ref while the submit handler
/// clones another for the callback fire.
type OpenProjectSubmit = Rc<dyn Fn(OpenProjectChoice, bool, PathBuf, &mut Window, &mut gpui::App)>;

pub(crate) struct OpenProjectModal {
    panel_focus_handle: FocusHandle,
    /// Absolute path of the folder the user picked from the file dialog.
    path: PathBuf,
    /// Currently selected radio option.
    choice: OpenProjectChoice,
    /// When `true`, the picked choice persists into
    /// [`crate::workspace::Workspace::set_window_open_policy`] before
    /// the modal's submit callback fires.
    dont_ask: bool,
    /// Fires with the user's pick on [Open]. Cancel / Esc dismisses
    /// without firing.
    on_submit: OpenProjectSubmit,
}

impl OpenProjectModal {
    pub(crate) fn new(
        path: PathBuf,
        initial: OpenProjectChoice,
        on_submit: OpenProjectSubmit,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            panel_focus_handle: cx.focus_handle(),
            path,
            choice: initial,
            dont_ask: false,
            on_submit,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on_submit = self.on_submit.clone();
        let choice = self.choice;
        let dont_ask = self.dont_ask;
        let path = self.path.clone();
        let wh = window.window_handle();
        window.close_dialog(cx);
        // Defer the on_submit fire — close_dialog only schedules the
        // modal entity's teardown, so calling on_submit synchronously
        // here re-enters update_window from inside a still-alive modal
        // entity's borrow and GPUI returns "window not found". Capture
        // the window handle and re-enter on the next effect cycle once
        // the modal has been dropped (G9: capture
        // `window.window_handle()` and re-enter via
        // `cx.update_window(handle, ...)`).
        cx.defer(move |app_cx| {
            crate::windows::try_update_workspace_window(
                wh,
                app_cx,
                "open_project_modal.submit",
                move |window, cx_w| {
                    (on_submit)(choice, dont_ask, path, window, cx_w);
                },
            );
        });
    }
}

impl Focusable for OpenProjectModal {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.panel_focus_handle.clone()
    }
}

impl ModalView for OpenProjectModal {}

impl Render for OpenProjectModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folder_name: String = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project")
            .to_string();
        let theme_ref = cx.theme();
        let body_color = theme_ref.muted_foreground;
        let prompt_text = format!("How should \"{folder_name}\" open?");

        let here_checked = self.choice == OpenProjectChoice::AddHere;
        let new_checked = self.choice == OpenProjectChoice::NewWindow;
        let dont_ask_checked = self.dont_ask;

        let radio_group = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP / 2.0))
            .child(
                radio("open-project-add-here", "Add to this window", 0_isize)
                    .checked(here_checked)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choice = OpenProjectChoice::AddHere;
                        cx.notify();
                    })),
            )
            .child(
                radio("open-project-new-window", "Open in new window", 1_isize)
                    .checked(new_checked)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choice = OpenProjectChoice::NewWindow;
                        cx.notify();
                    })),
            );

        let dont_ask = checkbox("open-project-dont-ask", "Don't ask again", 2_isize)
            .checked(dont_ask_checked)
            .on_click(cx.listener(|this, _, _, cx| {
                this.dont_ask = !this.dont_ask;
                cx.notify();
            }));

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("open-project-cancel", "Cancel").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            )
            .child(
                button_primary("open-project-open", "Open").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    },
                )),
            );

        div()
            .flex()
            .flex_col()
            .key_context("OpenProjectModal")
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(prompt_text),
            )
            .child(radio_group)
            .child(dont_ask)
            .child(footer)
    }
}

/// Open the chooser modal. `on_submit` runs synchronously inside the
/// caller's [`gpui::App`] right after the user clicks [Open]; cancel
/// / Esc dismisses without firing.
pub(crate) fn open_choose_window_modal<F>(
    path: PathBuf,
    initial: OpenProjectChoice,
    on_submit: F,
    window: &mut Window,
    cx: &mut gpui::App,
) where
    F: Fn(OpenProjectChoice, bool, PathBuf, &mut Window, &mut gpui::App) + 'static,
{
    let on_submit: OpenProjectSubmit = Rc::new(on_submit);
    open_form_modal(
        "Open Project",
        None,
        move |_window, modal_cx| OpenProjectModal::new(path, initial, on_submit, modal_cx),
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_maps_to_policy() {
        assert_eq!(
            OpenProjectChoice::AddHere.as_policy(),
            WindowOpenPolicy::AddHere
        );
        assert_eq!(
            OpenProjectChoice::NewWindow.as_policy(),
            WindowOpenPolicy::NewWindow
        );
    }
}
