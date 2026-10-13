//! Background file-tree operations + visible-row cache for the left dock
//! Files view.
//!
//! `ensure_file_tree` lazily creates the per-lane `FileTree` and kicks a
//! root scan; `toggle_files_expand` flips the expanded set and, on
//! `UnloadedDir → expanded`, kicks a `load_dir` task returning via
//! `apply_dir_load_result`. All filesystem reads run on
//! `cx.background_executor()` so the UI thread never blocks.
//!
//! `cached_or_rebuild_visible` flattens the tree into the linear list
//! `uniform_list` consumes, memoised in the lane's visible cache. The cache
//! is invalidated only at fixed trigger points (toggle expand, load
//! result, watcher event, focused-file-viewer change, activate_lane, git
//! status update, config change); other `cx.notify()` calls read the
//! cached `Arc` directly.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::files::gitignore::GitignoreSet;
use crate::files::load::load_dir;
use crate::files::tree::{EntryId, EntryKind, FileTree};
use crate::files::watcher::{DebouncedEvent, FileTreeWatcher};
use crate::lane::availability::LaneAvailability;
use crate::workspace::Workspace;

/// Distinguishes the two `apply_dir_load_result` call sites so the
/// watcher-driven path can stay silent on `NotFound` (a directory
/// deleted between the change event and the read — expected).
#[derive(Copy, Clone)]
pub(in crate::workspace) enum DirLoadSource {
    UserExpand,
    WatcherReload,
}

// ----------------------------------------------------------------
// Reload queue — per-lane serial drain (single in-flight task)
// ----------------------------------------------------------------

/// Queue of pending reloads for one lane. Serialised so a burst of
/// watcher events does not spawn one fs task per affected directory.
#[derive(Default)]
pub(in crate::workspace) struct FilesReloadQueue {
    /// FIFO of absolute parent-directory paths to reload. Caller pops
    /// from the front; producer pushes to the back.
    pending_parents: VecDeque<PathBuf>,
    /// Membership view of `pending_parents` for O(1) dedup.
    pending_seen: HashSet<PathBuf>,
    /// When set, every pending Changed entry is dropped — a Bulk
    /// reload (root + currently expanded) supersedes them.
    pending_bulk: bool,
    /// `true` while `kick_files_reload`'s drain task is running; new
    /// events just enqueue rather than spawning another task.
    running: bool,
}

impl FilesReloadQueue {
    #[cfg(test)]
    pub(in crate::workspace) fn is_running_for_test(&self) -> bool {
        self.running
    }
}

enum ReloadTask {
    Bulk,
    /// Absolute path of the directory to reload.
    Parent(PathBuf),
}

// ----------------------------------------------------------------
// Workspace impl
// ----------------------------------------------------------------

impl Workspace {
    pub(in crate::workspace) fn ensure_file_tree(
        &mut self,
        wt_ref: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(wt_ref) else {
            return;
        };
        let root = wt.path.clone();

        // An unavailable lane root (deleted / unreadable) can't be
        // scanned without `read_dir` failing every tick and spamming the
        // toast. Tear down any state built while it was present and skip
        // the load / watcher / gitignore build.
        if wt.availability != LaneAvailability::Present {
            self.teardown_unavailable_lane_state(wt_ref);
            return;
        }

        let needs_load = match self.lane_file_tree(wt_ref) {
            Some(tree) => tree
                .entry(tree.root_id)
                .map(|e| matches!(e.kind, EntryKind::UnloadedDir))
                .unwrap_or(false),
            None => {
                self.lane_scoped_mut(wt_ref).files.tree = Some(FileTree::new(root.clone()));
                true
            }
        };

        if needs_load {
            if let Some(tree) = self.lane_file_tree_mut(wt_ref)
                && let Some(entry) = tree.entry_mut(EntryId(0))
                && matches!(entry.kind, EntryKind::UnloadedDir)
            {
                entry.kind = EntryKind::PendingDir;
            }
            self.invalidate_visible_files_cache(wt_ref);
            self.kick_dir_load(wt_ref, EntryId(0), root.clone(), cx);
        }

        // Start a watcher on first touch. The watcher is GPUI-free; the
        // polling task belongs to Workspace and is created lazily.
        if self
            .lane_scoped
            .get(&wt_ref)
            .is_none_or(|state| state.files.watcher.is_none())
        {
            self.spawn_files_watcher(wt_ref, root.clone(), cx);
        }
        // Build the gitignore matcher once on a background thread;
        // rebuilt when `.gitignore` changes.
        if self
            .lane_scoped
            .get(&wt_ref)
            .is_none_or(|state| state.files.gitignore.is_none())
        {
            self.kick_gitignore_build(wt_ref, root.clone(), cx);
        }
    }

