//! Workspace state persistence — save / restore project state plus
//! the helpers (`serialize_layout`, `normalize_ratios`,
//! `rebuild_layout`) that translate between the in-memory pane tree
//! and the on-disk JSON form.
//!
//! Owns `WorktreeRuntime`, the frozen-fields struct used by the
//! tab-swap path; both `restore_state` and `activate_worktree` live
//! across persistence + worktree_ops, but the *type* belongs here so
//! the inactive map and the save format share a single definition.

use std::collections::HashMap;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use gpui::{App, Context, Window};

use crate::path_ext::PathExt;

use super::Workspace;
use super::layout::{self, PaneLayout, SplitDirection};
use super::pane::{self, PaneSpawnError, TabEntry};

/// Frozen runtime state of a non-active worktree. `activate_worktree`
/// swaps this with the live `Workspace` fields (`tabs`, `panes`, etc.).
/// Kept in its own struct so the swap is a single `std::mem::take` +
/// assignment pair per direction, rather than a tangle of field moves.
#[derive(Default)]
pub(in crate::workspace) struct WorktreeRuntime {
    pub tabs: Vec<pane::TabEntry>,
    pub panes: Vec<pane::Pane>,
    pub active_tab_index: usize,
    /// Tab navigation history (most-recent-last). Not serialized —
    /// history is a session-only convenience; cold-restoring it would
    /// be confusing because the user's intent from a previous session
    /// is unknown. Starts empty on every app launch.
    pub tab_history: Vec<usize>,
    pub focused_pane_id: layout::PaneId,
}

impl Workspace {
    /// Serialize the current workspace state for persistence.
    pub fn save_state(&self, cx: &App) -> Option<daruda_store::project::ProjectState> {
        let project = self.project.as_ref()?;
        // Persist every worktree's tab tree. For the active worktree
        // the live `Workspace.tabs`/`panes` are the source; for
        // inactive worktrees the frozen runtime in
        // `inactive_worktree_runtimes` is used.
        let worktrees: Vec<daruda_store::project::SerializedWorktree> = self
            .worktrees
            .iter()
            .map(|wt| {
                let mut s = wt.to_serialized();
                let (tabs_src, panes_src, active_idx) = if wt.id == self.active_worktree_id {
                    (&self.tabs, &self.panes, self.active_tab_index)
                } else if let Some(rt) = self.inactive_worktree_runtimes.get(&wt.id) {
                    (&rt.tabs, &rt.panes, rt.active_tab_index)
                } else {
                    return s;
                };
                s.tabs = tabs_src
                    .iter()
                    .map(|tab| daruda_store::project::SerializedTab {
                        layout: serialize_layout(&tab.layout, panes_src),
                        last_focused_pane: tab.last_focused_pane,
                        user_label: tab.user_label.as_ref().map(|s| s.to_string()),
                    })
                    .collect();
                s.active_tab_index = active_idx;
                s
            })
            .collect();

        Some(daruda_store::project::ProjectState {
            root: project.root.clone(),
            worktrees,
            active_worktree_id: self.active_worktree_id,
            active_sidebar_view: self.left_sidebar_view,
            active_right_panel_view: self.right_sidebar_view,
            active_usage_window: self.claude.usage_window,
            // Top-level `tabs` stays empty from W-2 onward — the data
            // now lives on the active worktree. `skip_serializing_if`
            // keeps it out of the JSON.
            tabs: Vec::new(),
            active_tab_index: 0,
            focused_pane_id: self.focused_pane_id,
            docks: daruda_store::project::DockStates {
                left_open: self.left_dock.read(cx).is_open,
                left_size: self.left_dock.read(cx).size,
                bottom_open: self.bottom_dock.read(cx).is_open,
                bottom_size: self.bottom_dock.read(cx).size,
                right_open: self.right_dock.read(cx).is_open,
                right_size: self.right_dock.read(cx).size,
            },
            window: self.cached_window_bounds.clone().unwrap_or_default(),
            window_user_label: self.window_user_label.as_ref().map(|s| s.to_string()),
            font_size: self.terminal_config.font_size,
            vertical_spacing: self.terminal_config.vertical_spacing,
            horizontal_spacing: self.terminal_config.horizontal_spacing,
        })
    }

    /// Sample the current window's windowed bounds (position + size)
    /// and store them in `cached_window_bounds`. Skips fullscreen and
    /// maximized states — persisting those would relaunch the app in
    /// that mode next time even if the user exited it before quitting.
    /// The last known windowed geometry (from before entering those
    /// states) remains cached.
    pub(in crate::workspace) fn capture_window_bounds(&mut self, window: &Window) {
        let bounds = match window.window_bounds() {
            gpui::WindowBounds::Windowed(b) => b,
            gpui::WindowBounds::Maximized(_) | gpui::WindowBounds::Fullscreen(_) => return,
        };
        let new = daruda_store::project::WindowState {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        };
        if !new.is_valid() {
            return;
        }
        self.cached_window_bounds = Some(new);
    }

