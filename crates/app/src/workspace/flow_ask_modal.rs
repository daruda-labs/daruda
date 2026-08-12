//! The permission question a flow raised, brought to the front.
//!
//! Only for the lane in view. A run in another lane must not take the
//! window away from what someone is doing — the status bar chip says it
//! is waiting, and clicking it goes there.
//!
//! Closing this is **not** an answer. The question lives on the run
//! (`RunStage::Asking`), and this is one of two views onto it; dismissing
//! leaves it in the Flows panel, which is where it can wait indefinitely
//! without a clock the user cannot see.

use gpui::{Context, FocusHandle, Focusable, IntoElement, Render, SharedString, Window, div};
use gpui::{prelude::*, px};

use crate::ui::theme;
use crate::ui::{WindowExt as _, button_danger, button_primary};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_form_modal;
use crate::workspace::flow_ops::AskRowData;

pub(in crate::workspace) struct FlowAskModal {
    lane: daruda_store::project::LaneRef,
    ask: AskRowData,
    workspace: gpui::WeakEntity<Workspace>,
    panel_focus_handle: FocusHandle,
}

impl Focusable for FlowAskModal {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.panel_focus_handle.clone()
    }
}

impl ModalView for FlowAskModal {}

impl Render for FlowAskModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_LG))
            .child(div().text_color(t.text_body).child(self.ask.tool.clone()))
            .children(self.ask.detail.clone().map(|detail| {
                div()
                    .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(detail)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(theme::GAP_SM))
                    .children(
                        self.ask
                            .options
                            .iter()
                            .enumerate()
                            .map(|(ix, choice)| self.answer_button(ix, choice, cx)),
                    ),
            )
            // Its own row, neutral, and pushed away from the answers: this
            // is not a fourth answer, and it must not read heavier than the
            // path most people take. It has to be here at all because the
            // backdrop puts the panel's Stop out of reach while a question
            // is up — without it, leaving a run you no longer want means
            // dismissing the modal first and then going to find it.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .child(self.stop_button()),
            )
    }
}

impl FlowAskModal {
    /// Raise the question for `lane`, if that is the lane in view.
    ///
    /// `active` is passed rather than read: the only caller is inside
    /// `Workspace`'s own `update`, and reading the entity from there is a
    /// re-entrant borrow that panics (root `CLAUDE.md` pitfall 5). The
    /// *rule* still lives here — one place decides what "in view" means.
    pub(in crate::workspace) fn raise_if_in_view(
        workspace: gpui::WeakEntity<Workspace>,
        active: daruda_store::project::LaneRef,
        lane: daruda_store::project::LaneRef,
        ask: AskRowData,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        if active != lane {
            return;
        }
        let weak = workspace;
        open_form_modal::<FlowAskModal, _>(
            crate::surface::strings::flow_ask_modal_title(),
            None,
            move |_window, cx_modal| FlowAskModal {
                lane,
                ask,
                workspace: weak,
                panel_focus_handle: cx_modal.focus_handle(),
            },
            window,
            cx,
        );
    }

    /// Stop the run the question belongs to, and close. Not an answer:
    /// `stop_flow_run_in` settles the question on its way out, so nothing
    /// is left waiting on a reply that is never coming.
    fn stop_button(&self) -> impl IntoElement + use<> {
        let workspace = self.workspace.clone();
        let lane = self.lane;
        crate::ui::button(
            SharedString::from(format!("flow-ask-modal-stop-{}", self.ask.ask_id)),
            SharedString::from(crate::surface::strings::flow_ask_modal_stop()),
        )
        .on_click(move |_, window, cx| {
            match workspace.update(cx, |ws, cx| ws.stop_flow_run_in(lane, cx)) {
                Ok(()) => {}
                Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                    daruda_store::observability::error_report::ErrorReport::new(
                        "Flow ask modal: workspace gone while stopping the run",
                    )
                    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup("flow.ask_modal.stop")
                    .build(),
                ),
            }
            window.close_dialog(cx);
        })
    }

    /// One answer. The same treatment the panel gives, so a person sees
    /// the same shape wherever the question reaches them.
    fn answer_button(
        &self,
        ix: usize,
        choice: &daruda_acp::PermissionChoice,
        _cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = SharedString::from(format!("flow-ask-modal-{}-{ix}", self.ask.ask_id));
        let label = SharedString::from(choice.name.clone());
        let allow = matches!(
            choice.kind,
            daruda_acp::PermissionKindView::AllowOnce | daruda_acp::PermissionKindView::AllowAlways
        );
        let button = if allow {
            button_primary(id, label)
        } else {
            button_danger(id, label)
        };

        let option_id = choice.option_id.clone();
        let workspace = self.workspace.clone();
        let (lane, ask_id) = (self.lane, self.ask.ask_id);
        button.on_click(move |_, window, cx| {
            let decision = if allow {
                daruda_acp::PermissionDecision::Allow {
                    option_id: option_id.clone(),
                }
            } else {
                daruda_acp::PermissionDecision::Reject {
                    option_id: option_id.clone(),
                }
            };
            match workspace.update(cx, |ws, cx| ws.answer_flow_ask(lane, ask_id, decision, cx)) {
                Ok(()) => {}
                Err(e) => daruda_store::observability::log_writer::LogWriter::log(
                    daruda_store::observability::error_report::ErrorReport::new(
                        "Flow ask modal: workspace gone while answering",
                    )
                    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup("flow.ask_modal.answer")
                    .build(),
                ),
            }
            window.close_dialog(cx);
        })
    }
}
