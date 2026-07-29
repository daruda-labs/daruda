//! Workspace-level wrapper over the TerminalSession annotation mutators
//! (see `daruda_terminal::session::annotation_ops`): resolves a [`PaneId`]
//! to its `TerminalView` (the session owner — API is keyed by pane, not
//! [`LaneRef`]), routes failures through [`Workspace::report_error`], and
//! opens the [`AnnotationDialog`] for create/edit. A removed pane surfaces
//! as an Info report — a self-resolved race, not a loud error.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::annotation_dialog::{AnnotationDialog, AnnotationDialogTarget};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_terminal::session::interval_tree::{LineRange, MarkId};

/// Dedup key prefix for annotation-related error reports. Lets the
/// toast layer collapse repeat failures (e.g. clicking Save twice on
/// a vanished session) into a single toast.
const ANNOTATION_ERROR_DEDUP: &str = "workspace.annotation";

impl Workspace {
    /// Create a new annotation in the pane's session.
    pub(in crate::workspace) fn add_annotation(
        &mut self,
        pane_id: PaneId,
        range: LineRange,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.terminal_view_for_pane(pane_id) else {
            self.report_pane_missing("add_annotation", pane_id, cx);
            return;
        };
        let outcome = view.update(cx, |view, cx| {
            let result = view.add_annotation(range, text);
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        if let Err(err) = outcome {
            self.report_annotation_error("add_annotation", err, cx);
        }
    }

    /// Replace an existing annotation's text.
    pub(in crate::workspace) fn update_annotation_text(
        &mut self,
        pane_id: PaneId,
        id: MarkId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.terminal_view_for_pane(pane_id) else {
            self.report_pane_missing("update_annotation_text", pane_id, cx);
            return;
        };
        let outcome = view.update(cx, |view, cx| {
            let result = view.update_annotation_text(id, text);
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        if let Err(err) = outcome {
            self.report_annotation_error("update_annotation_text", err, cx);
        }
    }

    /// Delete an annotation. Wired to the unified pane context-menu
    /// "Delete annotation" entry when the click lands on an existing mark.
    pub(in crate::workspace) fn remove_annotation(
        &mut self,
        pane_id: PaneId,
        id: MarkId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.terminal_view_for_pane(pane_id) else {
            self.report_pane_missing("remove_annotation", pane_id, cx);
            return;
        };
        let outcome = view.update(cx, |view, cx| {
            let result = view.remove_annotation(id);
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        if let Err(err) = outcome {
            self.report_annotation_error("remove_annotation", err, cx);
        }
    }

    /// Pull up the [`AnnotationDialog`] in **create** mode, pre-filled
    /// with the supplied range. Called from the context-menu "Add
    /// annotation" listener.
    pub(in crate::workspace) fn open_annotation_dialog_for_create(
        &mut self,
        pane_id: PaneId,
        range: LineRange,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        AnnotationDialog::open(
            workspace,
            AnnotationDialogTarget::Create { pane_id, range },
            String::new(),
            window,
            cx,
        );
    }

    /// Pull up the [`AnnotationDialog`] in **edit** mode for the given mark.
    /// Uses the session's O(1) `annotation_by_id` lookup so it works even
    /// when the mark is scrolled out of the viewport.
    pub(in crate::workspace) fn open_annotation_dialog_for_edit(
        &mut self,
        pane_id: PaneId,
        id: MarkId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let initial = self
            .terminal_view_for_pane(pane_id)
            .and_then(|view| {
                view.read(cx)
                    .session()
                    .annotation_by_id(id)
                    .map(|payload| payload.text.clone())
            })
            .unwrap_or_default();

        let workspace = cx.entity().downgrade();
        AnnotationDialog::open(
            workspace,
            AnnotationDialogTarget::Edit { pane_id, id },
            initial,
            window,
            cx,
        );
    }

    // ---- Internal helpers ----

    /// Find the [`Entity<TerminalView>`] backing the given pane id.
    /// Returns `None` for non-terminal panes or unknown ids.
    pub(in crate::workspace) fn terminal_view_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<gpui::Entity<daruda_terminal::view::TerminalView>> {
        let pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)?;
        match &pane.content {
            PaneContent::Terminal(t) => Some(t.view.clone()),
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => None,
        }
    }

    fn report_pane_missing(&mut self, site: &str, pane_id: PaneId, cx: &mut Context<Self>) {
        let report = ErrorReport::new(s::terminal_target_pane_missing_title())
            .severity(ErrorSeverity::Info)
            .message(s::terminal_target_pane_missing_message())
            .dedup(format!(
                "{ANNOTATION_ERROR_DEDUP}.pane_missing.{site}.{pane_id}"
            ))
            .at(file!(), line!())
            .build();
        self.report_error(report, cx);
    }

    fn report_annotation_error(
        &mut self,
        site: &str,
        err: daruda_terminal::session::AnnotationError,
        cx: &mut Context<Self>,
    ) {
        // `site` and `err` go in the dedup key, not the user-facing
        // message — debugging details must not leak into a localized toast.
        let report = ErrorReport::new(s::terminal_annotation_err_operation_failed_title())
            .severity(ErrorSeverity::Warning)
            .message(s::terminal_annotation_err_operation_failed_message())
            .dedup(format!(
                "{ANNOTATION_ERROR_DEDUP}.session_error.{site}.{err}"
            ))
            .at(file!(), line!())
            .build();
        self.report_error(report, cx);
    }
}
