//! Pane-area file viewer — open / split / mode / scroll / load.

use std::path::PathBuf;

use daruda_store::project::LaneRef;
use gpui::{Context, Window, point, px};

use crate::lane::availability::LaneAvailability;
use crate::ui::diff_editor::{DiffColors, build_diff_editor_model};
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::{
    DiffSource, FileOrigin, FileViewMode, PaneFileContent, PaneFileView,
};
use crate::workspace::main_area::pane::FileContent;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::main_area::tab_ops::{OpenIntent, PaneEntry};

mod load;

fn line_to_editor_position(line: usize) -> gpui_component::input::Position {
    let row = line.saturating_sub(1);
    gpui_component::input::Position::new(u32::try_from(row).unwrap_or(u32::MAX), 0)
}

#[derive(Clone)]
struct FilePaneLoadRequest {
    pane_id: PaneId,
    owner: LaneRef,
    path: PathBuf,
    source: DiffSource,
    mode: FileViewMode,
    live_status: Option<char>,
}

impl FilePaneLoadRequest {
    fn from_view(pane_id: PaneId, owner: LaneRef, view: &PaneFileView) -> Self {
        Self {
            pane_id,
            owner,
            path: view.path.clone(),
            source: view.source.clone(),
            mode: view.view_mode,
            live_status: view.live_status,
        }
    }

    fn matches_view(&self, view: &PaneFileView) -> bool {
        view.path == self.path && view.source == self.source && view.view_mode == self.mode
    }
}