    /// Tear down file-tree state for a lane whose root flipped to
    /// non-`Present` (deleted / unreadable): removes the watcher (so it
    /// stops firing against the missing path), the stale tree, visible
    /// cache, gitignore matcher, and reload queue. Per-lane git and
    /// cursor state stay untouched — those are reset by lane removal, not
    /// a transient availability flip.
    ///
    /// Called from `ensure_file_tree` (lazy-create path, catches a lane
    /// gone missing between sessions) and `apply_dir_load_result`
    /// (watcher-driven, catches an active lane going missing mid-session,
    /// which `ensure_file_tree` never reaches once a tree exists).
    pub(super) fn teardown_unavailable_lane_state(&mut self, wt_ref: LaneRef) {
        if let Some(state) = self.lane_scoped.get_mut(&wt_ref) {
            // Stop the watcher before discarding data from the unavailable root.
            state.files.watcher = None;
            state.files = Default::default();
        }
    }

    pub(in crate::workspace) fn toggle_files_expand(
        &mut self,
        wt_ref: LaneRef,
        entry_id: EntryId,
        cx: &mut Context<Self>,
    ) {
        let to_load: Option<PathBuf> = {
            let Some(tree) = self.lane_file_tree_mut(wt_ref) else {
                return;
            };
            let now_expanded = tree.toggle_expand(entry_id);
            if !now_expanded {
                None
            } else {
                let needs_load = matches!(
                    tree.entry(entry_id).map(|e| e.kind),
                    Some(EntryKind::UnloadedDir)
                );
                if needs_load {
                    let abs = tree.entry(entry_id).map(|e| tree.root.join(&e.path));
                    if let Some(em) = tree.entry_mut(entry_id) {
                        em.kind = EntryKind::PendingDir;
                    }
                    abs
                } else {
                    None
                }
            }
        };
        // Trigger #1 — expand toggle.
        self.invalidate_visible_files_cache(wt_ref);
        cx.notify();
        if let Some(abs_path) = to_load {
            self.kick_dir_load(wt_ref, entry_id, abs_path, cx);
        }
    }

