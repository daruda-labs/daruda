//! Background loading of file-pane content — pending loads on restore /
//! activation, theme-driven reloads, and the editor (re)configuration.

use gpui::{Context, Window};

use crate::ui::diff_editor::{DiffColors, build_diff_editor_model};
use crate::ui::mermaid_palette;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::PaneFileContent;
use crate::workspace::main_area::file_view_pane::file_content::LoadOutcome;

use super::{
    FilePaneLoadRequest, apply_pending_file_viewer_scroll_without_window, line_to_editor_position,
};

impl Workspace {
    /// Trigger content loads for every File pane in the active lane's
    /// `panes` whose content is still `Loading`. Called at the end of
    /// `restore_state` (for the active lane's panes only — parked lanes
    /// load when next activated) and at the end of `activate_lane`.
    /// Already-loaded panes are skipped, so re-activations are cheap.
    pub(in crate::workspace) fn load_pending_file_panes(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        let pending: Vec<FilePaneLoadRequest> = self
            .active_runtime()
            .panes
            .iter()
            .filter_map(|p| {
                p.file_view()
                    .filter(|fv| matches!(fv.content, PaneFileContent::Loading))
                    .map(|fv| FilePaneLoadRequest::from_view(p.id, active, fv))
            })
            .collect();
        // Every pending pane lives in the active lane, and a pane's lane is
        // the one whose runtime holds it, so the owning ref is `self.active`.
        for request in pending {
            self.load_pane_file_content(request, cx);
        }
    }

    /// Re-apply the current syntax palette to every open file-view pane,
    /// across *all* lanes (active and parked) so a backgrounded lane's
    /// open diff/markdown view doesn't keep stale colours after a theme
    /// or syntax-palette switch.
    ///
    /// Raw panes highlight live from `cx.theme().highlight_theme` at paint
    /// time, so a `cx.notify()` on the editor recolours them while
    /// preserving scroll position and selection. Rows store token buckets
    /// and markdown resolves them at paint, but a diff pane's editor model
    /// and every mermaid raster carry resolved colours, so they need a full
    /// reload. Called when `config.file_viewer.syntax_theme` changes.
    pub(in crate::workspace) fn reload_file_panes(&mut self, cx: &mut Context<Self>) {
        // Collect first — `load_pane_file_content` reborrows `self`. Each
        // reload carries its owning `LaneRef` so the loader resolves the
        // right lane path and re-finds the pane in the right runtime.
        let mut raw_editors = Vec::new();
        let mut reloads: Vec<FilePaneLoadRequest> = Vec::new();
        for (lane_ref, runtime) in &self.main_area.runtimes {
            for pane in &runtime.panes {
                let Some(f) = pane.file_content() else {
                    continue;
                };
                match &f.view.content {
                    PaneFileContent::LoadedRaw { .. } => raw_editors.push(f.editor_state.clone()),
                    PaneFileContent::LoadedDiff { .. } | PaneFileContent::LoadedMarkdown { .. } => {
                        reloads.push(FilePaneLoadRequest::from_view(pane.id, *lane_ref, &f.view));
                    }
                    _ => {}
                }
            }
        }
        for editor in raw_editors {
            editor.update(cx, |_, cx| cx.notify());
        }
        for request in reloads {
            self.load_pane_file_content(request, cx);
        }
    }