fn title_for_file_path(path: &std::path::Path) -> gpui::SharedString {
    path.file_name()
        .map(|n| gpui::SharedString::from(n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| {
            gpui::SharedString::from(crate::surface::strings::file_viewer::untitled_tab_title())
        })
}

fn markdown_raw_line_scroll_offset(line: usize, cx: &gpui::App) -> gpui::Point<gpui::Pixels> {
    let row = line.saturating_sub(1);
    let row_h = crate::ui::theme::editor_font_size(cx) * crate::ui::theme::editor_line_height(cx);
    point(
        px(crate::ui::theme::FILE_VIEWER_SCROLL_ORIGIN_X),
        px(-(row as f32 * row_h)),
    )
}

fn apply_pending_file_viewer_scroll(
    fc: &mut FileContent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(line) = fc.view.pending_scroll_line() else {
        return;
    };

    match &fc.view.content {
        PaneFileContent::LoadedRaw { .. } | PaneFileContent::LoadedDiff { .. } => {
            fc.view.clear_pending_scroll_line();
            let editor = fc.editor_state.clone();
            editor.update(cx, |state, cx| {
                state.set_cursor_position(line_to_editor_position(line), window, cx);
            });
        }
        PaneFileContent::LoadedMarkdown { .. } => {
            apply_pending_file_viewer_scroll_without_window(fc, cx);
        }
        PaneFileContent::Loading => {}
        PaneFileContent::Error(_) | PaneFileContent::Binary | PaneFileContent::Deleted => {
            fc.view.clear_pending_scroll_line();
        }
    }
}

fn apply_pending_file_viewer_scroll_without_window(
    fc: &mut FileContent,
    cx: &mut Context<Workspace>,
) {
    let Some(line) = fc.view.pending_scroll_line() else {
        return;
    };

    match &fc.view.content {
        PaneFileContent::LoadedMarkdown { raw_rows, .. }
            if fc.view.view_mode == FileViewMode::Raw =>
        {
            let line = line.min(raw_rows.len().max(1));
            fc.view.clear_pending_scroll_line();
            fc.scroll_handle
                .set_offset(markdown_raw_line_scroll_offset(line, cx));
            cx.notify();
        }
        PaneFileContent::Loading => {}
        _ => {
            fc.view.clear_pending_scroll_line();
        }
    }
}

impl Workspace {
    fn file_content_mut_for_pane(&mut self, pane_id: PaneId) -> Option<&mut FileContent> {
        self.active_runtime_mut()
            .panes
            .iter_mut()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.file_content_mut())
    }

    /// Record whether the tab at `tab_idx` is the replaceable scratch one.
    /// A preview claims the slot; anything else releases it, which is how a
    /// file the user committed to stops being something a later skim can take.
    ///
    /// Only for the open that actually *filled* the tab — see
    /// [`Self::release_preview_tab`] for why re-activating one must not claim.
    fn mark_preview_tab(&mut self, tab_idx: usize, intent: OpenIntent) {
        if !intent.claims_preview_slot() {
            self.release_preview_tab(tab_idx);
            return;
        }
        let Some(tab_id) = self.active_runtime().tabs.get(tab_idx).map(|t| t.id) else {
            return;
        };
        self.active_runtime_mut().preview_tab_id = Some(tab_id);
    }

    /// Drop the scratch slot if `tab_idx` holds it.
    pub(super) fn release_preview_tab(&mut self, tab_idx: usize) {
        let Some(tab_id) = self.active_runtime().tabs.get(tab_idx).map(|t| t.id) else {
            return;
        };
        let rt = self.active_runtime_mut();
        if rt.preview_tab_id == Some(tab_id) {
            rt.preview_tab_id = None;
        }
    }

    /// Walking into a pane is a commit: the tab holding it stops being the
    /// scratch one. Without this a row the arrow preview opened and the user
    /// then entered stays replaceable, and the next arrow key takes the file
    /// they had walked into.
    pub(in crate::workspace) fn release_preview_tab_for_pane(&mut self, pane_id: PaneId) {
        let Some(idx) = self
            .active_runtime()
            .tabs
            .iter()
            // `pane_ids` walks the split tree, not just a one-leaf tab: a pane
            // can be split and collapsed back onto the same `TabEntry`, and the
            // slot has to be released for the tab that actually holds it.
            .position(|t| t.layout.pane_ids().contains(&pane_id))
        else {
            return;
        };
        self.release_preview_tab(idx);
    }

    /// Select a file in the Git Changes view: open the pane-area file viewer
    /// in a new tab (or activate the existing tab if the file is already open).
    ///
    /// `intent` is never [`OpenIntent::Enter`]: every way to reach this is a
    /// panel row, and a panel row that moved the user into the pane would take
    /// its own arrow keys with it. The caller still chooses between resting on
    /// the row ([`OpenIntent::Preview`]) and picking it ([`OpenIntent::Commit`]).
    pub(in crate::workspace) fn open_git_file_diff(
        &mut self,
        target: LaneRef,
        path: PathBuf,
        source: DiffSource,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        debug_assert_ne!(intent, OpenIntent::Enter, "a git row never enters the pane");
        self.open_pane_file_view(
            target,
            path,
            source,
            FileViewMode::Changes,
            intent,
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
    ///
    /// The pane's `live_status` is derived here via
    /// [`Self::git_status_for_path`], never supplied by the caller.
    ///
    /// `intent` says who ends up focused and whether the tab is replaceable;
    /// see [`OpenIntent`].
    // Every argument is an independent axis of what to open; bundling them
    // into a struct would tax all eight call sites to satisfy the lint.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn open_pane_file_view(
        &mut self,
        target: LaneRef,
        path: PathBuf,
        source: DiffSource,
        initial_mode: FileViewMode,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_pane(
            target,
            FileOrigin::Lane,
            path,
            source,
            initial_mode,
            intent,
            window,
            cx,
        );
    }

    /// Open a file an agent pointed at: in the lane that owns it, like any
    /// lane file — or, when no lane holds it, for reference in the lane on
    /// screen, read-only. An agent may name any path on disk.
    pub(in crate::workspace) fn open_linked_file(
        &mut self,
        path: PathBuf,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        match self.projects.lane_owning(&path) {
            Some(owner) => self.open_pane_file_view(
                owner,
                path,
                DiffSource::WorkingTree,
                FileViewMode::Raw,
                OpenIntent::Enter,
                window,
                cx,
            ),
            None => self.open_reference_file(path, window, cx),
        }
    }

    /// Open `path` read-only in the lane on screen, for a panel that only
    /// consults lanes (Skills, Tools). The file may live anywhere on disk,
    /// so no lane is looked up and none is moved to.
    pub(in crate::workspace) fn open_reference_file(
        &mut self,
        path: PathBuf,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file_pane(
            self.active,
            FileOrigin::Reference,
            path,
            DiffSource::WorkingTree,
            FileViewMode::Raw,
            OpenIntent::Enter,
            window,
            cx,
        );
    }

    /// The one place a file pane is opened. A pane lives in the runtime of
    /// the lane it belongs to, so a user's open of another lane's file moves
    /// there first — the pane and the lane its content is read against are
    /// then the same by construction.
    #[allow(clippy::too_many_arguments)]
    fn open_file_pane(
        &mut self,
        target: LaneRef,
        origin: FileOrigin,
        path: PathBuf,
        source: DiffSource,
        initial_mode: FileViewMode,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // A lane whose path is gone cannot show a file, and is not moved to.
        let present = |ws: &Self| {
            ws.lane_for(target)
                .filter(|lane| lane.availability == LaneAvailability::Present)
                .map(|lane| lane.path.clone())
        };
        // A pane is its lane's content only when the file is inside the lane;
        // anything else is read for reference. Decided here, once, so whether
        // a pane edits and whether it saves are one answer. A reference file
        // belongs to no lane, so it needs none — not even in an empty window.
        let origin = match origin {
            FileOrigin::Lane => {
                let Some(lane_root) = present(self) else {
                    return;
                };
                if daruda_core::path::is_within(&path, &lane_root) {
                    FileOrigin::Lane
                } else {
                    FileOrigin::Reference
                }
            }
            FileOrigin::Reference => FileOrigin::Reference,
        };
        if target != self.active {
            self.activate_lane(target, window, cx);
            // Activation re-checks the path; a lane found gone opens nothing.
            if target != self.active || present(self).is_none() {
                return;
            }
        }
        let entry = intent.pane_entry();
        let owner = target;
        let live_status = self.git_status_for_path(owner, &path, &source);

        // Always dedupe: clicking the same file activates its existing tab.
        if let Some((tab_idx, pane_id)) = self.find_existing_file_tab(&path, &source) {
            // Re-stamp the tab being reused. It was opened against an older
            // the lane's cached status, and the toolbar's mode strip reads
            // `status()` to decide whether Changes is offered at all.
            if let Some(fc) = self.file_content_mut_for_pane(pane_id)
                && (fc.view.live_status != live_status || fc.view.origin != origin)
            {
                fc.view.live_status = live_status;
                // A file first opened for reference may since have become a
                // lane's own, or the reverse.
                fc.view.origin = origin;
                cx.notify();
            }
            // Re-activating a tab that already holds this file: a deliberate
            // open releases the slot, but a preview must not claim it. Skimming
            // back across a row the user opened with Enter would otherwise make
            // it replaceable again (zed's `set_up_existing_item` likewise only
            // ever un-previews).
            if !intent.claims_preview_slot() {
                self.release_preview_tab(tab_idx);
            }
            self.activate_tab_as(tab_idx, entry, window, cx);
            return;
        }

        let effective_mode = FileViewMode::effective_for_path(initial_mode, &path);

        // Preview-tab mode: reuse the existing file-viewer tab when available.
        if self.mirrors.file_viewer_preview_tab
            && let Some((tab_idx, pane_id)) = self.find_preview_file_tab(cx)
        {
            // Replace the pane's view in place; keep its scroll handle,
            // search input, focus handle, and subscription unchanged.
            let Some(load_request) = (if let Some(pane) = self
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane_id)
                && let Some(fc) = pane.file_content_mut()
            {
                let new_title = title_for_file_path(&path);
                // The reused pane's images belong to the file it is leaving.
                fc.release_images(Some(&mut *window), cx);
                fc.view.replace_with_loading(
                    origin,
                    path.clone(),
                    source,
                    live_status,
                    effective_mode,
                );
                fc.scroll_handle = gpui::ScrollHandle::new();
                fc.cached_title = new_title;
                fc.search_input
                    .update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
                Some(FilePaneLoadRequest::from_view(pane_id, owner, &fc.view))
            } else {
                None
            }) else {
                return;
            };
            // Clear the reused tab's user label (it was set for the old file).
            if let Some(tab) = self.active_runtime_mut().tabs.get_mut(tab_idx) {
                tab.user_label = None;
            }
            self.mark_preview_tab(tab_idx, intent);
            self.activate_tab_as(tab_idx, entry, window, cx);
            if entry == PaneEntry::Enter {
                self.focus_pane(pane_id, window, cx);
            }
            // The reused pane already lived in this lane, so one lane's
            // selection is all that moved.
            self.invalidate_visible_files_cache(owner);
            cx.notify();
            self.load_pane_file_content(load_request, cx);
            return;
        }

        // No reusable tab (or multi-tab mode): open a new tab.
        let pane = self.create_file_pane(
            origin,
            path.clone(),
            source,
            live_status,
            effective_mode,
            window,
            cx,
        );
        let pane_id = pane.id;
        let load_request =
            FilePaneLoadRequest::from_view(pane_id, owner, pane.file_view().expect("file pane"));
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
        self.mark_preview_tab(last_tab, intent);
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        if entry == PaneEntry::Enter {
            self.focus_pane(pane_id, window, cx);
        }

        // Selection moved — the dock row picks up its selected background.
        self.invalidate_visible_files_cache(owner);
        cx.notify();

        self.load_pane_file_content(load_request, cx);
    }

    /// Switch a file pane between Raw / Preview / Changes mode.
    /// For a Markdown file switching between Raw and Preview, the content is
    /// already loaded in both representations — skip the reload.
    pub(in crate::workspace) fn set_file_view_mode(
        &mut self,
        mode: FileViewMode,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.active_runtime().focused_pane_id;
        self.set_file_view_mode_for_pane(pane_id, mode, window, cx);
    }

    /// `window` is only here to release the outgoing mode's GPU images — see
    /// [`crate::workspace::main_area::pane::FileContent::release_images`].
    pub(in crate::workspace) fn set_file_view_mode_for_pane(
        &mut self,
        pane_id: PaneId,
        mode: FileViewMode,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // Apply the mutation in an inner scope so the focused-pane borrow
        // releases before we call `load_pane_file_content` (which reborrows
        // self for the spawn).
        let Some(owner) = self.lane_ref_for_pane(pane_id) else {
            return;
        };
        let load_request = {
            let Some(fc) = self.file_content_mut_for_pane(pane_id) else {
                return;
            };
            let Some(needs_reload) = fc.begin_mode_change(mode, Some(&mut *window), cx) else {
                return;
            };
            fc.scroll_handle = gpui::ScrollHandle::new();

            if needs_reload {
                Some(FilePaneLoadRequest::from_view(pane_id, owner, &fc.view))
            } else {
                None
            }
        };
        cx.notify();

        if let Some(request) = load_request {
            self.load_pane_file_content(request, cx);
        }
    }

    /// Toggle whether context lines are hidden in Changes (diff) mode.
    pub(in crate::workspace) fn toggle_hide_unchanged_for_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(fc) = self.file_content_mut_for_pane(pane_id) else {
            return;
        };
        let needs_editor_rebuild = fc.view.toggle_hide_unchanged();
        let rebuild = needs_editor_rebuild.then(|| {
            let surface = crate::ui::theme::PaneSurfaceTokens::file_viewer(cx);
            cx.try_global::<crate::ui::theme::DarudaTheme>().map(|t| {
                build_diff_editor_model(
                    fc.view.active_rows(),
                    &DiffColors::from_file_viewer_surface(t, surface),
                    &crate::ui::theme::syntax_theme_of(
                        crate::ui::theme::active_syntax_palette(cx),
                        surface.syntax_is_light,
                    ),
                    true,
                )
            })
        });
        let editor = fc.editor_state.clone();
        cx.notify();
        if let Some(Some(model)) = rebuild {
            // The caller (toolbar mouse-down) already holds this window's live
            // `&mut Window` — update the editor entity directly with it rather
            // than going through `configure_file_editor`'s fresh
            // `WindowRegistry` + `cx.update_window` lookup. That lookup exists
            // for the async load-completion path, which has no live `Window`
            // in scope; calling it here, nested inside the same window's
            // active mouse-down dispatch, re-enters `cx.update_window` on a
            // window already checked out and silently fails ("window not
            // found", logged as a Warning) — the flag flips and the toolbar
            // repaints, but the editor's `set_value` never runs, so the diff
            // content never visually filters.
            editor.update(cx, move |state, cx_s| {
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
        // Through the prompting close, so an unsaved buffer is asked about.
        self.close_focused_pane(window, cx);
    }

    /// Scroll the focused file pane's body so the currently focused
    /// search match is visible. Does nothing when no match is focused,
    /// the focused pane is not a file viewer, or the viewer is in
    /// Preview mode (blocks have variable heights).
    pub(in crate::workspace) fn scroll_file_viewer_to_focused_match(&mut self, cx: &gpui::App) {
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
        let line_h =
            crate::ui::theme::editor_font_size(cx) * crate::ui::theme::editor_line_height(cx);
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

    /// Reveal a 1-based source line in the focused file viewer. If the file is
    /// still loading, store the target and apply it when the load completes.
    pub(in crate::workspace) fn scroll_focused_file_viewer_to_line(
        &mut self,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if line == 0 {
            return;
        }
        let Some(fc) = self.focused_file_content_mut() else {
            return;
        };
        fc.view.set_pending_scroll_line(line);
        apply_pending_file_viewer_scroll(fc, window, cx);
    }
}
