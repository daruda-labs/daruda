//! Background git-status / diff / commit / push operations for the
//! Git Changes sidebar view.
//!
//! All git CLI calls run on `background_executor` so the UI thread never
//! blocks. State mutations come back via `cx.spawn(|this, cx| ...)` where
//! `this` is `WeakEntity<Workspace>` auto-injected by `Context::spawn`.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::WorktreeId;
use gpui::{AppContext as _, Context, Window};

use crate::path_ext::PathExt;
use crate::surface::strings as app_strings;
use crate::ui::ButtonVariant;
use crate::workspace::dialog_helpers::open_confirm_dialog;

use super::file_viewer::{FileViewMode, PaneFileContent};
use super::{CommitChanges, PushChanges, Workspace};

// ----------------------------------------------------------------
// Shared git status color helper
// ----------------------------------------------------------------

/// Map a git status character to a single-letter display symbol. Lifted
/// from the old `git_changes::status_display` so the sidebar Files
/// view (W-7f) can show the same badge with one source of truth.
pub(in crate::workspace) fn git_status_symbol(ch: char) -> &'static str {
    match ch {
        'M' => "M",
        // Untracked files render as additions in the sidebar.
        'A' | '?' => "A",
        'D' => "D",
        'R' => "R",
        'C' => "C",
        'U' => "U",
        _ => "·",
    }
}

/// Map a git status character and staged flag to a display colour.
/// Used by both the sidebar file list and the pane-area file viewer toolbar.
/// Reads from the live `DarudaTheme` Global so colours flip on
/// light-mode switch.
pub(in crate::workspace) fn git_status_color(ch: char, staged: bool, cx: &gpui::App) -> gpui::Hsla {
    use crate::ui::theme;
    let t = theme::current(cx);
    match ch {
        'M' | 'D' => {
            if staged {
                t.git_staged_color
            } else {
                t.git_unstaged_color
            }
        }
        'A' | 'R' | 'C' => t.git_staged_color,
        '?' => t.git_untracked_color,
        'U' => t.git_unstaged_color,
        _ => t.faint_text,
    }
}

// ----------------------------------------------------------------
// impl Workspace — git operations
// ----------------------------------------------------------------

impl Workspace {
    /// Repo root of worktree `id`, or `None` when it isn't git-backed.
    pub(in crate::workspace) fn git_repo_root_for(&self, id: WorktreeId) -> Option<PathBuf> {
        self.worktrees
            .iter()
            .find(|w| w.id == id)
            .and_then(|w| match &w.kind {
                daruda_store::project::WorktreeKind::Git { repo_root, .. } => {
                    Some(repo_root.clone())
                }
                daruda_store::project::WorktreeKind::Default => None,
            })
    }

