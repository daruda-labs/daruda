//! Workspace-level wrapper over the TerminalSession annotation mutators
//! (see `daruda_terminal::session::annotation_ops`): resolves a [`PaneId`]
//! to its `TerminalView` (the session owner — API is keyed by pane, not
//! [`LaneRef`]), routes failures through [`Workspace::report_error`], and
//! opens the [`AnnotationDialog`] for create/edit. A removed pane surfaces
//! as an Info report — a self-resolved race, not a loud error.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{Context, Pixels, Point};

use crate::surface::strings as s;
use crate::ui::{PopupMenu, PopupMenuItem};
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

    /// Copy the pane's terminal selection to the clipboard. Wired to the
    /// context-menu "Copy" entry (see [`Self::open_annotation_context_menu`]).
    pub(in crate::workspace) fn copy_pane_selection(
        &mut self,
        pane_id: PaneId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.terminal_view_for_pane(pane_id) else {
            self.report_pane_missing("copy_pane_selection", pane_id, cx);
            return;
        };
        view.update(cx, |view, cx| view.copy_selection(window, cx));
    }

    /// Open the right-click annotation context menu (Shift+Right-click, or
    /// a plain right-click when mouse reporting is off — see
    /// `TerminalView::on_mouse_down`). Leads with "Copy" (enabled only with
    /// an active selection — the most common single-click action), then a
    /// separator, then "Add annotation" (enabled only for a single-line
    /// selection, else disabled with a tooltip), then "Delete annotation"
    /// when the click landed on an existing mark.
    pub(in crate::workspace) fn open_annotation_context_menu(
        &mut self,
        pane_id: PaneId,
        position: Point<Pixels>,
        range: Option<LineRange>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let weak_ws = cx.entity().downgrade();

        let has_selection = self
            .terminal_view_for_pane(pane_id)
            .map(|view| view.read(cx).has_selection())
            .unwrap_or(false);
        // No tooltip on the disabled state — unlike "Add annotation", an
        // unavailable Copy with no selection is self-explanatory, matching
        // every other Copy menu item in this app (`ws_popup_clipboard_item`,
        // agent-chat's selection Copy item), neither of which annotates a
        // disabled Copy with a reason.
        let copy_item = if has_selection {
            let captured = pane_id;
            let weak = weak_ws.clone();
            PopupMenuItem::new(s::menu_copy()).on_click(move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(app_cx, |ws, cx| {
                        ws.copy_pane_selection(captured, window, cx);
                    });
                }
            })
        } else {
            PopupMenuItem::new(s::menu_copy()).disabled(true)
        };

        // Add entry — enabled iff the user has a single-line selection.
        let add_item = if let Some(line_range) = range {
            let captured = pane_id;
            let weak = weak_ws.clone();
            PopupMenuItem::new(s::terminal_annotation_action_add()).on_click(
                move |_, window, app_cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(app_cx, |ws, cx| {
                            ws.open_annotation_dialog_for_create(captured, line_range, window, cx);
                        });
                    }
                },
            )
        } else {
            PopupMenuItem::new(s::terminal_annotation_action_add())
                .disabled(true)
                .tooltip(s::terminal_annotation_action_add_disabled_tooltip())
        };

        let mut items = vec![copy_item, PopupMenuItem::separator(), add_item];

        // Delete entry — only when the click landed on an existing mark.
        if let Some(mark_id) = self.terminal_view_for_pane(pane_id).and_then(|view| {
            view.read(cx)
                .annotation_at_window_position(position, window)
        }) {
            let captured = pane_id;
            let weak = weak_ws;
            let delete_item = PopupMenuItem::new(s::terminal_annotation_action_delete()).on_click(
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

        // Bypasses `crate::ui::menu_builder` (declarative-path only) — apply
        // the same compact-size convention manually since this menu is built
        // imperatively via `PopupMenu::build`.
        let menu = PopupMenu::build(window, cx, move |menu, _window, _cx| {
            items.into_iter().fold(menu.small(), |m, item| m.item(item))
        });

        self.open_context_menu(position, menu, cx);
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
    fn terminal_view_for_pane(
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
