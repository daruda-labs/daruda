//! Files-view navigation: opening entries, the visible-row cache, and
//! keyboard / click selection over it.

use crate::workspace::main_area::file_view_pane::DiffSource;
use std::path::PathBuf;
use std::sync::Arc;

use daruda_store::project::LaneRef;
use gpui::{Context, ScrollStrategy};

use crate::files::tree::{EntryId, EntryKind};
use crate::files::watcher::DebouncedEvent;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::FileViewMode;
use crate::workspace::main_area::tab_ops::OpenIntent;

use super::{VisibleEntry, build_status_index, walk_into};

impl Workspace {
    /// Open `path` in a new tab as a `PaneContent::File` viewer (Raw by
    /// default; Markdown in Preview). Re-clicking the same `(lane, path)`
    /// reactivates the existing tab. Delegates to `open_pane_file_view`,
    /// which derives the pane's git status itself.
    ///
    /// `intent` is passed through rather than fixed here: the Files panel's
    /// own rows preview or commit, while opening a task's prompt file is
    /// navigation into that file.
    pub(in crate::workspace) fn open_files_entry(
        &mut self,
        wt_ref: LaneRef,
        path: PathBuf,
        intent: OpenIntent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_pane_file_view(
            wt_ref,
            path,
            DiffSource::WorkingTree,
            FileViewMode::Raw,
            intent,
            window,
            cx,
        );
    }

    /// Wired to `ToggleFilesFocus` — the only keyboard way into this panel,
    /// and the way back out.
    pub(in crate::workspace) fn toggle_files_focus(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.file_tree.files_panel_focus.clone();
        self.toggle_left_dock_panel_focus(
            daruda_store::project::LeftDockView::Files,
            panel,
            window,
            cx,
        );
    }

    // ------------------------------------------------------------
    // Visible-list cache
    // ------------------------------------------------------------

    /// Drop the cached visible list for `wt_ref`. Trigger sites are
    /// enumerated in the module-level doc comment.
    pub(in crate::workspace) fn invalidate_visible_files_cache(&mut self, wt_ref: LaneRef) {
        if let Some(state) = self.lane_scoped.get_mut(&wt_ref) {
            state.files.visible_cache = None;
        }
    }

    /// Return the cached `Arc<Vec<VisibleEntry>>` for `wt_ref`,
    /// rebuilding it from the live tree + git status if missing.
    pub(in crate::workspace) fn cached_or_rebuild_visible(
        &mut self,
        wt_ref: LaneRef,
    ) -> Arc<Vec<VisibleEntry>> {
        if let Some(cached) = self
            .lane_scoped
            .get(&wt_ref)
            .and_then(|state| state.files.visible_cache.as_ref())
        {
            return cached.clone();
        }
        let visible = self.build_visible_for(wt_ref);
        let arc = Arc::new(visible);
        self.lane_scoped_mut(wt_ref).files.visible_cache = Some(arc.clone());
        arc
    }

    fn build_visible_for(&self, wt_ref: LaneRef) -> Vec<VisibleEntry> {
        let Some(tree) = self.lane_file_tree(wt_ref) else {
            return Vec::new();
        };
        let status_index = match self.lane_for(wt_ref) {
            Some(lane) => build_status_index(self.lane_git_worktree(wt_ref), &lane.paths()),
            None => std::collections::HashMap::new(),
        };
        // Keyboard cursor only counts on the active lane; switching
        // lanes clears the cursor.
        let keyboard_focus = if wt_ref == self.active {
            self.file_tree.files_selection
        } else {
            None
        };
        let gitignore = if self.mirrors.files_use_gitignore {
            self.lane_scoped
                .get(&wt_ref)
                .and_then(|state| state.files.gitignore.as_ref())
        } else {
            None
        };

        let mut out = Vec::new();
        walk_into(
            tree,
            tree.root_id,
            0,
            &status_index,
            keyboard_focus,
            gitignore,
            self.mirrors.files_show_hidden,
            &mut out,
        );
        out
    }

