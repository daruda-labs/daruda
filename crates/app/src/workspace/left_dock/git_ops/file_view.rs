//! Pane-area file viewer — open / split / mode / scroll / load.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, build_diff_editor_model,
};
use crate::workspace::main_area::file_view_pane::file_content::LoadOutcome;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileContent, SelectionDrag};

impl Workspace {
    /// Select a file in the Git Changes view: open the pane-area file viewer
    /// in a new tab (or activate the existing tab if the file is already open).
    pub(in crate::workspace) fn open_git_file_diff(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        staged: bool,
        file_status: Option<char>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_pane_file_view(
            lane_id,
            path,
            staged,
            file_status,
            FileViewMode::Changes,
            window,
            cx,
        );
    }

    /// Open the pane-area file viewer for `path`.
    ///
    /// Behaviour depends on `file_viewer_preview_tab` config:
    ///
    /// **Preview-tab mode** (default, `preview_tab = true`):
    ///   - Same file already open → activate its tab (no duplicate).
    ///   - Any other file-viewer tab exists → replace its content in place
    ///     and activate it.
    ///   - No file-viewer tab → open a new tab.
    ///
    /// **Multi-tab mode** (`preview_tab = false`):
    ///   - Same file already open → activate its tab (no duplicate).
    ///   - Otherwise → always open a new tab.
    ///
    /// `initial_mode` selects Raw (Files view) or Changes (Git view);
    /// Markdown files open in Preview by default when Raw was requested.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn open_pane_file_view(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        staged: bool,
        file_status: Option<char>,
        initial_mode: FileViewMode,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // Always dedupe: clicking the same file activates its existing tab.
        if let Some((tab_idx, _pane_id)) = self.find_existing_file_tab(lane_id, &path, staged) {
            self.activate_tab(tab_idx, window, cx);
            return;
        }

        // Markdown files default to Preview when the caller requests Raw.
        let effective_mode = if initial_mode == FileViewMode::Raw {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "md" || ext == "markdown" {
                FileViewMode::Preview
            } else {
                initial_mode
            }
        } else {
            initial_mode
        };

        // Preview-tab mode: reuse the existing file-viewer tab when available.
        if self.file_viewer_preview_tab
            && let Some((tab_idx, pane_id)) = self.find_any_file_tab()
        {
            // Replace the pane's view in place; keep its scroll handle,
            // search input, focus handle, and subscription unchanged.
            let prev_lane = self
                .active_runtime()
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .and_then(|p| p.file_view())
                .map(|fv| fv.lane_id);
            if let Some(pane) = self
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane_id)
                && let Some(fc) = pane.file_content_mut()
            {
                let new_title = path
                    .file_name()
                    .map(|n| gpui::SharedString::from(n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| gpui::SharedString::from("(file)"));
                fc.view.lane_id = lane_id;
                fc.view.path = path.clone();
                fc.view.staged = staged;
                fc.view.file_status = file_status;
                fc.view.view_mode = effective_mode;
                fc.view.content = PaneFileContent::Loading;
                fc.view.hide_unchanged = false;
                fc.view.selection_drag = SelectionDrag::None;
                fc.view.search = None;
                fc.scroll_handle = gpui::ScrollHandle::new();
                fc.cached_title = new_title;
                fc.search_input
                    .update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
            }
            // Clear the reused tab's user label (it was set for the old file).
            if let Some(tab) = self.active_runtime_mut().tabs.get_mut(tab_idx) {
                tab.user_label = None;
            }
            self.activate_tab(tab_idx, window, cx);
            self.focus_pane(pane_id, window, cx);
            let project = self.active.project;
            if let Some(prev_id) = prev_lane {
                self.invalidate_visible_files_cache(daruda_store::project::LaneRef {
                    project,
                    lane: prev_id,
                });
            }
            self.invalidate_visible_files_cache(daruda_store::project::LaneRef {
                project,
                lane: lane_id,
            });
            cx.notify();
            let owner = daruda_store::project::LaneRef {
                project,
                lane: lane_id,
            };
            self.load_pane_file_content(owner, path, staged, effective_mode, file_status, cx);
            return;
        }