    /// Persist state to disk (debounce handled by caller).
    pub fn persist_state(&self, cx: &App) {
        if let Some(state) = self.save_state(cx) {
            if let Err(e) =
                daruda_store::project::persistence::save_state_in(&self.data_dir, &state)
            {
                LogWriter::log(
                    ErrorReport::new("Failed to persist workspace state")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("data_dir", redact_home(&self.data_dir))
                        .with_context("project_root", redact_home(&state.root))
                        .dedup("project.state.save")
                        .build(),
                );
            }
            if let Err(e) =
                daruda_store::project::persistence::touch_recent_in(&self.data_dir, &state.root)
            {
                LogWriter::log(
                    ErrorReport::new("Failed to update recent projects list")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("data_dir", redact_home(&self.data_dir))
                        .with_context("project_root", redact_home(&state.root))
                        .dedup("project.recent.touch")
                        .build(),
                );
            }
        }
    }

    /// Restore workspace from a saved state. Rebuilds dock open/size,
    /// font settings, and the full tab / split-tree layout. Each
    /// restored leaf starts at its serialized cwd so the shell opens
    /// where it was last tracked. On any pane spawn failure, falls
    /// back to a single fresh tab and surfaces the error in the
    /// status bar.
    pub fn restore_state(
        &mut self,
        state: &daruda_store::project::ProjectState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Restore dock states.
        let left_open = state.docks.left_open;
        let left_size = state.docks.left_size;
        self.left_dock.update(cx, |d, _| {
            d.is_open = left_open;
            if left_size > 0.0 {
                d.size = left_size;
            }
        });
        self.left_sidebar_view = state.active_sidebar_view;
        self.right_sidebar_view = state.active_right_panel_view;
        self.claude.usage_window = state.active_usage_window;
        // Resync the dropdown so its visible selection matches the
        // restored state (the entity was constructed with the
        // default `Last7d` value before this method runs).
        let restored_window = state.active_usage_window;
        let slug = gpui::SharedString::from(restored_window.slug());
        self.claude.usage_select.update(cx, |s, cx_inner| {
            s.set_selected_value(&slug, window, cx_inner);
        });
        let bottom_open = state.docks.bottom_open;
        let bottom_size = state.docks.bottom_size;
        self.bottom_dock.update(cx, |d, _| {
            d.is_open = bottom_open;
            if bottom_size > 0.0 {
                d.size = bottom_size;
            }
        });
        let right_open = state.docks.right_open;
        let right_size = state.docks.right_size;
        self.right_dock.update(cx, |d, _| {
            d.is_open = right_open;
            if right_size > 0.0 {
                d.size = right_size;
            }
        });

        // Restore font settings.
        self.terminal_config.font_size = state.font_size;
        self.terminal_config.vertical_spacing = state.vertical_spacing;
        self.terminal_config.horizontal_spacing = state.horizontal_spacing;
        self.terminal_config.clamp_font_settings();

        // Restore user-set window title, if any.
        self.window_user_label = state
            .window_user_label
            .as_ref()
            .map(|s| gpui::SharedString::from(s.clone()));

        // Restore worktrees (metadata only). A legacy file that only
        // had top-level tabs gets its default worktree synthesized by
        // `ProjectState::migrate_legacy` before we get here, so this
        // is always non-empty when the file had any saved session.
        if !state.worktrees.is_empty() {
            self.worktrees = state
                .worktrees
                .iter()
                .map(crate::worktree::Worktree::from_serialized)
                .collect();

            // Re-apply the git-root → project-subdirectory anchor so
            // that state files saved before the bootstrap fix still
            // match Claude's `cwd` after restore.
            if let Some(project_root) = self.project.as_ref().map(|p| p.root.as_path()) {
                anchor_worktree_paths_to_project_root(&mut self.worktrees, project_root);
            }

            self.active_worktree_id = state.active_worktree_id;
            // Clamp to a live worktree in case the saved id is stale.
            if !self
                .worktrees
                .iter()
                .any(|w| w.id == self.active_worktree_id)
                && let Some(first) = self.worktrees.first()
            {
                self.active_worktree_id = first.id;
            }
        }

        // Nothing to rebuild — keep the single pane `new_with_project`
        // already created.
        if state.worktrees.is_empty() {
            self.pending_resize = true;
            return;
        }

        // Drop bootstrapped pane/tab; we're about to rebuild every
        // worktree's runtime from scratch.
        self.tabs.clear();
        self.panes.clear();
        self.activity_counter.clear();
        self.inactive_worktree_runtimes.clear();

        // Rebuild each serialized worktree into a WorktreeRuntime.
        // `rebuild_layout` appends spawned panes onto `self.panes`, so
        // we split them off at the end of each worktree to keep them
        // segregated.
        let mut active_focus: Option<layout::PaneId> = None;
        for swt in &state.worktrees {
            // If the worktree's checkout is no longer on disk, warn via the
            // status bar. Inactive worktrees skip layout rebuild — their
            // runtime is built lazily on first activation (the PTY layer
            // then falls back to $HOME for the missing path). The active
            // worktree still rebuilds so the workspace is never blank; its
            // panes likewise start at $HOME.
            if !swt.path.is_accessible_dir() {
                let report = ErrorReport::new("Worktree path not found")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&swt.path))
                    .dedup("worktree.restore.missing_path")
                    .build();
                self.report_error(report, cx);
                if swt.id != state.active_worktree_id {
                    continue;
                }
            }