    /// Flip `files_show_hidden`, invalidate every cached visible list,
    /// and request a render. Wired to `FilesToggleHidden`.
    pub(in crate::workspace) fn toggle_files_show_hidden(&mut self, cx: &mut Context<Self>) {
        self.mirrors.files_show_hidden = !self.mirrors.files_show_hidden;
        let refs: Vec<_> = self.lane_file_tree_refs().collect();
        for wt_ref in refs {
            self.invalidate_visible_files_cache(wt_ref);
        }
        cx.notify();
    }

    // ------------------------------------------------------------
    // Keyboard navigation
    // ------------------------------------------------------------

    /// Move the keyboard cursor by `delta` rows (positive = down) in
    /// the current visible list of the active lane. Wraps to the
    /// first / last row at boundaries.
    pub(in crate::workspace) fn move_files_selection(
        &mut self,
        delta: isize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let wt_ref = self.active_ref();
        let visible = self.cached_or_rebuild_visible(wt_ref);
        if visible.is_empty() {
            return;
        }
        let cur = self
            .file_tree
            .files_selection
            .and_then(|sel| visible.iter().position(|v| v.entry_id == sel));
        let new_index = crate::workspace::left_dock::wrap_step(cur, delta, visible.len());
        let new_id = visible[new_index].entry_id;
        if self.file_tree.files_selection != Some(new_id) {
            self.file_tree.files_selection = Some(new_id);
            self.invalidate_visible_files_cache(wt_ref);
            self.file_tree
                .files_scroll_handle
                .scroll_to_item(new_index, ScrollStrategy::Nearest);
            let panel = self.file_tree.files_panel_focus.clone();
            self.arm_left_dock_preview(panel, window, cx, |ws, window, cx| {
                ws.preview_files_selection(window, cx)
            });
            cx.notify();
        }
    }

    /// Open the cursor row's file in the viewer without entering it — the
    /// settled end of the arrow-key preview. A directory has nothing to show,
    /// so it is left to Enter / click to expand.
    fn preview_files_selection(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let Some((kind, abs)) = self.lane_file_tree(wt_ref).and_then(|tree| {
            let entry = tree.entry(sel)?;
            Some((entry.kind, tree.root.join(&entry.path)))
        }) else {
            return;
        };
        if kind.is_dir() {
            return;
        }
        self.open_files_entry(wt_ref, abs, OpenIntent::Preview, window, cx);
    }

    /// A click on a Files row. Owns the whole transition — cursor, cache,
    /// and what the click means — so the row's closure stays a one-line
    /// dispatch and the behaviour is reachable by a test without a synthetic
    /// mouse event.
    ///
    /// Double click opens in the external editor; Alt on an expanded
    /// directory collapses its whole subtree; otherwise a directory toggles
    /// and a file previews.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn on_files_row_click(
        &mut self,
        wt_ref: LaneRef,
        entry_id: EntryId,
        abs_path: PathBuf,
        kind: EntryKind,
        click_count: usize,
        alt: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // A click landed in this panel, so this panel takes keyboard focus —
        // stated once here, ahead of the branches, because only one of them
        // opens a pane and the other two would otherwise just inherit
        // whatever had focus before. The opening branch passes
        // `OpenIntent::Preview`, which is what stops the new pane undoing this.
        self.file_tree.files_panel_focus.clone().focus(window, cx);
        self.file_tree.files_selection = Some(entry_id);
        self.invalidate_visible_files_cache(wt_ref);