        // No reusable tab (or multi-tab mode): open a new tab.
        let pane = self.create_file_pane(
            lane_id,
            path.clone(),
            staged,
            file_status,
            effective_mode,
            window,
            cx,
        );
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut()
            .tabs
            .push(crate::workspace::main_area::pane::TabEntry {
                id: tab_id,
                layout: crate::workspace::main_area::pane_tree::PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);

        // Selection moved — the dock row picks up its selected background.
        self.invalidate_visible_files_cache(daruda_store::project::LaneRef {
            project: self.active.project,
            lane: lane_id,
        });
        cx.notify();

        let owner = daruda_store::project::LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        self.load_pane_file_content(owner, path, staged, effective_mode, file_status, cx);
    }

    /// Open `path` as a file viewer in a new pane split to the right of
    /// `anchor`, used for `[Diff]` conflict resolution. Unlike
    /// [`Self::open_pane_file_view`], this keeps the anchor pane (typically a
    /// `TaskEdit`) visible side-by-side for comparison, with no dedup — the
    /// split is intentionally transient. Markdown honours the Raw→Preview
    /// default. Falls back silently when `anchor` no longer exists.
    pub(in crate::workspace) fn open_file_split_right(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        anchor: crate::workspace::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Markdown defaults to Preview here too, matching `open_pane_file_view`.
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let effective_mode = if ext == "md" || ext == "markdown" {
            FileViewMode::Preview
        } else {
            FileViewMode::Raw
        };

        let pane = self.create_file_pane(
            lane_id,
            path.clone(),
            /* staged = */ false,
            /* file_status = */ None,
            effective_mode,
            window,
            cx,
        );
        let new_pane_id = pane.id;
        self.active_runtime_mut().panes.push(pane);

        // Insert the new pane to the right of `anchor` in whichever tab owns
        // it — like `split_focused_pane`, but targeting the given anchor.
        let mut inserted_tab: Option<usize> = None;
        for (idx, tab) in self.active_runtime_mut().tabs.iter_mut().enumerate() {
            if crate::workspace::main_area::pane_tree::insert_split_at(
                &mut tab.layout,
                anchor,
                crate::workspace::main_area::pane_tree::SplitDirection::Horizontal,
                new_pane_id,
                false,
            ) {
                tab.last_focused_pane = new_pane_id;
                inserted_tab = Some(idx);
                break;
            }
        }
        let Some(tab_idx) = inserted_tab else {
            // Anchor vanished. Drop the orphan pane so the workspace
            // doesn't grow a phantom file viewer with no host tab.
            self.active_runtime_mut()
                .panes
                .retain(|p| p.id != new_pane_id);
            return;
        };

        self.active_runtime_mut().active_tab_index = tab_idx;
        self.set_focused_pane(new_pane_id, window, cx);
        self.bump_activity(new_pane_id);
        self.focus_pane(new_pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        self.invalidate_visible_files_cache(daruda_store::project::LaneRef {
            project: self.active.project,
            lane: lane_id,
        });
        cx.notify();

        let owner = daruda_store::project::LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        self.load_pane_file_content(owner, path, false, effective_mode, None, cx);
    }

    /// Switch the focused file pane between Raw / Preview / Changes mode.
    /// For a Markdown file switching between Raw and Preview, the content is
    /// already loaded in both representations — skip the reload.
    pub(in crate::workspace) fn set_file_view_mode(
        &mut self,
        mode: FileViewMode,
        cx: &mut Context<Self>,
    ) {
        // Apply the mutation in an inner scope so the focused-pane borrow
        // releases before we call `load_pane_file_content` (which reborrows
        // self for the spawn).
        let load_args: Option<(LaneId, PathBuf, bool, Option<char>)> = {
            let Some(fc) = self.focused_file_content_mut() else {
                return;
            };
            let fv = &mut fc.view;
            if fv.view_mode == mode {
                return;
            }

            // Markdown: both representations are already loaded — no I/O.
            let skip_reload = matches!(&fv.content, PaneFileContent::LoadedMarkdown { .. })
                && matches!(mode, FileViewMode::Preview | FileViewMode::Raw);

            fv.view_mode = mode;
            // Clear search — match indices are mode-specific.
            fv.search = None;
            fv.selection_drag = SelectionDrag::None;
            fc.scroll_handle = gpui::ScrollHandle::new();

            if skip_reload {
                None
            } else {
                fv.content = PaneFileContent::Loading;
                Some((fv.lane_id, fv.path.clone(), fv.staged, fv.file_status))
            }
        };
        cx.notify();

        if let Some((lane_id, path, staged, file_status)) = load_args {
            let owner = daruda_store::project::LaneRef {
                project: self.active.project,
                lane: lane_id,
            };
            self.load_pane_file_content(owner, path, staged, mode, file_status, cx);
        }
    }

    /// Toggle whether context lines are hidden in Changes (diff) mode.
    pub(in crate::workspace) fn toggle_hide_unchanged(&mut self, cx: &mut Context<Self>) {
        let Some(fc) = self.focused_file_content_mut() else {
            return;
        };
        fc.view.hide_unchanged = !fc.view.hide_unchanged;
        // The active row Vec swaps between `rows_all` and `rows_no_ctx`, so
        // cached search indices point into the wrong slice — drop the search
        // alongside the other view-derived state, as `set_file_view_mode` does.
        fc.view.search = None;
        fc.view.selection_drag = SelectionDrag::None;

        // The diff renders through the editor, so rebuild its synthetic
        // buffer from the now-active row list (context lines toggled).
        let rebuild = if let PaneFileContent::LoadedDiff {
            rows_all,
            rows_no_ctx,
            ..
        } = &fc.view.content
        {
            let rows = if fc.view.hide_unchanged {
                rows_no_ctx
            } else {
                rows_all
            };
            cx.try_global::<crate::ui::theme::DarudaTheme>()
                .map(|t| build_diff_editor_model(rows, &DiffColors::from_theme(t), true))
        } else {
            None
        };
        let editor = fc.editor_state.clone();
        cx.notify();
        if let Some(model) = rebuild {
            configure_file_editor(cx, editor, move |state, window, cx_s| {
                state.set_value(model.text, window, cx_s);
                state.set_disabled(true, cx_s);
                state.set_line_decorations(model.decorations, cx_s);
                state.set_highlight_override(Some(model.highlights), cx_s);
            });
        }
    }

    /// Close the focused file pane's tab (the file viewer is its own
    /// `Pane` post-Plan-B, so closing it goes through the normal pane
    /// close path). No-op when the focused pane is a terminal.
    pub(in crate::workspace) fn close_focused_file_pane(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.active_runtime().focused_pane_id;
        let is_file = self
            .active_runtime()
            .panes
            .iter()
            .any(|p| p.id == id && p.file_content().is_some());
        if !is_file {
            return;
        }
        self.close_pane_by_id(id, window, cx);
    }

    /// Scroll the focused file pane's body so the currently focused
    /// search match is visible. Does nothing when no match is focused,
    /// the focused pane is not a file viewer, or the viewer is in
    /// Preview mode (blocks have variable heights).
    pub(in crate::workspace) fn scroll_file_viewer_to_focused_match(&mut self) {
        let Some(fc) = self.focused_file_content() else {
            return;
        };
        let fv = &fc.view;
        if fv.view_mode == FileViewMode::Preview {
            return;
        }
        let Some(row) = fv.search_focused_row() else {
            return;
        };
        let line_h = crate::ui::theme::FILE_VIEWER_LINE_H;
        let target_y = row as f32 * line_h;
        let viewport_h: f32 = fc.scroll_handle.bounds().size.height.into();
        let current_y: f32 = (-fc.scroll_handle.offset().y).into();
        let visible_bottom = current_y + viewport_h;
        if target_y < current_y || target_y + line_h > visible_bottom {
            use gpui::{Point, px};
            let origin_x = crate::ui::theme::FILE_VIEWER_SCROLL_ORIGIN_X;
            fc.scroll_handle.set_offset(Point {
                x: px(origin_x),
                y: px(-target_y),
            });
        }
    }

    /// Trigger content loads for every File pane in the active lane's
    /// `panes` whose content is still `Loading`. Called at the end of
    /// `restore_state` (for the active lane's panes only — parked lanes
    /// load when next activated) and at the end of `activate_lane`.
    /// Already-loaded panes are skipped, so re-activations are cheap.
    pub(in crate::workspace) fn load_pending_file_panes(&mut self, cx: &mut Context<Self>) {
        let active = self.active;
        let pending: Vec<(PathBuf, bool, FileViewMode, Option<char>)> = self
            .active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.file_view())
            .filter(|fv| matches!(fv.content, PaneFileContent::Loading))
            .map(|fv| (fv.path.clone(), fv.staged, fv.view_mode, fv.file_status))
            .collect();
        // Every pending pane lives in the active lane, so the owning ref
        // is `self.active` (a file pane always references the lane it
        // lives in — `fv.lane_id == active.lane`).
        for (path, staged, mode, file_status) in pending {
            self.load_pane_file_content(active, path, staged, mode, file_status, cx);
        }
    }

