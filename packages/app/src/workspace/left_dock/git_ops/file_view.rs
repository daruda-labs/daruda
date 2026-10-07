//! Pane-area file viewer — open / split / mode / scroll / load.

use std::path::PathBuf;

use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window, point, px};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, build_diff_editor_model,
};
use crate::workspace::main_area::file_view_pane::file_content::LoadOutcome;
use crate::workspace::main_area::file_view_pane::mermaid_theme;
use crate::workspace::main_area::file_view_pane::{
    DiffSource, FileViewMode, PaneFileContent, PaneFileView,
};
use crate::workspace::main_area::pane::FileContent;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::main_area::tab_ops::{OpenIntent, PaneEntry};

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
        view.lane_id == self.owner.lane
            && view.path == self.path
            && view.source == self.source
            && view.view_mode == self.mode
    }
}

/// Debug-only guard for the invariant `load_pane_file_content`'s owner-keyed
/// runtime lookup depends on: every file pane that lives in
/// `self.active_runtime_mut()` must carry an `owner`/`fv.lane_id` equal to
/// `active_lane`. A pane is *always* pushed into the active runtime
/// (`open_pane_file_view` et al. never target a parked lane's runtime), but
/// `owner` is built from a caller-supplied `lane_id` that isn't
/// type-constrained to match — `lane_ref_for_pane` in particular resolves
/// across *all* lanes by design, so a future caller that feeds its result
/// into a pane-opening path here would violate this silently. If that
/// happens, `load_pane_file_content`'s completion callback looks the pane up
/// via `runtimes.get_mut(&owner)` — a *different* runtime than the one the
/// pane was actually pushed into — never finds it, and drops the loaded
/// content: the pane sticks on "Loading" forever with no error surfaced.
/// Panics only in debug builds, matching a programming-error guard rather
/// than a recoverable runtime condition.
fn debug_assert_owner_is_active(lane_id: LaneId, active_lane: LaneId) {
    debug_assert_eq!(
        lane_id, active_lane,
        "file pane content targets lane {lane_id:?} but is pushed into the \
         active runtime for lane {active_lane:?} — load_pane_file_content's \
         owner-keyed lookup will silently miss it"
    );
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

    /// The `owner: LaneRef` for a file pane that is pushed into (or already
    /// lives in) `self.active_runtime_mut()`. Single construction site for
    /// that pairing — see [`debug_assert_owner_is_active`] for why `lane_id`
    /// must match `self.active.lane` here.
    fn owner_lane_ref(&self, lane_id: LaneId) -> LaneRef {
        debug_assert_owner_is_active(lane_id, self.active.lane);
        LaneRef {
            project: self.active.project,
            lane: lane_id,
        }
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
        lane_id: LaneId,
        path: PathBuf,
        source: DiffSource,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        debug_assert_ne!(intent, OpenIntent::Enter, "a git row never enters the pane");
        self.open_pane_file_view(
            lane_id,
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
        lane_id: LaneId,
        path: PathBuf,
        source: DiffSource,
        initial_mode: FileViewMode,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let entry = intent.pane_entry();
        let owner = self.owner_lane_ref(lane_id);
        let live_status = self.git_status_for_path(owner, &path, &source);

        // Always dedupe: clicking the same file activates its existing tab.
        if let Some((tab_idx, pane_id)) = self.find_existing_file_tab(lane_id, &path, &source) {
            // Re-stamp the tab being reused. It was opened against an older
            // the lane's cached status, and the toolbar's mode strip reads
            // `status()` to decide whether Changes is offered at all.
            if let Some(fc) = self.file_content_mut_for_pane(pane_id)
                && fc.view.live_status != live_status
            {
                fc.view.live_status = live_status;
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
        if self.file_viewer_preview_tab
            && let Some((tab_idx, pane_id)) = self.find_preview_file_tab(cx)
        {
            let project = self.active.project;
            // Replace the pane's view in place; keep its scroll handle,
            // search input, focus handle, and subscription unchanged.
            let prev_lane = self
                .active_runtime()
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .and_then(|p| p.file_view())
                .map(|fv| fv.lane_id);
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
                    lane_id,
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
            self.load_pane_file_content(load_request, cx);
            return;
        }

        // No reusable tab (or multi-tab mode): open a new tab.
        let pane = self.create_file_pane(
            lane_id,
            path.clone(),
            source,
            live_status,
            effective_mode,
            window,
            cx,
        );
        let pane_id = pane.id;
        let owner = self.owner_lane_ref(lane_id);
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
        self.invalidate_visible_files_cache(daruda_store::project::LaneRef {
            project: self.active.project,
            lane: lane_id,
        });
        cx.notify();

        self.load_pane_file_content(load_request, cx);
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
        let effective_mode = FileViewMode::effective_for_path(FileViewMode::Raw, &path);

        let owner = self.owner_lane_ref(lane_id);
        let live_status = self.git_status_for_path(owner, &path, &DiffSource::WorkingTree);
        let pane = self.create_file_pane(
            lane_id,
            path.clone(),
            DiffSource::WorkingTree,
            live_status,
            effective_mode,
            window,
            cx,
        );
        let new_pane_id = pane.id;
        let load_request = FilePaneLoadRequest::from_view(
            new_pane_id,
            owner,
            pane.file_view().expect("file pane"),
        );
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
        let project = self.active.project;
        let active_lane = self.active.lane;
        let load_request = {
            let Some(fc) = self.file_content_mut_for_pane(pane_id) else {
                return;
            };
            let Some(needs_reload) = fc.begin_mode_change(mode, Some(&mut *window), cx) else {
                return;
            };
            fc.scroll_handle = gpui::ScrollHandle::new();

            if needs_reload {
                // `fc` already borrows `self` mutably here, so this can't go
                // through `Self::owner_lane_ref` (an `&self` method) — see
                // its doc comment for what this guards against.
                debug_assert_owner_is_active(fc.view.lane_id, active_lane);
                let owner = LaneRef {
                    project,
                    lane: fc.view.lane_id,
                };
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
        // Every pending pane lives in the active lane, so the owning ref
        // is `self.active` (a file pane always references the lane it
        // lives in — `fv.lane_id == active.lane`).
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
    fn load_pane_file_content(&mut self, request: FilePaneLoadRequest, cx: &mut Context<Self>) {
        // `owner` is the lane that holds the pane *and* the lane the file
        // belongs to (a file pane always lives in the lane it references),
        // so it drives both the path/repo resolution here and the
        // pane-match scan in the completion callback below. This lets the
        // loader run for a parked lane, not just the active one — but only
        // for a pane whose `owner` was actually built by `Self::owner_lane_ref`
        // (or, in `reload_file_panes`, from the pane's own runtime key); see
        // `debug_assert_owner_is_active` for what breaks if that's not true.
        let target = request.owner;
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(target);
        // Match rendered diagrams (mermaid) to the file-viewer surface.
        // Computed here because the loader runs GPUI-free on a background
        // thread.
        let mermaid_palette = mermaid_theme::file_viewer_palette(cx);

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
