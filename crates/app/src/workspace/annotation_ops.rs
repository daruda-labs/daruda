//! Workspace-side annotation operations (SP-1).
//!
//! The TerminalSession exposes the low-level `add_annotation` /
//! `update_annotation_text` / `remove_annotation` mutators (see
//! `daruda_terminal::session::annotation_ops`). This file is the
//! Workspace-level wrapper that:
//!
//! 1. Resolves a [`PaneId`] back to its `TerminalView` (the entity
//!    that owns the session — lanes do not own sessions, so the API
//!    is keyed by pane rather than [`LaneRef`]).
//! 2. Routes session-side failures through [`Workspace::report_error`]
//!    so the user gets a toast + an NDJSON log entry instead of a
//!    silent no-op.
//! 3. Opens the [`AnnotationDialog`] modal for the create / edit flows
//!    (the view-purity rule says the context-menu listener is a
//!    one-liner; the body lives here).
//!
//! The pane lookup walks `self.main_area.panes`; a removed pane (race
//! between the context-menu click and a `Cmd+W`) surfaces as an Info
//! report — the underlying session is gone but the user does not need
//! a loud error toast for a self-resolved race.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Pixels, Point};

use crate::surface::strings as s;
use crate::ui::ContextMenuItem;
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

    /// Delete an annotation. Wired to the context-menu "Delete annotation"
    /// entry that opens when the user Shift+Right-clicks on an existing
    /// mark (see [`Self::open_annotation_context_menu`]).
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

    /// Open the Shift+Right-click context menu for the annotation
    /// affordance. The menu always carries the "Add annotation" entry
    /// (enabled when `range` is a single-line selection, otherwise
    /// disabled with an explanatory tooltip). When the click landed on
    /// an existing annotation, a "Delete annotation" entry is appended
    /// below.
    pub(in crate::workspace) fn open_annotation_context_menu(
        &mut self,
        pane_id: PaneId,
        position: Point<Pixels>,
        range: Option<LineRange>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let weak_ws = cx.entity().downgrade();

        // Add entry — enabled iff the user has a single-line selection.
        let add_item = if let Some(line_range) = range {
            let captured = pane_id;
            let weak = weak_ws.clone();
            ContextMenuItem::new(
                s::terminal_annotation_action_add(),
                move |_, window, app_cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(app_cx, |ws, cx| {
                            ws.open_annotation_dialog_for_create(captured, line_range, window, cx);
                        });
                    }
                },
            )
        } else {
            ContextMenuItem::new(s::terminal_annotation_action_add(), |_, _, _| {})
                .disabled(true)
                .with_tooltip(s::terminal_annotation_action_add_disabled_tooltip())
        };

        let mut items = vec![add_item];

        // Delete entry — only when the click landed on an existing mark.
        if let Some(mark_id) = self.terminal_view_for_pane(pane_id).and_then(|view| {
            view.read(cx)
                .annotation_at_window_position(position, window)
        }) {
            let captured = pane_id;
            let weak = weak_ws;
            let delete_item = ContextMenuItem::new(
                s::terminal_annotation_action_delete(),
                move |_, _, app_cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(app_cx, |ws, cx| {
                            ws.remove_annotation(captured, mark_id, cx);
                        });
                    }
                },
            );
            items.push(delete_item);
        }

        self.open_context_menu(position, items, cx);
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

    /// Pull up the [`AnnotationDialog`] in **edit** mode for the
    /// supplied existing mark. Uses the session's O(1) `annotation_by_id`
    /// lookup so the dialog opens correctly even when the mark is
    /// scrolled out of the visible viewport.
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
    fn terminal_view_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<gpui::Entity<daruda_terminal::view::TerminalView>> {
        let pane = self.main_area.panes.iter().find(|p| p.id == pane_id)?;
        match &pane.content {
            PaneContent::Terminal(t) => Some(t.view.clone()),
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => None,
        }
    }

    fn report_pane_missing(&mut self, site: &str, pane_id: PaneId, cx: &mut Context<Self>) {
        let report = ErrorReport::new(s::terminal_annotation_err_pane_missing_title())
            .severity(ErrorSeverity::Info)
            .message(s::terminal_annotation_err_pane_missing_message())
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
        // `site` and `err` stay in the dedup key (internal identifier)
        // rather than the user-facing message — they are debugging
        // details that should not leak into a localized toast.
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