            let panes_start = self.panes.len();
            let mut id_map: HashMap<u64, layout::PaneId> = HashMap::new();
            let mut tabs: Vec<TabEntry> = Vec::new();
            let mut failed = false;

            for stab in &swt.tabs {
                let panes_before = self.panes.len();
                // Pass the worktree's own path as the fallback cwd so
                // restore never lands a pane at the parent process's
                // cwd (HOME when daruda is launched from Finder/Dock).
                // This matters when the saved state has `cwd = None`
                // for a leaf — typical when the workspace was
                // serialized before the shell first emitted OSC 7.
                match self.rebuild_layout(&stab.layout, Some(&swt.path), &mut id_map, window, cx) {
                    Ok(layout) => {
                        let last_focus = id_map
                            .get(&stab.last_focused_pane)
                            .copied()
                            .unwrap_or_else(|| layout.first_leaf());
                        let tab_id = self.alloc_id();
                        tabs.push(TabEntry {
                            id: tab_id,
                            layout,
                            last_focused_pane: last_focus,
                            user_label: stab
                                .user_label
                                .as_ref()
                                .map(|s| gpui::SharedString::from(s.clone())),
                        });
                    }
                    Err(e) => {
                        self.panes.truncate(panes_before);
                        self.report_pane_error("restore", e, cx);
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                break;
            }

            let wt_panes = self.panes.split_off(panes_start);
            let wt_active_tab = swt.active_tab_index.min(tabs.len().saturating_sub(1));
            // Focus defaults to the tab's last focused pane; only the
            // active worktree honours the top-level `focused_pane_id`
            // hint from the saved state.
            let focused = if swt.id == state.active_worktree_id {
                let focused_tab = tabs.get(wt_active_tab);
                let leaves = focused_tab.map(|t| t.layout.pane_ids());
                id_map
                    .get(&state.focused_pane_id)
                    .copied()
                    .filter(|id| leaves.as_ref().is_some_and(|l| l.contains(id)))
                    .unwrap_or_else(|| focused_tab.map(|t| t.last_focused_pane).unwrap_or_default())
            } else {
                tabs.get(wt_active_tab)
                    .map(|t| t.last_focused_pane)
                    .unwrap_or_default()
            };

            let runtime = WorktreeRuntime {
                tabs,
                panes: wt_panes,
                active_tab_index: wt_active_tab,
                tab_history: Vec::new(),
                focused_pane_id: focused,
            };

            if swt.id == state.active_worktree_id {
                self.tabs = runtime.tabs;
                self.panes = runtime.panes;
                self.active_tab_index = runtime.active_tab_index;
                self.focused_pane_id = runtime.focused_pane_id;
                self.tab_history = runtime.tab_history;
                active_focus = Some(runtime.focused_pane_id);
            } else {
                self.inactive_worktree_runtimes.insert(swt.id, runtime);
            }
        }

        // Fall back to a fresh tab if nothing was restored.
        if self.tabs.is_empty() {
            self.add_tab(window, cx);
            return;
        }

        if let Some(focus) = active_focus {
            self.bump_activity(focus);
            self.focus_pane(focus, window, cx);
        }
        self.pending_resize = true;

        // Auto-refresh git status when restoring with the Git Changes sidebar
        // active — the cache is always empty on startup so the sidebar would
        // show the placeholder until the user clicked Refresh manually.
        if self.left_sidebar_view == daruda_store::project::LeftSidebarView::GitChanges {
            let id = self.active_worktree_id;
            self.refresh_git_status(id, cx);
        }

        // Kick off content loads for any restored File panes in the
        // active worktree. Inactive worktrees' file panes load when
        // their worktree is next activated.
        self.load_pending_file_panes(cx);

        cx.notify();
    }