    /// Spawn a background task to load file content for the given mode
    /// and update the matching file pane's `content` on completion. The
    /// pane is identified by id and validated against `(lane_id, path, source,
    /// mode)` — if no pane still matches when the load returns (because the
    /// user switched mode or closed the tab), the result is dropped.
    pub(super) fn load_pane_file_content(
        &mut self,
        request: FilePaneLoadRequest,
        cx: &mut Context<Self>,
    ) {
        // `owner` is the lane whose runtime holds the pane, which is the lane
        // the file belongs to — every opener puts a pane there. It drives
        // both the path/repo resolution here and the pane-match scan in the
        // completion callback, so a parked lane's pane loads as well.
        let target = request.owner;
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(target);
        // Match rendered diagrams (mermaid) to the file-viewer surface.
        // Computed here because the loader runs GPUI-free on a background
        // thread.
        let mermaid_palette = mermaid_palette::file_viewer_palette(cx);

        let request_for_load = request.clone();
        let request_for_match = request;
        let path_bg = request_for_load.path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                crate::workspace::main_area::file_view_pane::file_content::load_file_content(
                    &wt_path,
                    repo_root.as_deref(),
                    &path_bg,
                    &request_for_load.source,
                    request_for_load.mode,
                    request_for_load.live_status,
                    &mermaid_palette,
                )
            },
            move |ws, outcome, cx| {
                // Apply only if a file pane in the owning lane still
                // matches the load criteria — the user may have switched
                // modes, closed the tab, or removed the lane while the load
                // was in flight. Scan the owner's runtime directly so a
                // parked lane's pane is found (the active lane may have
                // changed since the load started).
                let pane_match = ws
                    .main_area
                    .runtimes
                    .get_mut(&request_for_match.owner)
                    .and_then(|rt| {
                        rt.panes.iter_mut().find(|p| {
                            p.id == request_for_match.pane_id
                                && p.file_view()
                                    .is_some_and(|fv| request_for_match.matches_view(fv))
                        })
                    });
                let Some(pane) = pane_match else { return };
                let Some(fc) = pane.file_content_mut() else {
                    return;
                };
                match outcome {
                    LoadOutcome::Plain { content, rasters } => {
                        // A diff renders through the shared editor too (one
                        // renderer for raw and diff): convert the rows into a
                        // synthetic buffer + decorations + injected highlight
                        // spans before moving `content`.
                        let diff_model = if content.is_loaded_diff() {
                            let surface = crate::ui::theme::PaneSurfaceTokens::file_viewer(cx);
                            cx.try_global::<crate::ui::theme::DarudaTheme>().map(|t| {
                                build_diff_editor_model(
                                    fc.view.rows_for_content(&content),
                                    &DiffColors::from_file_viewer_surface(t, surface),
                                    &crate::ui::theme::syntax_theme_of(
                                        crate::ui::theme::active_syntax_palette(cx),
                                        surface.syntax_is_light,
                                    ),
                                    true,
                                )
                            })
                        } else {
                            None
                        };
                        let pending_scroll_line = fc.view.pending_scroll_line();
                        fc.install_content(content, rasters, None, cx);
                        if let Some(model) = diff_model {
                            let editor = fc.editor_state.clone();
                            configure_file_editor(cx, editor, move |state, window, cx_s| {
                                state.set_value(model.text, window, cx_s);
                                state.set_disabled(true, cx_s);
                                state.set_line_decorations(model.decorations, cx_s);
                                state.set_highlight_override(Some(model.highlights), cx_s);
                                if let Some(line) = pending_scroll_line {
                                    state.set_cursor_position(
                                        line_to_editor_position(line),
                                        window,
                                        cx_s,
                                    );
                                }
                            });
                            fc.view.clear_pending_scroll_line();
                        } else {
                            apply_pending_file_viewer_scroll_without_window(fc, cx);
                        }
                    }
                    LoadOutcome::Raw { text, truncated } => {
                        // The editor entity owns the raw text from here on;
                        // feed it exactly once and clear any diff config left
                        // over from a previous mode (read-only + decorations).
                        fc.saved_text = text.clone();
                        fc.install_content(
                            PaneFileContent::LoadedRaw { truncated },
                            Vec::new(),
                            None,
                            cx,
                        );
                        let pending_scroll_line = fc.view.take_pending_scroll_line();
                        let editor = fc.editor_state.clone();
                        configure_file_editor(cx, editor, move |state, window, cx_s| {
                            state.set_value(text, window, cx_s);
                            state.set_disabled(truncated, cx_s);
                            state.set_line_decorations(Vec::new(), cx_s);
                            state.set_highlight_override(None, cx_s);
                            if let Some(line) = pending_scroll_line {
                                state.set_cursor_position(
                                    line_to_editor_position(line),
                                    window,
                                    cx_s,
                                );
                            }
                        });
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }
}

/// Re-enter the owning workspace window to (re)configure the file pane's
/// shared editor. `set_value` and the decoration setters need a live
/// `&mut Window`, which the background-load continuation doesn't hold.
/// No-op if the window is gone (logged by `try_update_workspace_window`).
fn configure_file_editor(
    cx: &mut Context<Workspace>,
    editor: gpui::Entity<gpui_component::input::InputState>,
    apply: impl FnOnce(
        &mut gpui_component::input::InputState,
        &mut Window,
        &mut Context<gpui_component::input::InputState>,
    ) + 'static,
) {
    let entity_id = cx.entity_id();
    if let Some(wh) = crate::window_registry::WindowRegistry::handle_for_workspace(entity_id, cx) {
        crate::windows::try_update_workspace_window(
            wh,
            cx,
            "file_view.configure_editor",
            move |window, cx_w| {
                editor.update(cx_w, |state, cx_s| apply(state, window, cx_s));
            },
        );
    }
}