    /// Kick off a background `git status` for `worktree_id` and update
    /// `git_status_cache` when done. No-op for non-git worktrees.
    ///
    /// Concurrency guard: at most one in-flight task per worktree. A
    /// second call while one is running sets `git_status_pending_repeat`,
    /// which the in-flight task drains by re-invoking itself once
    /// before returning. This collapses watcher-event bursts (a `cargo
    /// build` can emit 30+ debounced events per second) into at most
    /// two `git status` invocations: the running one + one repeat that
    /// captures everything that landed during the run.
    pub(in crate::workspace) fn refresh_git_status(
        &mut self,
        worktree_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let path = wt.path.clone();
        if !wt.is_git() {
            return;
        }

        if !self.git_status_in_flight.insert(worktree_id) {
            // Already running — request a re-fire on completion.
            self.git_status_pending_repeat.insert(worktree_id);
            return;
        }

        let path_for_report = path.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_status(&path),
            move |ws, result, cx| {
                ws.git_status_in_flight.remove(&worktree_id);
                match result {
                    Ok(data) => {
                        ws.git_status_cache.insert(worktree_id, data);
                        // Trigger #6 — git status refresh updates badges.
                        ws.invalidate_visible_files_cache(worktree_id);
                        // Commit button reflects the active worktree's
                        // staged count — recompute when that worktree's
                        // cache changes.
                        if worktree_id == ws.active_worktree_id {
                            ws.sync_commit_buttons(cx);
                        }
                    }
                    Err(e) => {
                        // `git status` failure disables the entire Git Changes
                        // panel — staged/unstaged lists, commit footer, file
                        // badges all go stale. That meets the CLAUDE.md
                        // "core function broke" bar for Error severity.
                        let report = ErrorReport::new("git status failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.status")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
                // Drain the repeat slot — re-fire once to capture
                // events that landed while the previous run was busy.
                if ws.git_status_pending_repeat.remove(&worktree_id) {
                    ws.refresh_git_status(worktree_id, cx);
                }
            },
        )
        .detach();
    }

    /// Refresh git status for the merge target worktree after a successful merge.
    pub(in crate::workspace) fn finalize_merge(
        &mut self,
        target_wt_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        self.refresh_git_status(target_wt_id, cx);
    }

    /// Select a file in the Git Changes view: open the pane-area file viewer
    /// in a new tab (or activate the existing tab if the file is already open).
    pub(in crate::workspace) fn open_git_file_diff(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        staged: bool,
        file_status: Option<char>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_pane_file_view(
            worktree_id,
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
        worktree_id: WorktreeId,
        path: PathBuf,
        staged: bool,
        file_status: Option<char>,
        initial_mode: FileViewMode,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        // Always dedupe: clicking the same file activates its existing tab.
        if let Some((tab_idx, _pane_id)) = self.find_existing_file_tab(worktree_id, &path, staged) {
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
            let prev_worktree = self
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .and_then(|p| p.file_view())
                .map(|fv| fv.worktree_id);
            if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id)
                && let Some(fc) = pane.file_content_mut()
            {
                let new_title = path
                    .file_name()
                    .map(|n| gpui::SharedString::from(n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| gpui::SharedString::from("(file)"));
                fc.view.worktree_id = worktree_id;
                fc.view.path = path.clone();
                fc.view.staged = staged;
                fc.view.file_status = file_status;
                fc.view.view_mode = effective_mode;
                fc.view.content = PaneFileContent::Loading;
                fc.view.hide_unchanged = false;
                fc.view.char_selection = None;
                fc.view.char_anchor = None;
                fc.view.is_drag_selecting = false;
                fc.view.search = None;
                fc.scroll_handle = gpui::ScrollHandle::new();
                fc.cached_title = new_title;
                fc.search_input
                    .update(cx, |inp, cx_state| inp.set_value("", window, cx_state));
            }
            // Clear the reused tab's user label (it was set for the old file).
            if let Some(tab) = self.tabs.get_mut(tab_idx) {
                tab.user_label = None;
            }
            self.activate_tab(tab_idx, window, cx);
            self.focus_pane(pane_id, window, cx);
            if let Some(prev_id) = prev_worktree {
                self.invalidate_visible_files_cache(prev_id);
            }
            self.invalidate_visible_files_cache(worktree_id);
            cx.notify();
            self.load_pane_file_content(worktree_id, path, staged, effective_mode, file_status, cx);
            return;
        }

        // No reusable tab (or multi-tab mode): open a new tab.
        let pane = self.create_file_pane(
            worktree_id,
            path.clone(),
            staged,
            file_status,
            effective_mode,
            window,
            cx,
        );
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.panes.push(pane);
        self.tabs.push(super::pane::TabEntry {
            id: tab_id,
            layout: super::layout::PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        self.tab_history.push(self.active_tab_index);
        self.active_tab_index = self.tabs.len() - 1;
        self.focused_pane_id = pane_id;
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);

        // Trigger #4 — selection moved (sidebar row gets selected BG).
        self.invalidate_visible_files_cache(worktree_id);
        cx.notify();

        self.load_pane_file_content(worktree_id, path, staged, effective_mode, file_status, cx);
    }

    /// Open `path` as a file viewer in a new pane *split to the right
    /// of* `anchor` (R-20 `[Diff]` conflict resolution).
    ///
    /// Unlike [`Self::open_pane_file_view`] — which adds the file in a
    /// fresh tab — this variant keeps the anchor pane (typically a
    /// `TaskEdit`) visible side-by-side with the disk version, so the
    /// user can compare without tab-switching. No dedup against
    /// existing file tabs — the split is intentionally transient.
    /// Markdown files honour the same Raw→Preview default.
    ///
    /// Falls back silently when `anchor` no longer exists; the caller
    /// (`open_disk_file_for_diff`) already routed through a stale
    /// `pane_id` would otherwise crash on the layout walk.
    pub(in crate::workspace) fn open_file_split_right(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        anchor: super::layout::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Markdown defaults to Preview here too — keeps the visual
        // language consistent with `open_pane_file_view`.
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
            worktree_id,
            path.clone(),
            /* staged = */ false,
            /* file_status = */ None,
            effective_mode,
            window,
            cx,
        );
        let new_pane_id = pane.id;
        self.panes.push(pane);

        // Insert the new pane to the right of `anchor` in whichever
        // tab owns it. Mirrors `split_focused_pane` but targets a
        // caller-supplied anchor instead of `focused_pane_id`.
        let mut inserted_tab: Option<usize> = None;
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            if super::layout::insert_split_at(
                &mut tab.layout,
                anchor,
                super::layout::SplitDirection::Horizontal,
                new_pane_id,
            ) {
                tab.last_focused_pane = new_pane_id;
                inserted_tab = Some(idx);
                break;
            }
        }
        let Some(tab_idx) = inserted_tab else {
            // Anchor vanished. Drop the orphan pane so the workspace
            // doesn't grow a phantom file viewer with no host tab.
            self.panes.retain(|p| p.id != new_pane_id);
            return;
        };

        self.active_tab_index = tab_idx;
        self.focused_pane_id = new_pane_id;
        self.bump_activity(new_pane_id);
        self.focus_pane(new_pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        self.invalidate_visible_files_cache(worktree_id);
        cx.notify();

        self.load_pane_file_content(worktree_id, path, false, effective_mode, None, cx);
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
        let load_args: Option<(WorktreeId, PathBuf, bool, Option<char>)> = {
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
            fv.char_selection = None;
            fv.char_anchor = None;
            fv.is_drag_selecting = false;
            fc.scroll_handle = gpui::ScrollHandle::new();

            if skip_reload {
                None
            } else {
                fv.content = PaneFileContent::Loading;
                Some((fv.worktree_id, fv.path.clone(), fv.staged, fv.file_status))
            }
        };
        cx.notify();

        if let Some((worktree_id, path, staged, file_status)) = load_args {
            self.load_pane_file_content(worktree_id, path, staged, mode, file_status, cx);
        }
    }

    /// Toggle whether context lines are hidden in Changes (diff) mode.
    pub(in crate::workspace) fn toggle_hide_unchanged(&mut self, cx: &mut Context<Self>) {
        let Some(fv) = self.focused_file_view_mut() else {
            return;
        };
        fv.hide_unchanged = !fv.hide_unchanged;
        // The active row Vec swaps between `rows_all` and `rows_no_ctx`
        // here, so any cached search `matches: Vec<usize>` / `focused`
        // now index into the wrong slice. Mirror `set_file_view_mode`
        // and drop the search alongside the other view-derived state.
        fv.search = None;
        fv.char_selection = None;
        fv.char_anchor = None;
        fv.is_drag_selecting = false;
        cx.notify();
    }

    /// Close the focused file pane's tab (the file viewer is its own
    /// `Pane` post-Plan-B, so closing it goes through the normal pane
    /// close path). No-op when the focused pane is a terminal.
    pub(in crate::workspace) fn close_focused_file_pane(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.focused_pane_id;
        let is_file = self
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

    /// Trigger content loads for every File pane in the live `panes`
    /// vec whose content is still `Loading`. Called at the end of
    /// `restore_state` (for the active worktree's panes only — others
    /// live in `inactive_worktree_runtimes` and load when their
    /// worktree is next activated) and at the end of `activate_worktree`.
    /// Already-loaded panes are skipped, so re-activations are cheap.
    pub(in crate::workspace) fn load_pending_file_panes(&mut self, cx: &mut Context<Self>) {
        let pending: Vec<(WorktreeId, PathBuf, bool, FileViewMode, Option<char>)> = self
            .panes
            .iter()
            .filter_map(|p| p.file_view())
            .filter(|fv| matches!(fv.content, PaneFileContent::Loading))
            .map(|fv| {
                (
                    fv.worktree_id,
                    fv.path.clone(),
                    fv.staged,
                    fv.view_mode,
                    fv.file_status,
                )
            })
            .collect();
        for (worktree_id, path, staged, mode, file_status) in pending {
            self.load_pane_file_content(worktree_id, path, staged, mode, file_status, cx);
        }
    }

    /// Spawn a background task to load file content for the given mode
    /// and update the matching file pane's `content` on completion. The
    /// pane is identified by `(worktree_id, path, staged, mode)` — if
    /// no pane still matches when the load returns (because the user
    /// switched mode or closed the tab), the result is dropped.
    fn load_pane_file_content(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        staged: bool,
        mode: FileViewMode,
        file_status: Option<char>,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(worktree_id);
        let syntax_theme = self.syntax_theme.clone();

        let path_bg = path.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                super::file_content::load_file_content(
                    &wt_path,
                    repo_root.as_deref(),
                    &path_bg,
                    staged,
                    mode,
                    file_status,
                    &syntax_theme,
                )
            },
            move |ws, content, cx| {
                // Apply only if a file pane still matches the load
                // criteria — the user may have switched modes or
                // closed the tab while the load was in flight.
                let pane_match = ws.panes.iter_mut().find(|p| {
                    p.file_view().is_some_and(|fv| {
                        fv.worktree_id == worktree_id
                            && fv.path == path
                            && fv.staged == staged
                            && fv.view_mode == mode
                    })
                });
                if let Some(pane) = pane_match
                    && let Some(fv) = pane.file_view_mut()
                {
                    fv.content = content;
                    cx.notify();
                }
            },
        )
        .detach();
    }

    /// Stage a single file from the working tree into the index.
    ///
    /// Runs from the worktree's git toplevel so:
    /// (a) linked worktrees stage into their own per-worktree index
    ///     rather than the shared `repo_root` (which `git_repo_root_for`
    ///     returns and which is wrong for any non-primary worktree); and
    /// (b) an anchored main worktree (where `wt.path` points at a
    ///     subdirectory the user opened) still gets the porcelain-path
    ///     base correct — `git status --porcelain` paths are
    ///     toplevel-relative, so `git add` must run from there.
    pub(in crate::workspace) fn stage_file(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_add(&wt_top, &path),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git add failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("worktree", redact_home(&wt_for_report))
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.stage")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Remove a file from the index (unstage), keeping working-tree changes.
    ///
    /// Runs from the worktree's git toplevel — see [`stage_file`] for
    /// why `wt.path` and the shared `repo_root` are both unsuitable.
    pub(in crate::workspace) fn unstage_file(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_restore_staged(&wt_top, &path),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git restore --staged failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("worktree", redact_home(&wt_for_report))
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.unstage")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Stage every path in `paths` in one git invocation. Used by the
    /// per-directory "stage all in this dir" checkbox.
    pub(in crate::workspace) fn stage_paths(
        &mut self,
        worktree_id: WorktreeId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight || paths.is_empty() {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_add_paths(&wt_top, &paths),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git add (paths) failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("worktree", redact_home(&wt_for_report))
                            .with_context("count", paths_count.to_string())
                            .dedup("git.stage_paths")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Unstage every path in `paths` in one git invocation. Companion to
    /// [`stage_paths`] for the per-dir "unstage all" toggle.
    pub(in crate::workspace) fn unstage_paths(
        &mut self,
        worktree_id: WorktreeId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight || paths.is_empty() {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_restore_staged_paths(&wt_top, &paths),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git restore --staged (paths) failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("worktree", redact_home(&wt_for_report))
                            .with_context("count", paths_count.to_string())
                            .dedup("git.unstage_paths")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Stage all unstaged and untracked files (`git add --all`).
    pub(in crate::workspace) fn stage_all(
        &mut self,
        worktree_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_top.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_add_all(&wt_top),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git add --all failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.stage_all")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Recompute and push the disabled state of the commit footer's
    /// split-button.
    ///
    /// The `commit` split-button is disabled when an op is in flight or
    /// no files are staged. Committing with nothing staged would just
    /// fail at the git CLI, so the UI surfaces the "no-op" state
    /// up-front. The Amend item inside the dropdown shares the same
    /// disabled state because `DropdownButton` ties the caret to the
    /// primary button's enablement.
    pub(in crate::workspace) fn sync_commit_buttons(&mut self, cx: &mut Context<Self>) {
        let staged_count = self
            .git_status_cache
            .get(&self.active_worktree_id)
            .map(|s| s.staged.len())
            .unwrap_or(0);
        let in_flight = self.git_op_in_flight;
        let commit_disabled = in_flight || staged_count == 0;
        self.git_commit_input.update(cx, |panel, cx| {
            panel.set_action_disabled("commit", commit_disabled, cx);
        });
    }

    /// Commit staged changes with the current commit-message input text.
    /// Validates the message, then opens a confirm dialog summarising the
    /// commit. The actual git operation runs in [`do_commit_changes`] only
    /// after the user confirms.
    pub(in crate::workspace) fn on_commit_changes(
        &mut self,
        _: &CommitChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_op_in_flight {
            return;
        }

        let message = {
            let panel = self.git_commit_input.read(cx);
            panel.text(cx).to_string()
        };
        if message.trim().is_empty() {
            let report = ErrorReport::new("Commit message cannot be empty")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("git.commit.empty_message")
                .build();
            self.report_error(report, cx);
            return;
        }

        let staged_count = self
            .git_status_cache
            .get(&self.active_worktree_id)
            .map(|s| s.staged.len())
            .unwrap_or(0);
        let first_line = message.lines().next().unwrap_or("").to_string();
        let body = format!("{staged_count} file(s) staged — {first_line}");

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::GIT_CONFIRM_COMMIT_TITLE,
            body,
            app_strings::GIT_CONFIRM_COMMIT_OK,
            ButtonVariant::Primary,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let message = message.clone();
                    let wh = window.window_handle();
                    ws.update(app_cx, |ws, cx| ws.do_commit_changes(message, wh, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git commit -m <message>` in the background. Caller must have
    /// already validated the message and obtained user confirmation.
    /// `wh` is captured so the post-commit `set_text("")` on the commit
    /// input can recover a live `&mut Window` after the async git call
    /// returns.
    fn do_commit_changes(
        &mut self,
        message: String,
        wh: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self.git_repo_root_for(self.active_worktree_id) else {
            return;
        };
        let active_id = self.active_worktree_id;

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let message_bg = message.clone();
        let repo_for_report = repo_root.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_commit(&repo_root, &message_bg),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        let input = ws.git_commit_input.clone();
                        if cx
                            .update_window(wh, |_, window, cx| {
                                input.update(cx, |panel, cx_state| {
                                    panel.set_text("", window, cx_state)
                                });
                            })
                            .is_err()
                        {
                            // Window closed during async commit — input no longer exists.
                        }
                        ws.refresh_git_status(active_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git commit failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.commit")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }

    /// Push the current branch to the remote (no action struct needed — for
    /// direct calls from sidebar render closures that can't import `PushChanges`).
    pub(in crate::workspace) fn trigger_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_push_changes(&PushChanges, window, cx);
    }

    /// Push the current branch to the remote. Opens a confirm dialog before
    /// running the actual push — the git operation lives in [`do_push`].
    pub(in crate::workspace) fn on_push_changes(
        &mut self,
        _: &PushChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_op_in_flight {
            return;
        }
        if self.git_repo_root_for(self.active_worktree_id).is_none() {
            return;
        }

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::GIT_CONFIRM_PUSH_TITLE,
            app_strings::GIT_CONFIRM_PUSH_BODY,
            app_strings::GIT_CONFIRM_PUSH_OK,
            ButtonVariant::Primary,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(app_cx, |ws, cx| ws.do_push(cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git push` in the background. Caller must have obtained user
    /// confirmation.
    fn do_push(&mut self, cx: &mut Context<Self>) {
        let Some(repo_root) = self.git_repo_root_for(self.active_worktree_id) else {
            return;
        };

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let repo_for_report = repo_root.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_push(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                cx.notify();
                if let Err(e) = result {
                    let report = ErrorReport::new("git push failed")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("repo", redact_home(&repo_for_report))
                        .dedup("git.push")
                        .build();
                    ws.report_error(report, cx);
                }
            },
        )
        .detach();
    }

    /// Unstage all files (`git restore --staged .`).
    pub(in crate::workspace) fn unstage_all(
        &mut self,
        worktree_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_top.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_restore_all_staged(&wt_top),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git restore --staged . failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.unstage_all")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Open a confirm dialog before discarding working-tree changes for a
    /// file. The actual git operation runs in [`do_discard_file`] only
    /// after the user confirms. Both untracked deletes (`git clean -f`)
    /// and tracked restores (`git restore`) are irreversible, so the
    /// confirm body spells out which one the user is about to do.
    pub(in crate::workspace) fn on_discard_file(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        is_untracked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        if self.git_stage_in_flight {
            return;
        }
        if !self.worktrees.iter().any(|w| w.id == worktree_id) {
            return;
        }
        let filename = path.file_name_lossy();
        let body = if is_untracked {
            format!(
                "Delete untracked file \"{filename}\"? This file is not in git and cannot be recovered."
            )
        } else {
            format!(
                "Discard working-tree changes to \"{filename}\"? The committed version will replace your edits."
            )
        };

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::GIT_CONFIRM_DISCARD_TITLE,
            body,
            app_strings::GIT_CONFIRM_DISCARD_OK,
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let path = path.clone();
                    ws.update(app_cx, |ws, cx| {
                        ws.do_discard_file(worktree_id, path, is_untracked, cx)
                    });
                }
            },
            window,
            cx,
        );
    }

    /// Discard working-tree changes for a file. For untracked files, deletes
    /// the file (`git clean -f`); for tracked files, restores the last committed
    /// state (`git restore`). Caller must have obtained user confirmation via
    /// [`on_discard_file`].
    fn do_discard_file(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        is_untracked: bool,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(worktree_id);
        // `path` is a repo-root-relative pathspec (from git status output).
        // `git restore`/`git clean` must run from the worktree directory with a
        // worktree-relative path — use WorktreePaths for the two-step conversion.
        let paths = crate::worktree::paths::WorktreePaths {
            wt_path: &wt_path,
            repo_root: repo_root.as_deref(),
        };
        let abs = paths.from_git_status(&path);
        let wt_rel_path = paths.to_wt_relative(&abs).unwrap_or(path);
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_path.clone();
        let rel_for_report = wt_rel_path.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                if is_untracked {
                    crate::worktree::git::git_clean_untracked(&wt_path, &wt_rel_path)
                } else {
                    crate::worktree::git::git_discard_working(&wt_path, &wt_rel_path)
                }
            },
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Err(e) => {
                        let title = if is_untracked {
                            "git clean -f failed"
                        } else {
                            "git restore failed"
                        };
                        let report = ErrorReport::new(title)
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .with_context("file", redact_home(&rel_for_report))
                            .dedup("git.discard")
                            .build();
                        ws.report_error(report, cx);
                        cx.notify();
                    }
                }
            },
        )
        .detach();
    }

    /// Amend the last commit with the current staged changes and the given
    /// message. Opens a confirm dialog warning about history rewrite before
    /// the actual amend runs in [`do_commit_amend`].
    pub(in crate::workspace) fn on_commit_amend(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(cx);
        if self.git_op_in_flight {
            return;
        }

        let message = {
            let panel = self.git_commit_input.read(cx);
            panel.text(cx).to_string()
        };
        if message.trim().is_empty() {
            let report = ErrorReport::new("Commit message cannot be empty")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("git.amend.empty_message")
                .build();
            self.report_error(report, cx);
            return;
        }

        if self.git_repo_root_for(self.active_worktree_id).is_none() {
            return;
        }

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::GIT_CONFIRM_AMEND_TITLE,
            app_strings::GIT_CONFIRM_AMEND_BODY,
            app_strings::GIT_CONFIRM_AMEND_OK,
            ButtonVariant::Danger,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let message = message.clone();
                    let wh = window.window_handle();
                    ws.update(app_cx, |ws, cx| ws.do_commit_amend(message, wh, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Run `git commit --amend -m <message>` in the background. Caller must
    /// have obtained user confirmation. `wh` is captured so the
    /// post-amend `set_text("")` on the commit input recovers a live
    /// `&mut Window` after the async git call.
    fn do_commit_amend(
        &mut self,
        message: String,
        wh: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self.git_repo_root_for(self.active_worktree_id) else {
            return;
        };
        let active_id = self.active_worktree_id;

        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();

        let repo_for_report = repo_root.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_commit_amend(&repo_root, &message),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        let input = ws.git_commit_input.clone();
                        if cx
                            .update_window(wh, |_, window, cx| {
                                input.update(cx, |panel, cx_state| {
                                    panel.set_text("", window, cx_state)
                                });
                            })
                            .is_err()
                        {
                            // Window closed during async amend — input no longer exists.
                        }
                        ws.refresh_git_status(active_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git commit --amend failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.amend")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }

    /// Build the keyboard-navigable file order for the Git Changes view
    /// in the active worktree. Defers to the sidebar's
    /// `ordered_visible_paths` helper so any future change to the render
    /// order (sticky conflicts, custom sort) automatically applies to
    /// `↑↓` nav.
    fn git_changes_visible_paths(&self) -> Vec<PathBuf> {
        let Some(s) = self.git_status_cache.get(&self.active_worktree_id) else {
            return Vec::new();
        };
        let Some(wt) = self
            .worktrees
            .iter()
            .find(|w| w.id == self.active_worktree_id)
        else {
            return Vec::new();
        };
        let collapsed = self
            .git_collapsed_dirs
            .get(&self.active_worktree_id)
            .cloned()
            .unwrap_or_default();
        crate::workspace::sidebar::git_changes::ordered_visible_paths(s, &collapsed, &wt.paths())
    }

    /// Run `git init` in a non-git worktree, then re-probe so the
    /// worktree's `kind` flips from `Default` to `Git` and the Git
    /// Changes view starts surfacing changes immediately. No-op for
    /// worktrees that are already git-backed.
    pub(in crate::workspace) fn init_git_repo(
        &mut self,
        worktree_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        if self.git_op_in_flight {
            return;
        }
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        if wt.is_git() {
            return;
        }
        let path = wt.path.clone();
        self.git_op_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || -> Result<Option<_>, crate::worktree::git::GitError> {
                crate::worktree::git::git_init(&path)?;
                Ok(crate::worktree::git::probe_repo(&path))
            },
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                match result {
                    Ok(Some(probe)) => {
                        // Pick the matching worktree entry's branch from
                        // `git worktree list` so the header label flips
                        // from "detached" to the actual branch name (git
                        // init defaults to `main`, but a user-configured
                        // `init.defaultBranch` may differ — read what git
                        // actually decided rather than guessing).
                        if let Some(wt) = ws.worktrees.iter_mut().find(|w| w.id == worktree_id) {
                            let probed_entry = probe
                                .worktrees
                                .iter()
                                .find(|p| p.path == wt.path)
                                .or_else(|| probe.worktrees.first());
                            let probed_branch = probed_entry.and_then(|p| p.branch.clone());
                            // Freshly init'd repo: the toplevel from the
                            // probe entry IS this worktree's root. Fall
                            // back to `wt.path` if for some reason the
                            // entry list is empty.
                            let worktree_root = probed_entry
                                .map(|p| p.path.clone())
                                .unwrap_or_else(|| wt.path.clone());
                            wt.kind = daruda_store::project::WorktreeKind::Git {
                                repo_root: probe.repo_root,
                                branch: probed_branch,
                                worktree_root,
                            };
                        }
                        ws.refresh_git_status(worktree_id, cx);
                    }
                    Ok(None) => {
                        // `git init` succeeded — the repo is on disk and
                        // usable; only the follow-up probe that flips
                        // `WorktreeKind::Default → Git` failed. The user
                        // can re-open the project and the next probe
                        // will pick it up. Warning, not Error.
                        let report = ErrorReport::new("git init succeeded but probe failed")
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.init.probe")
                            .build();
                        ws.report_error(report, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git init failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.init")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }

    /// Move the Git Changes keyboard cursor to a specific path. Used by
    /// row clicks so subsequent arrow-key nav resumes from the clicked
    /// row rather than wherever the cursor was last left.
    pub(in crate::workspace) fn set_git_changes_cursor(
        &mut self,
        worktree_id: WorktreeId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.git_changes_cursor.insert(worktree_id, path);
        cx.notify();
    }

    /// Move the Git Changes keyboard cursor to the next or previous row.
    /// `delta = +1` walks down, `delta = -1` walks up. Out-of-list
    /// cursors snap to the first/last visible row; an empty list is a
    /// no-op.
    pub(in crate::workspace) fn move_git_changes_cursor(
        &mut self,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let visible = self.git_changes_visible_paths();
        if visible.is_empty() {
            return;
        }
        let active_id = self.active_worktree_id;
        let current_idx = self
            .git_changes_cursor
            .get(&active_id)
            .and_then(|p| visible.iter().position(|v| v == p));
        let new_idx: usize = match (current_idx, delta) {
            (None, d) if d >= 0 => 0,
            (None, _) => visible.len() - 1,
            (Some(i), d) => {
                let len = visible.len() as isize;
                ((i as isize + d).rem_euclid(len)) as usize
            }
        };
        self.git_changes_cursor
            .insert(active_id, visible[new_idx].clone());
        cx.notify();
    }

    /// Toggle the staged/unstaged state of the file under the keyboard
    /// cursor (Space). No-op when the cursor is unset or the file has
    /// vanished from `git status`.
    pub(in crate::workspace) fn toggle_git_changes_cursor_stage(&mut self, cx: &mut Context<Self>) {
        let active_id = self.active_worktree_id;
        let Some(cursor) = self.git_changes_cursor.get(&active_id).cloned() else {
            return;
        };
        let Some(s) = self.git_status_cache.get(&active_id) else {
            return;
        };
        let is_staged = s.staged.iter().any(|e| e.path == cursor);
        if is_staged {
            self.unstage_file(active_id, cursor, cx);
        } else {
            self.stage_file(active_id, cursor, cx);
        }
    }

    /// Open the diff viewer for the file under the keyboard cursor (Enter).
    pub(in crate::workspace) fn activate_git_changes_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_id = self.active_worktree_id;
        let Some(cursor) = self.git_changes_cursor.get(&active_id).cloned() else {
            return;
        };
        let Some(s) = self.git_status_cache.get(&active_id) else {
            return;
        };
        let staged_entry = s.staged.iter().find(|e| e.path == cursor);
        let unstaged_entry = s.unstaged.iter().find(|e| e.path == cursor);
        let (is_staged, status_char) = match (staged_entry, unstaged_entry) {
            (Some(se), _) => (true, se.x),
            (None, Some(ue)) => (false, ue.y),
            (None, None) => return,
        };

        // The diff viewer wants the absolute path (it routes through
        // `open_pane_file_view` which loads from the filesystem); resolve
        // the repo-root-relative cursor via WorktreePaths.
        let Some(wt) = self.worktrees.iter().find(|w| w.id == active_id) else {
            return;
        };
        let abs = wt.paths().from_git_status(&cursor);
        self.open_git_file_diff(active_id, abs, is_staged, Some(status_char), window, cx);
    }

    /// Toggle the collapse state of a directory group in the Git Changes
    /// view. State is per-worktree and in-memory only — Git Changes is
    /// task-driven (open it, deal with the diff, close it), so persisting
    /// collapse state across app restarts would mostly preserve stale
    /// "I last collapsed this dir three weeks ago" noise.
    pub(in crate::workspace) fn toggle_git_dir_collapse(
        &mut self,
        worktree_id: WorktreeId,
        dir: String,
        cx: &mut Context<Self>,
    ) {
        let set = self.git_collapsed_dirs.entry(worktree_id).or_default();
        if !set.remove(&dir) {
            set.insert(dir);
        }
        cx.notify();
    }

    /// Fetch from all remotes.
    pub(in crate::workspace) fn on_fetch(&mut self, cx: &mut Context<Self>) {
        if self.git_op_in_flight {
            return;
        }
        let Some(repo_root) = self.git_repo_root_for(self.active_worktree_id) else {
            return;
        };
        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();
        let repo_for_report = repo_root.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_fetch(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                if let Err(e) = result {
                    let report = ErrorReport::new("git fetch failed")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("repo", redact_home(&repo_for_report))
                        .dedup("git.fetch")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Pull from the remote tracking branch.
    pub(in crate::workspace) fn on_pull(&mut self, cx: &mut Context<Self>) {
        if self.git_op_in_flight {
            return;
        }
        let Some(repo_root) = self.git_repo_root_for(self.active_worktree_id) else {
            return;
        };
        let active_id = self.active_worktree_id;
        self.git_op_in_flight = true;
        self.sync_commit_buttons(cx);
        cx.notify();
        let repo_for_report = repo_root.clone();
        super::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_pull(&repo_root),
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                ws.sync_commit_buttons(cx);
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(active_id, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git pull failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("repo", redact_home(&repo_for_report))
                            .dedup("git.pull")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Open `path` from worktree `worktree_id` using the system default
    /// application. Runs the `open` command on a background thread so the
    /// UI thread is never blocked. Kept for a future context-menu "Open in
    /// default app" action on the Git Changes file list.
    ///
    /// `path` may be either worktree-relative (Files sidebar convention) or
    /// absolute (Git Changes sidebar uses repo-root-relative paths and joins
    /// against repo_root before calling). `Path::join` returns the absolute
    /// argument unchanged, so the same code handles both cases.
    #[allow(dead_code)]
    pub(in crate::workspace) fn open_file_externally(
        &mut self,
        worktree_id: daruda_store::project::WorktreeId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let full_path = wt.path.join(&path);
        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .spawn(async move {
                    let _ = std::process::Command::new("open").arg(&full_path).status();
                })
                .await;
        })
        .detach();
    }
}
