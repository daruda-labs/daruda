//! Create / Edit dialog for terminal annotations, reached via the
//! Shift+Right-click context menu or a double-click on an existing overlay.
//! Mounted through [`crate::workspace::dialog_helpers::open_form_modal`], so
//! `gpui_component::Dialog` owns the chrome and this entity owns only the
//! body. Save/Cancel listeners forward to the workspace mutation methods,
//! keeping the view-purity rule.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, Render, WeakEntity,
    Window, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{InputState, WindowExt as _, button, button_primary, input};
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_form_modal;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::modal_view::ModalView;
use daruda_terminal::session::interval_tree::{LineRange, MarkId};

/// Which mutation the dialog will dispatch on Save.
#[derive(Clone, Copy, Debug)]
pub(in crate::workspace) enum AnnotationDialogTarget {
    /// Adding a brand-new annotation at the supplied single-line range.
    Create { pane_id: PaneId, range: LineRange },
    /// Editing the text of an existing annotation identified by `id`.
    Edit { pane_id: PaneId, id: MarkId },
}

/// Dialog body — text input + Save / Cancel footer. Title is owned by
/// the Dialog chrome (passed through `open_form_modal`).
pub(in crate::workspace) struct AnnotationDialog {
    panel_focus_handle: FocusHandle,
    text_input: Entity<InputState>,
    target: AnnotationDialogTarget,
    workspace: WeakEntity<Workspace>,
}

impl AnnotationDialog {
    /// Open the dialog. `initial` is the pre-populated input text —
    /// empty string for Create, the existing payload for Edit. Title
    /// is chosen from `target` so the Dialog chrome reads correctly.
    pub(in crate::workspace) fn open(
        workspace: WeakEntity<Workspace>,
        target: AnnotationDialogTarget,
        initial: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        let title = match target {
            AnnotationDialogTarget::Create { .. } => s::terminal_annotation_dialog_title_create(),
            AnnotationDialogTarget::Edit { .. } => s::terminal_annotation_dialog_title_edit(),
        };
        let initial = Rc::new(initial);
        open_form_modal(
            title,
            None,
            move |window, cx| {
                let initial = initial.clone();
                AnnotationDialog::new(workspace.clone(), target, (*initial).clone(), window, cx)
            },
            window,
            cx,
        );
    }

    fn new(
        workspace: WeakEntity<Workspace>,
        target: AnnotationDialogTarget,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = s::terminal_annotation_placeholder();
        let initial_owned: gpui::SharedString = initial.into();
        let text_input = cx.new(|cx_input| {
            let mut state = InputState::new(window, cx_input)
                .placeholder(gpui::SharedString::from(placeholder));
            if !initial_owned.is_empty() {
                state = state.default_value(initial_owned.clone());
            }
            state
        });
        Self {
            panel_focus_handle: cx.focus_handle(),
            text_input,
            target,
            workspace,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Clone an owned String for the workspace call — the dialog is
        // about to be dropped.
        let text = self.text_input.read(cx).value().to_string();
        let target = self.target;
        // Close the dialog before re-entering the workspace: the modal
        // stack is owned by Dialog/gpui_component::Root, so mutating
        // afterwards is the safer order.
        window.close_dialog(cx);

        // The target match is a single dispatch decision with nowhere else
        // to live — the only view-purity deviation from the one-liner
        // template.
        let Some(ws) = self.workspace.upgrade() else {
            return;
        };
        match target {
            AnnotationDialogTarget::Create { pane_id, range } => {
                ws.update(cx, |ws, cx| ws.add_annotation(pane_id, range, text, cx));
            }
            AnnotationDialogTarget::Edit { pane_id, id } => {
                ws.update(cx, |ws, cx| {
                    ws.update_annotation_text(pane_id, id, text, cx)
                });
            }
        }
    }
}

impl Focusable for AnnotationDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Land focus inside the text input so the user can start
        // typing immediately. `dialog_helpers::open_form_modal`
        // schedules the focus claim post-paint; returning the input's
        // handle here is what that defer reads.
        self.text_input.focus_handle(cx)
    }
}

impl ModalView for AnnotationDialog {}

impl Render for AnnotationDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("annotation-dialog-cancel", s::annotation_dialog_cancel()).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    }),
                ),
            )
            .child(
                button_primary("annotation-dialog-save", s::annotation_dialog_save()).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    }),
                ),
            );

        div()
            .flex()
            .flex_col()
            .key_context("AnnotationDialog")
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(input(&self.text_input, cx, 0_isize))
            .child(footer)
    }
}