    /// Recursively rebuild a `PaneLayout` from its serialized form.
    /// Panes are spawned eagerly; `id_map` records the new PaneId for
    /// each serialized pane_id so cross-references (focus, last
    /// focused per tab) can be rewritten.
    /// Recursively materialize a serialized layout into live panes.
    /// `fallback_cwd` is the cwd to use when a leaf's serialized cwd
    /// is missing — typically the owning worktree's path so restore
    /// keeps each pane inside its worktree even for sessions that
    /// were saved before OSC 7 first reported the live cwd.
    fn rebuild_layout(
        &mut self,
        slayout: &daruda_store::project::SerializedLayout,
        fallback_cwd: Option<&std::path::Path>,
        id_map: &mut HashMap<u64, layout::PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneLayout, PaneSpawnError> {
        match slayout {
            daruda_store::project::SerializedLayout::Leaf { pane_id, cwd, file } => {
                let pane = if let Some(fc) = file {
                    // File pane — `file_status` is not persisted; the
                    // git badge re-derives when the worktree's git
                    // status next refreshes. Content stays at
                    // `Loading` until the owning worktree becomes
                    // active and `load_pending_file_panes` fires.
                    self.create_file_pane(
                        fc.worktree_id,
                        fc.path.clone(),
                        fc.staged,
                        None,
                        deserialize_view_mode(fc.view_mode),
                        window,
                        cx,
                    )
                } else {
                    let effective = effective_cwd(cwd.clone(), fallback_cwd);
                    self.create_pane_with_cwd(effective, window, cx)?
                };
                let new_id = pane.id;
                id_map.insert(*pane_id, new_id);
                self.panes.push(pane);
                Ok(PaneLayout::Pane(new_id))
            }
            daruda_store::project::SerializedLayout::Split {
                direction,
                children,
                ratios,
            } => {
                let dir = match direction {
                    daruda_store::project::SplitDirectionSerde::Horizontal => {
                        SplitDirection::Horizontal
                    }
                    daruda_store::project::SplitDirectionSerde::Vertical => {
                        SplitDirection::Vertical
                    }
                };
                let mut rebuilt = Vec::with_capacity(children.len());
                for c in children {
                    rebuilt.push(self.rebuild_layout(c, fallback_cwd, id_map, window, cx)?);
                }
                let n = rebuilt.len();
                if n == 0 {
                    // Degenerate serialization — materialize a fresh leaf
                    // so we never surface an empty Split to the renderer.
                    let pane = self.create_pane_with_cwd(
                        fallback_cwd.map(|p| p.to_path_buf()),
                        window,
                        cx,
                    )?;
                    let id = pane.id;
                    self.panes.push(pane);
                    return Ok(PaneLayout::Pane(id));
                }
                if n == 1 {
                    // Invariant: n == 1, rebuilt has exactly one element.
                    return Ok(rebuilt
                        .into_iter()
                        .next()
                        .expect("n == 1: rebuilt has one element"));
                }
                let normalized = normalize_ratios(ratios, n);
                Ok(PaneLayout::Split {
                    direction: dir,
                    children: rebuilt,
                    ratios: normalized,
                })
            }
        }
    }
}

/// Clamp a serialized ratios vector back into a valid distribution:
/// length must match the child count; the sum must normalize to 1.0.
/// Falls back to equal splits if either invariant is violated.
pub(in crate::workspace) fn normalize_ratios(ratios: &[f32], expected_len: usize) -> Vec<f32> {
    let fallback = || vec![1.0 / expected_len as f32; expected_len];
    if ratios.len() != expected_len {
        return fallback();
    }
    let sum: f32 = ratios.iter().sum();
    if sum <= f32::EPSILON || !sum.is_finite() {
        return fallback();
    }
    ratios.iter().map(|r| r / sum).collect()
}

/// Map the in-memory `FileViewMode` to its persistence mirror.
/// Kept as a free function so the conversion lives next to the only
/// site that needs it (serialization round-trips).
fn serialize_view_mode(
    mode: super::file_viewer::FileViewMode,
) -> daruda_store::project::SerializedFileViewMode {
    use super::file_viewer::FileViewMode;
    match mode {
        FileViewMode::Raw => daruda_store::project::SerializedFileViewMode::Raw,
        FileViewMode::Preview => daruda_store::project::SerializedFileViewMode::Preview,
        FileViewMode::Changes => daruda_store::project::SerializedFileViewMode::Changes,
    }
}

/// Inverse of [`serialize_view_mode`].
pub(in crate::workspace) fn deserialize_view_mode(
    mode: daruda_store::project::SerializedFileViewMode,
) -> super::file_viewer::FileViewMode {
    use super::file_viewer::FileViewMode;
    match mode {
        daruda_store::project::SerializedFileViewMode::Raw => FileViewMode::Raw,
        daruda_store::project::SerializedFileViewMode::Preview => FileViewMode::Preview,
        daruda_store::project::SerializedFileViewMode::Changes => FileViewMode::Changes,
    }
}

/// Select the cwd to use when spawning a restored terminal pane.
///
/// `saved` is the path serialized at the time of the last save (reported
/// via OSC 7 from the shell). `worktree_root` is the physical checkout
/// directory of the owning worktree.
///
/// **Invariant**: the returned path is always a descendant of
/// `worktree_root` (or `worktree_root` itself). A saved path that
/// escapes the worktree — e.g. a stale cwd from a different checkout —
/// is discarded in favour of `worktree_root`. This prevents a pane from
/// opening in an unrelated directory or the parent process's cwd (HOME
/// when daruda is launched from Finder/Dock).
fn effective_cwd(
    saved: Option<std::path::PathBuf>,
    worktree_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    saved
        .filter(|s| worktree_root.is_none_or(|root| s.starts_with(root)))
        .or_else(|| worktree_root.map(|p| p.to_path_buf()))
}

/// Re-anchor each worktree's `path` to the project subdirectory when
/// the serialized path is a git root that *contains* the project root.
///
/// **Before the bootstrap fix**: `Worktree::bootstrap_from_project`
/// stored the git root (`/repo`) as the worktree path even when the
/// user opened a subdirectory (`/repo/subdir`). After the fix, the
/// path is anchored to `project_root`. This function updates surviving
/// old-format entries so restored cwd anchoring (`effective_cwd`)
/// rejects out-of-worktree paths correctly.
///
/// **Prefix relationship**:
/// ```text
/// wt.path  = /repo          (git root — the old value)
/// canonical = /repo/subdir  (project root — the new anchor)
///
/// canonical.starts_with(wt.path)  →  true   (subdir of the worktree)
/// canonical != wt.path            →  true   (not identical — needs update)
/// ```
/// Only the first matching worktree is updated because each project has
/// exactly one primary checkout that maps to the project root.
fn anchor_worktree_paths_to_project_root(
    worktrees: &mut [crate::worktree::Worktree],
    project_root: &std::path::Path,
) {
    if let Ok(canonical) = std::fs::canonicalize(project_root) {
        for wt in worktrees.iter_mut() {
            if canonical.starts_with(&wt.path) && canonical != wt.path {
                wt.path = canonical;
                break;
            }
        }
    }
}

fn serialize_layout(
    layout: &layout::PaneLayout,
    panes: &[pane::Pane],
) -> daruda_store::project::SerializedLayout {
    match layout {
        layout::PaneLayout::Pane(id) => {
            let pane = panes.iter().find(|p| p.id == *id);
            // File panes serialize their viewer state; Terminal panes
            // serialize their cwd. The two are mutually exclusive —
            // a File leaf carries `cwd: None` because it derives its
            // cwd from `path.parent()` at runtime.
            let file = pane.and_then(|p| p.file_view()).map(|fv| {
                daruda_store::project::SerializedFileContent {
                    worktree_id: fv.worktree_id,
                    path: fv.path.clone(),
                    staged: fv.staged,
                    view_mode: serialize_view_mode(fv.view_mode),
                }
            });
            let cwd = if file.is_some() {
                None
            } else {
                pane.and_then(|p| p.cwd().map(std::path::Path::to_path_buf))
            };
            daruda_store::project::SerializedLayout::Leaf {
                pane_id: *id,
                cwd,
                file,
            }
        }
        layout::PaneLayout::Split {
            direction,
            children,
            ratios,
        } => {
            let dir = match direction {
                layout::SplitDirection::Horizontal => {
                    daruda_store::project::SplitDirectionSerde::Horizontal
                }
                layout::SplitDirection::Vertical => {
                    daruda_store::project::SplitDirectionSerde::Vertical
                }
            };
            daruda_store::project::SerializedLayout::Split {
                direction: dir,
                children: children
                    .iter()
                    .map(|c| serialize_layout(c, panes))
                    .collect(),
                ratios: ratios.clone(),
            }
        }
    }
}