    /// Re-apply the current syntax palette to every open file-view pane,
    /// across *all* lanes (active and parked) so a backgrounded lane's
    /// open diff/markdown view doesn't keep stale colours after a theme
    /// or syntax-palette switch.
    ///
    /// Raw panes highlight live from `cx.theme().highlight_theme` at paint
    /// time, so a `cx.notify()` on the editor recolours them while
    /// preserving scroll position and selection. Diff and markdown panes
    /// bake their colours into spans at load time, so they need a full
    /// reload. Called when `config.file_viewer.syntax_theme` changes.
    pub(in crate::workspace) fn reload_file_panes(&mut self, cx: &mut Context<Self>) {
        // Collect first — `load_pane_file_content` reborrows `self`. Each
        // reload carries its owning `LaneRef` so the loader resolves the
        // right lane path and re-finds the pane in the right runtime.
        let mut raw_editors = Vec::new();
        let mut reloads: Vec<(LaneRef, PathBuf, bool, FileViewMode, Option<char>)> = Vec::new();
        for (lane_ref, runtime) in &self.main_area.runtimes {
            for pane in &runtime.panes {
                let Some(f) = pane.file_content() else {
                    continue;
                };
                match &f.view.content {
                    PaneFileContent::LoadedRaw => raw_editors.push(f.editor_state.clone()),
                    PaneFileContent::LoadedDiff { .. } | PaneFileContent::LoadedMarkdown { .. } => {
                        reloads.push((
                            *lane_ref,
                            f.view.path.clone(),
                            f.view.staged,
                            f.view.view_mode,
                            f.view.file_status,
                        ));
                    }
                    _ => {}
                }
            }
        }
        for editor in raw_editors {
            editor.update(cx, |_, cx| cx.notify());
        }
        for (owner, path, staged, mode, file_status) in reloads {
            self.load_pane_file_content(owner, path, staged, mode, file_status, cx);
        }
    }