        if click_count >= 2 {
            if !kind.is_dir() {
                self.open_file_externally(wt_ref, abs_path, cx);
            }
            return;
        }
        if kind.is_dir() {
            if alt {
                self.collapse_files_subtree(wt_ref, entry_id, cx);
            } else {
                self.toggle_files_expand(wt_ref, entry_id, cx);
            }
            return;
        }
        self.open_files_entry(wt_ref, abs_path, OpenIntent::Preview, window, cx);
    }

    /// Enter on the cursor row: file → open in `PaneFileView::Raw`,
    /// directory → toggle expand. Mirrors single-click semantics, except for
    /// the second Enter on a row already open, which steps into the viewer —
    /// see [`Workspace::step_into_open_file_view`].
    pub(in crate::workspace) fn activate_files_selection(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // The user asked for this row now; letting the timer fire afterwards
        // would re-open the same file as a replaceable preview.
        self.left_dock_preview = None;
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let (kind, path, tree_root) = {
            let Some(tree) = self.lane_file_tree(wt_ref) else {
                return;
            };
            let Some(entry) = tree.entry(sel) else {
                return;
            };
            (entry.kind, entry.path.clone(), tree.root.clone())
        };
        if kind.is_dir() {
            self.toggle_files_expand(wt_ref, sel, cx);
        } else {
            // Absolutize like the row's click handler does: `open_pane_file_view`
            // dedupes on `fv.path`, so opening the same file by Enter and by
            // click must produce the same path or it lands in a second tab.
            let abs = tree_root.join(&path);
            if self.step_into_open_file_view(Some(abs.clone()), DiffSource::WorkingTree, window, cx)
            {
                return;
            }
            self.open_files_entry(wt_ref, abs, OpenIntent::Commit, window, cx);
        }
    }

    /// Right-arrow on the cursor row: expand it if it's a collapsed dir,
    /// else no-op.
    pub(in crate::workspace) fn expand_at_files_selection(&mut self, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let should_toggle = {
            let Some(tree) = self.lane_file_tree(wt_ref) else {
                return;
            };
            let Some(entry) = tree.entry(sel) else {
                return;
            };
            entry.kind.is_dir() && !tree.is_expanded(sel)
        };
        if should_toggle {
            self.toggle_files_expand(wt_ref, sel, cx);
        }
    }

    /// Left-arrow on the cursor row: collapse it if it's an expanded dir,
    /// else no-op.
    pub(in crate::workspace) fn collapse_at_files_selection(&mut self, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let should_toggle = {
            let Some(tree) = self.lane_file_tree(wt_ref) else {
                return;
            };
            let Some(entry) = tree.entry(sel) else {
                return;
            };
            entry.kind.is_dir() && tree.is_expanded(sel)
        };
        if should_toggle {
            self.toggle_files_expand(wt_ref, sel, cx);
        }
    }

    /// Manual root re-scan for the active lane. Used by both the
    /// dock's ⟳ button and the `FilesRefresh` action.
    pub(in crate::workspace) fn refresh_files_root(&mut self, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        // Bulk reload via the queue keeps the same serial guarantee
        // watcher-driven reloads use.
        self.queue_files_event(wt_ref, DebouncedEvent::Bulk, cx);
    }

    /// Recursive collapse: drop `entry_id` and every expanded descendant
    /// from the expanded set — cleans up a deep subtree with one
    /// Alt+click. (Recursive expand isn't supported; Alt+click on a
    /// collapsed dir falls through to the regular toggle.)
    pub(in crate::workspace) fn collapse_files_subtree(
        &mut self,
        wt_ref: LaneRef,
        entry_id: EntryId,
        cx: &mut Context<Self>,
    ) {
        let invalidated = {
            let Some(tree) = self.lane_file_tree_mut(wt_ref) else {
                return;
            };
            if !tree.is_expanded(entry_id) {
                return;
            }
            let mut victims: Vec<EntryId> = vec![entry_id];
            victims.extend(tree.descendants(entry_id));
            for v in &victims {
                if tree.is_expanded(*v) {
                    tree.toggle_expand(*v);
                }
            }
            true
        };
        if invalidated {
            self.invalidate_visible_files_cache(wt_ref);
            cx.notify();
        }
    }
}