    pub(super) fn kick_dir_load(
        &mut self,
        wt_ref: LaneRef,
        parent_id: EntryId,
        abs_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || load_dir(&abs_path),
            move |ws, result, cx| {
                ws.apply_dir_load_result(wt_ref, parent_id, result, DirLoadSource::UserExpand, cx);
            },
        )
        .detach();
    }

    /// Rebuild the gitignore matcher for `wt_ref` on a background thread.
    /// The existing entry stays in place until the new one is ready, so
    /// filtering never lapses during the build.
    pub(super) fn kick_gitignore_build(
        &mut self,
        wt_ref: LaneRef,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || GitignoreSet::build(&root),
            move |ws, gi_set, cx| {
                ws.lane_scoped_mut(wt_ref).files.gitignore = Some(gi_set);
                ws.invalidate_visible_files_cache(wt_ref);
                cx.notify();
            },
        )
        .detach();
    }

    // ------------------------------------------------------------
    // Watcher / event queue / serial reload
    // ------------------------------------------------------------

    /// Create a `FileTreeWatcher` for `wt_ref` and start (or reuse) the
    /// workspace-level polling task that drains every watcher's
    /// `events_rx` once per tick.
    pub(super) fn spawn_files_watcher(
        &mut self,
        wt_ref: LaneRef,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let watcher = match FileTreeWatcher::new(root.clone()) {
            Ok(w) => w,
            Err(e) => {
                let report =
                    ErrorReport::new(crate::surface::strings::error::file_watcher_init_failed())
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&root))
                        .dedup("files.watcher_init")
                        .build();
                self.report_error(report, cx);
                return;
            }
        };
        self.lane_scoped_mut(wt_ref).files.watcher = Some(watcher);
        if self.file_tree.files_watcher_poll.is_none() {
            let task = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    let alive = this
                        .update(cx, |ws, cx| ws.drain_files_watcher_events(cx))
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            });
            self.file_tree.files_watcher_poll = Some(task);
        }
    }

    /// Called once per polling tick. Drains every watcher's queue
    /// without blocking, dispatching each debounced event into the
    /// per-lane reload queue.
    pub(super) fn drain_files_watcher_events(&mut self, cx: &mut Context<Self>) {
        let mut events: Vec<(LaneRef, DebouncedEvent)> = Vec::new();
        for (wt_ref, state) in &self.lane_scoped {
            let Some(watcher) = &state.files.watcher else {
                continue;
            };
            while let Ok(ev) = watcher.events_rx.try_recv() {
                events.push((*wt_ref, ev));
            }
        }
        for (wt_ref, ev) in events {
            self.queue_files_event(wt_ref, ev, cx);
        }
    }

    /// Enqueue one debounced event for `wt_ref`. Inactive lanes
    /// only mark `dirty = true`; active ones append to the reload
    /// queue and kick the drain task.
    pub(in crate::workspace) fn queue_files_event(
        &mut self,
        wt_ref: LaneRef,
        ev: DebouncedEvent,
        cx: &mut Context<Self>,
    ) {
        if wt_ref != self.active {
            if let Some(t) = self.lane_file_tree_mut(wt_ref) {
                t.dirty = true;
            }
            return;
        }
        // Decide whether to refresh *before* the match consumes `ev`. This
        // watcher never carries `.git/` paths (`Batch::absorb` drops them, and
        // `git_ops::watch` owns them instead), so every event but a watcher
        // failure is a real working-tree change.
        let should_refresh_git_status = !matches!(ev, DebouncedEvent::Error(_));
        let root = self.lane_file_tree(wt_ref).map(|t| t.root.clone());
        let bulk_pending = self
            .lane_scoped
            .get(&wt_ref)
            .and_then(|state| state.files.reload_queue.as_ref())
            .is_some_and(|q| q.pending_bulk);
        match ev {
            DebouncedEvent::Bulk => {
                let q = self
                    .lane_scoped_mut(wt_ref)
                    .files
                    .reload_queue
                    .get_or_insert_default();
                q.pending_bulk = true;
                q.pending_parents.clear();
                q.pending_seen.clear();
            }
            DebouncedEvent::Removed { paths } if !bulk_pending => {
                // Apply Remove events directly to avoid the NotFound
                // race a follow-up parent reload would hit. Sibling
                // Modify events arrive as a separate Changed event and
                // reload the parent normally.
                let Some(root) = root.clone() else { return };
                if let Some(tree) = self.lane_file_tree_mut(wt_ref) {
                    // The watcher reports paths in its root's spelling, and
                    // it was started from the same `wt.path` as this tree.
                    for abs in paths {
                        if let Ok(rel) = abs.strip_prefix(&root) {
                            tree.remove_subtree(rel);
                        }
                    }
                }
                self.invalidate_visible_files_cache(wt_ref);
                // `refresh_git_status` below early-returns for non-git
                // lanes, so this direct tree mutation needs its own
                // notify (Pitfall #10).
                cx.notify();
            }
            DebouncedEvent::Removed { .. } => {}
            DebouncedEvent::Changed { paths } if !bulk_pending => {
                let Some(root) = root.clone() else { return };
                // `.gitignore` / `.git/info/exclude` changes invalidate
                // the matcher; detect via filename so nested-ignore
                // tweaks get picked up.
                let mut gitignore_dirty = false;
                for p in &paths {
                    if let Some(name) = p.file_name()
                        && (name == ".gitignore"
                            || (name == "exclude"
                                && p.parent().and_then(|pp| pp.file_name())
                                    == Some(std::ffi::OsStr::new("info"))
                                && p.parent()
                                    .and_then(|pp| pp.parent())
                                    .and_then(|gp| gp.file_name())
                                    == Some(std::ffi::OsStr::new(".git"))))
                    {
                        gitignore_dirty = true;
                    }
                }
                if gitignore_dirty {
                    self.kick_gitignore_build(wt_ref, root.clone(), cx);
                    self.invalidate_visible_files_cache(wt_ref);
                }
                let q = self
                    .lane_scoped_mut(wt_ref)
                    .files
                    .reload_queue
                    .get_or_insert_default();
                for p in paths {
                    let parent = p
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| root.clone());
                    if q.pending_seen.insert(parent.clone()) {
                        q.pending_parents.push_back(parent);
                    }
                }
            }
            DebouncedEvent::Changed { .. } => {}
            DebouncedEvent::Error(msg) => {
                let report = ErrorReport::new(crate::surface::strings::error::file_watcher_error())
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("detail", msg)
                    .dedup("files.watcher_event")
                    .build();
                self.report_error(report, cx);
                return;
            }
        }
        self.kick_files_reload(wt_ref, cx);
        // Watcher events also stale the Git Changes view. Only the working
        // tree moved here, so the refs half stays as it is; the refresh's
        // in-flight guard collapses bursts.
        if should_refresh_git_status {
            self.refresh_worktree_status(wt_ref, cx);
        }
    }
}

mod navigation;
mod reload;
mod walker;
use walker::walk_into;
pub(in crate::workspace) use walker::{VisibleEntry, build_status_index, repo_status_index};

/// True if any path in `ev` lies *outside* a `.git/` directory.
/// Bulk events default to true (path set unknown). Error events
/// never trigger a git refresh.
#[cfg(test)]
mod tests;