    /// Spawn a background task to load file content for the given mode
    /// and update the matching file pane's `content` on completion. The
    /// pane is identified by `(lane_id, path, staged, mode)` — if
    /// no pane still matches when the load returns (because the user
    /// switched mode or closed the tab), the result is dropped.
    fn load_pane_file_content(
        &mut self,
        owner: LaneRef,
        path: PathBuf,
        staged: bool,
        mode: FileViewMode,
        file_status: Option<char>,
        cx: &mut Context<Self>,
    ) {
        // `owner` is the lane that holds the pane *and* the lane the file
        // belongs to (a file pane always lives in the lane it references),
        // so it drives both the path/repo resolution here and the
        // pane-match scan in the completion callback below. This lets the
        // loader run for a parked lane, not just the active one.
        let target = owner;
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(target);
        let syntax_theme = self.syntax_theme.clone();
        // Match rendered diagrams (mermaid) to the host appearance. Computed
        // here because the loader runs GPUI-free on a background thread.
        // Falls back to dark (the default theme) if the global is absent.
        let diagram_dark = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .unwrap_or(true);

        let path_bg = path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                crate::workspace::main_area::file_view_pane::file_content::load_file_content(
                    &wt_path,
                    repo_root.as_deref(),
                    &path_bg,
                    staged,
                    mode,
                    file_status,
                    &syntax_theme,
                    diagram_dark,
                )
            },
            move |ws, outcome, cx| {
                // Apply only if a file pane in the owning lane still
                // matches the load criteria — the user may have switched
                // modes, closed the tab, or removed the lane while the load
                // was in flight. Scan the owner's runtime directly so a
                // parked lane's pane is found (the active lane may have
                // changed since the load started).
                let pane_match = ws.main_area.runtimes.get_mut(&owner).and_then(|rt| {
                    rt.panes.iter_mut().find(|p| {
                        p.file_view().is_some_and(|fv| {
                            fv.lane_id == owner.lane
                                && fv.path == path
                                && fv.staged == staged
                                && fv.view_mode == mode
                        })
                    })
                });
                let Some(pane) = pane_match else { return };
                let Some(fc) = pane.file_content_mut() else {
                    return;
                };
                match outcome {
                    LoadOutcome::Plain(content) => {
                        // A diff renders through the shared editor too (one
                        // renderer for raw and diff): convert the rows into a
                        // synthetic buffer + decorations + injected highlight
                        // spans before moving `content`.
                        let diff_model = if let PaneFileContent::LoadedDiff { rows_all, .. } =
                            &content
                        {
                            cx.try_global::<crate::ui::theme::DarudaTheme>().map(|t| {
                                build_diff_editor_model(rows_all, &DiffColors::from_theme(t), true)
                            })
                        } else {
                            None
                        };
                        fc.view.content = content;
                        if let Some(model) = diff_model {
                            let editor = fc.editor_state.clone();
                            configure_file_editor(cx, editor, move |state, window, cx_s| {
                                state.set_value(model.text, window, cx_s);
                                state.set_disabled(true, cx_s);
                                state.set_line_decorations(model.decorations, cx_s);
                                state.set_highlight_override(Some(model.highlights), cx_s);
                            });
                        }
                    }
                    LoadOutcome::Raw { text } => {
                        // The editor entity owns the raw text from here on;
                        // feed it exactly once and clear any diff config left
                        // over from a previous mode (read-only + decorations).
                        fc.saved_text = text.clone();
                        fc.view.content = PaneFileContent::LoadedRaw;
                        let editor = fc.editor_state.clone();
                        configure_file_editor(cx, editor, move |state, window, cx_s| {
                            state.set_value(text, window, cx_s);
                            state.set_disabled(false, cx_s);
                            state.set_line_decorations(Vec::new(), cx_s);
                            state.set_highlight_override(None, cx_s);
                        });
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Open `path` from lane `lane_id` using the system default
    /// application. Runs the `open` command on a background thread so the
    /// UI thread is never blocked. Kept for a future context-menu "Open in
    /// default app" action on the Git Changes file list.
    ///
    /// `path` may be either lane-relative (Files left-dock convention) or
    /// absolute (Git Changes left-dock uses repo-root-relative paths and joins
    /// against repo_root before calling). `Path::join` returns the absolute
    /// argument unchanged, so the same code handles both cases.
    #[allow(dead_code)]
    pub(in crate::workspace) fn open_file_externally(
        &mut self,
        lane_id: daruda_store::project::LaneId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let full_path = wt.path.join(&path);
        // `open::that_detached` launches the default handler without
        // blocking on it — the prior `.status()` waited for the child
        // process to exit.
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                let result = open::that_detached(&full_path);
                (full_path, result)
            },
            |ws, (full_path, result), cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new("Failed to open file externally")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&full_path))
                        .dedup("files.open_external")
                        .build();
                    ws.report_error(report, cx);
                }
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
