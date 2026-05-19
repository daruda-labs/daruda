//! Modal confirming a Close-Project action and offering an opt-in
//! "also delete worktrees on disk" mode.
//!
//! Two mutually-exclusive radios:
//!   * Remove from Daruda only (default; disk untouched).
//!   * Remove and delete worktrees on disk (`git worktree remove`
//!     each entry + `fs::remove_dir_all`).
//!
//! Routed from the `cmd-shift-w` (`CloseProject`) action and any
//! future left-dock `×` button. The disk-delete path is destructive,
//! so the safer "keep on disk" choice ships as the default. No "Don't
//! ask again" — every invocation re-asks because the cost of a wrong
//! click is high.

use std::rc::Rc;

use gpui::{
    ClickEvent, Context, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*, px,
};

use crate::ui::theme;
use crate::ui::{ActiveTheme, WindowExt as _, button, button_danger, radio};
use crate::workspace::ModalView;
use crate::workspace::dialog_helpers::open_form_modal;

/// User's pick on the delete-project chooser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeleteProjectChoice {
    /// Drop the project from Daruda's registered state. Worktree
    /// directories stay on disk.
    KeepOnDisk,
    /// `git worktree remove` every entry and remove the worktree
    /// directories. Daruda also drops its registered state once the
    /// disk cleanup finishes.
    DeleteOnDisk,
}

type DeleteProjectSubmit = Rc<dyn Fn(DeleteProjectChoice, &mut Window, &mut gpui::App)>;

pub(crate) struct DeleteProjectModal {
    panel_focus_handle: FocusHandle,
    /// Human-readable name of the project being removed — surfaced in
    /// the prompt so the user sees which project they're confirming.
    project_name: String,
    /// Currently selected radio option. Defaults to the safe
    /// `KeepOnDisk` so a careless [Delete] click does not destroy
    /// disk state.
    choice: DeleteProjectChoice,
    on_submit: DeleteProjectSubmit,
}

impl DeleteProjectModal {
    pub(crate) fn new(
        project_name: String,
        on_submit: DeleteProjectSubmit,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            panel_focus_handle: cx.focus_handle(),
            project_name,
            choice: DeleteProjectChoice::KeepOnDisk,
            on_submit,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on_submit = self.on_submit.clone();
        let choice = self.choice;
        window.close_dialog(cx);
        (on_submit)(choice, window, cx);
    }
}

impl Focusable for DeleteProjectModal {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.panel_focus_handle.clone()
    }
}

impl ModalView for DeleteProjectModal {}

impl Render for DeleteProjectModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body_color = cx.theme().muted_foreground;
        let prompt = format!("Close project \"{}\"?", self.project_name);
        let keep_checked = self.choice == DeleteProjectChoice::KeepOnDisk;
        let delete_checked = self.choice == DeleteProjectChoice::DeleteOnDisk;
        let delete_label = match self.choice {
            DeleteProjectChoice::KeepOnDisk => "Close",
            DeleteProjectChoice::DeleteOnDisk => "Delete",
        };

        let radio_group = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP / 2.0))
            .child(
                radio(
                    "delete-project-keep",
                    "Remove from Daruda only (keep worktrees on disk)",
                    0_isize,
                )
                .checked(keep_checked)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.choice = DeleteProjectChoice::KeepOnDisk;
                    cx.notify();
                })),
            )
            .child(
                radio(
                    "delete-project-disk",
                    "Remove worktrees and delete on disk",
                    1_isize,
                )
                .checked(delete_checked)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.choice = DeleteProjectChoice::DeleteOnDisk;
                    cx.notify();
                })),
            );

        let primary = match self.choice {
            DeleteProjectChoice::KeepOnDisk => button("delete-project-confirm", delete_label),
            DeleteProjectChoice::DeleteOnDisk => {
                button_danger("delete-project-confirm", delete_label)
            }
        };

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("delete-project-cancel", "Cancel").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            )
            .child(
                primary.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.submit(window, cx);
                })),
            );

        div()
            .flex()
            .flex_col()
            .key_context("DeleteProjectModal")
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(body_color)
                    .child(prompt),
            )
            .child(radio_group)
            .child(footer)
    }
}

/// Open the chooser modal. `on_submit` fires when the user clicks the
/// primary action; cancel / Esc dismisses silently.
pub(crate) fn open_delete_project_modal<F>(
    project_name: String,
    on_submit: F,
    window: &mut Window,
    cx: &mut gpui::App,
) where
    F: Fn(DeleteProjectChoice, &mut Window, &mut gpui::App) + 'static,
{
    let on_submit: DeleteProjectSubmit = Rc::new(on_submit);
    open_form_modal(
        "Close Project",
        None,
        move |_window, modal_cx| DeleteProjectModal::new(project_name, on_submit, modal_cx),
        window,
        cx,
    );
}
