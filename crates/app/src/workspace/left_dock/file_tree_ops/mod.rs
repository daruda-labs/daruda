//! Background file-tree operations + visible-row cache for the left dock
//! Files view.
//!
//! `Workspace::ensure_file_tree` lazily creates the per-lane
//! `FileTree` and kicks a root scan; `toggle_files_expand` flips the
//! expanded set, and on `UnloadedDir → expanded` transitions kicks a
//! `load_dir` task that comes back via `apply_dir_load_result`. All
//! filesystem reads run on `cx.background_executor()` so the UI thread
//! never blocks.
//!
//! `cached_or_rebuild_visible` flattens the tree into the linear list
//! that `uniform_list` consumes; results are memoised in
//! `Workspace::files_visible_cache` so repeated `cx.notify()` cycles do
//! not re-walk the tree. The cache is only invalidated at the seven
//! trigger points listed in the W-7 plan (toggle expand, load result,
//! watcher event (W-7g), focused-file-viewer change (W-7f),
//! activate_lane, git status update, config change). Anything else
//! that calls `cx.notify()` reads the cached `Arc` directly.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::{Context, ScrollStrategy};

use crate::files::gitignore::GitignoreSet;
use crate::files::load::load_dir;
use crate::files::tree::{EntryId, EntryKind, FileTree, FileTreeError, LoadedEntry};
use crate::files::watcher::{DebouncedEvent, FileTreeWatcher};
use crate::lane::availability::{LaneAvailability, classify_dir};
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::FileViewMode;

/// Distinguishes the two `apply_dir_load_result` call sites so the
/// watcher-driven path can stay silent on `NotFound` (the directory was
/// legitimately deleted between the change event and the actual read —
/// expected, not user-actionable).
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
    /// `true` while `kick_files_reload`'s drain task is running. New
    /// events do not need to spawn another task — they just enqueue.
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

        // An unavailable lane root (deleted / unreadable) cannot be
        // scanned — `read_dir` would fail every tick and spam the toast.
        // Skip the root load, watcher spawn, and gitignore build, and
        // tear down any file-tree state built while the directory was
        // still present (see `teardown_unavailable_lane_state`).
        if wt.availability != LaneAvailability::Present {
            self.teardown_unavailable_lane_state(wt_ref);
            return;
        }

        let needs_load = match self.file_tree.file_trees.get(&wt_ref) {
            Some(tree) => tree
                .entry(tree.root_id)
                .map(|e| matches!(e.kind, EntryKind::UnloadedDir))
                .unwrap_or(false),
            None => {
                self.file_tree
                    .file_trees
                    .insert(wt_ref, FileTree::new(root.clone()));
                true
            }
        };

        if needs_load {
            if let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref)
                && let Some(entry) = tree.entry_mut(EntryId(0))
                && matches!(entry.kind, EntryKind::UnloadedDir)
            {
                entry.kind = EntryKind::PendingDir;
            }
            self.invalidate_visible_files_cache(wt_ref);
            self.kick_dir_load(wt_ref, EntryId(0), root.clone(), cx);
        }

        // Start a watcher the first time this lane's tree is
        // touched. The watcher itself is GPUI-free; the polling task
        // belongs to Workspace and is created lazily on demand.
        if !self.file_tree.file_watchers.contains_key(&wt_ref) {
            self.spawn_files_watcher(wt_ref, root.clone(), cx);
        }
        // Build the gitignore matcher once on a background thread.
        // Rebuilt when `.gitignore` changes (see `queue_files_event`).
        if !self.file_tree.files_gitignore_index.contains_key(&wt_ref) {
            self.kick_gitignore_build(wt_ref, root.clone(), cx);
        }
    }

    /// Tear down the file-tree state built while a lane's root was still
    /// present, once that root has flipped to non-`Present` (deleted or
    /// unreadable). Removes the watcher (so it stops firing reload
    /// events against the missing path), the stale tree, the visible
    /// cache, the gitignore matcher, and the pending reload queue.
    /// Per-lane git and cursor state are left untouched — they are reset
    /// by the lane-removal path, not by a transient availability flip.
    ///
    /// Called from two sites: `ensure_file_tree` (the lazy-create path,
    /// which only runs when no tree exists yet — so it catches a lane
    /// that went missing between sessions) and `apply_dir_load_result`
    /// (the watcher-driven path, which catches an *active* lane whose
    /// tree already exists and goes missing mid-session — the case
    /// `ensure_file_tree` never reaches, since render skips it once a
    /// tree is present).
    pub(in crate::workspace) fn teardown_unavailable_lane_state(&mut self, wt_ref: LaneRef) {
        self.file_tree.file_watchers.remove(&wt_ref);
        self.file_tree.file_trees.remove(&wt_ref);
        self.file_tree.files_reload_queues.remove(&wt_ref);
        self.file_tree.files_gitignore_index.remove(&wt_ref);
        self.invalidate_visible_files_cache(wt_ref);
    }

    pub(in crate::workspace) fn toggle_files_expand(
        &mut self,
        wt_ref: LaneRef,
        entry_id: EntryId,
        cx: &mut Context<Self>,
    ) {
        let to_load: Option<PathBuf> = {
            let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref) else {
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

    pub(in crate::workspace) fn kick_dir_load(
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

    /// Rebuild the gitignore matcher for `wt_ref` on a background
    /// thread. The existing entry (if any) stays in place until the new
    /// one is ready, so gitignore filtering never lapses during the build.
    pub(in crate::workspace) fn kick_gitignore_build(
        &mut self,
        wt_ref: LaneRef,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || GitignoreSet::build(&root),
            move |ws, gi_set, cx| {
                ws.file_tree.files_gitignore_index.insert(wt_ref, gi_set);
                ws.invalidate_visible_files_cache(wt_ref);
                cx.notify();
            },
        )
        .detach();
    }

    // ------------------------------------------------------------
    // Watcher / event queue / serial reload (W-7g)
    // ------------------------------------------------------------

    /// Create a `FileTreeWatcher` for `wt_ref` and start (or
    /// reuse) the workspace-level polling task that drains every
    /// watcher's `events_rx` once per tick.
    pub(in crate::workspace) fn spawn_files_watcher(
        &mut self,
        wt_ref: LaneRef,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let watcher = match FileTreeWatcher::new(root.clone()) {
            Ok(w) => w,
            Err(e) => {
                let report = ErrorReport::new("File watcher init failed")
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
        self.file_tree.file_watchers.insert(wt_ref, watcher);
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
    pub(in crate::workspace) fn drain_files_watcher_events(&mut self, cx: &mut Context<Self>) {
        let mut events: Vec<(LaneRef, DebouncedEvent)> = Vec::new();
        for (wt_ref, watcher) in &self.file_tree.file_watchers {
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
            if let Some(t) = self.file_tree.file_trees.get_mut(&wt_ref) {
                t.dirty = true;
            }
            return;
        }
        // Decide whether this event warrants a git-status refresh
        // *before* `ev` is consumed by the match below. Events that
        // only touch paths inside `.git/` are skipped — `git status`
        // itself writes `.git/index` (stat-cache update), which would
        // re-fire fsevents and create a self-sustaining poll loop
        // costing ~3.5% idle CPU. External git activity that matters
        // (commit, checkout) also writes files *outside* `.git/`
        // (working tree), so this filter does not blind us to user
        // actions; the rare `.git/HEAD`-only change is recovered via
        // the manual "Refresh Git Status" command.
        let should_refresh_git_status = event_has_non_git_path(&ev);
        let root = self
            .file_tree
            .file_trees
            .get(&wt_ref)
            .map(|t| t.root.clone());
        let bulk_pending = self
            .file_tree
            .files_reload_queues
            .get(&wt_ref)
            .is_some_and(|q| q.pending_bulk);
        match ev {
            DebouncedEvent::Bulk => {
                let q = self
                    .file_tree
                    .files_reload_queues
                    .entry(wt_ref)
                    .or_default();
                q.pending_bulk = true;
                q.pending_parents.clear();
                q.pending_seen.clear();
            }
            DebouncedEvent::Removed { paths } if !bulk_pending => {
                // Apply notify's Remove(_) events directly — avoids
                // the NotFound race where a follow-up parent reload
                // would fail. Sibling Modify events in the same
                // debounce window arrive as a separate Changed event
                // and trigger the parent reload normally.
                let Some(root) = root.clone() else { return };
                if let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref) {
                    for abs in paths {
                        if let Ok(rel) = abs.strip_prefix(&root) {
                            tree.remove_subtree(rel);
                        }
                    }
                }
                self.invalidate_visible_files_cache(wt_ref);
                // The `refresh_git_status` below early-returns for non-git
                // lanes, so this direct tree mutation needs its own
                // left-dock notify (Pitfall #10).
                cx.notify();
            }
            DebouncedEvent::Removed { .. } => {}
            DebouncedEvent::Changed { paths } if !bulk_pending => {
                let Some(root) = root.clone() else { return };
                // `.gitignore` / `.git/info/exclude` changes invalidate
                // the matcher. Detect via filename so subdir tweaks
                // (Zed-style nested ignore) get picked up later.
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
                    .file_tree
                    .files_reload_queues
                    .entry(wt_ref)
                    .or_default();
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
                let report = ErrorReport::new("File watcher reported error")
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
        // Watcher events also stale the Git Changes view. The
        // refresh's own in-flight guard collapses bursts, but the
        // `should_refresh_git_status` gate skips pure `.git/` noise
        // entirely so the refresh never re-triggers itself.
        if should_refresh_git_status {
            self.refresh_git_status(wt_ref, cx);
        }
    }

    /// Drive the reload queue for `wt_ref`. Idempotent — calling
    /// while a drain task is already running is a no-op (the running
    /// task picks up the new entries on its next iteration).
    pub(in crate::workspace) fn kick_files_reload(
        &mut self,
        wt_ref: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let q = self
            .file_tree
            .files_reload_queues
            .entry(wt_ref)
            .or_default();
        if q.running {
            return;
        }
        if !q.pending_bulk && q.pending_parents.is_empty() {
            return;
        }
        q.running = true;
        cx.spawn(async move |this, cx| {
            loop {
                let task = this
                    .update(cx, |ws, _| {
                        let q = ws.file_tree.files_reload_queues.get_mut(&wt_ref)?;
                        if q.pending_bulk {
                            q.pending_bulk = false;
                            return Some(ReloadTask::Bulk);
                        }
                        if let Some(p) = q.pending_parents.pop_front() {
                            q.pending_seen.remove(&p);
                            return Some(ReloadTask::Parent(p));
                        }
                        q.running = false;
                        None
                    })
                    .ok()
                    .flatten();
                let Some(task) = task else { break };

                match task {
                    ReloadTask::Bulk => {
                        // Reload root + every currently-expanded dir,
                        // sequentially so the queue's serial guarantee
                        // holds.
                        let plan = this
                            .update(cx, |ws, _| {
                                let tree = ws.file_tree.file_trees.get(&wt_ref)?;
                                let mut targets: Vec<(EntryId, PathBuf)> =
                                    vec![(tree.root_id, tree.root.clone())];
                                for &eid in tree.expanded_ids() {
                                    if let Some(e) = tree.entry(eid) {
                                        targets.push((eid, tree.root.join(&e.path)));
                                    }
                                }
                                Some(targets)
                            })
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        for (parent_id, abs) in plan {
                            let abs_clone = abs.clone();
                            let result = cx
                                .background_executor()
                                .spawn(async move { load_dir(&abs_clone) })
                                .await;
                            if let Err(e) = this.update(cx, |ws, cx| {
                                ws.apply_dir_load_result(
                                    wt_ref,
                                    parent_id,
                                    result,
                                    DirLoadSource::WatcherReload,
                                    cx,
                                );
                            }) {
                                daruda_store::observability::log_writer::LogWriter::log(
                                    daruda_store::observability::error_report::ErrorReport::new(
                                        "File tree bulk reload could not reach workspace",
                                    )
                                    .severity(daruda_store::observability::error_report::ErrorSeverity::Info)
                                    .at(file!(), line!())
                                    .with_context("error", format!("{e}"))
                                    .dedup("file_tree.bulk_reload.workspace_dropped")
                                    .build(),
                                );
                            }
                        }
                    }
                    ReloadTask::Parent(abs) => {
                        let parent_id = this
                            .update(cx, |ws, _| {
                                let tree = ws.file_tree.file_trees.get(&wt_ref)?;
                                let rel = abs.strip_prefix(&tree.root).ok()?;
                                tree.id_for_path(rel)
                            })
                            .ok()
                            .flatten();
                        let Some(parent_id) = parent_id else { continue };
                        let abs_clone = abs.clone();
                        let result = cx
                            .background_executor()
                            .spawn(async move { load_dir(&abs_clone) })
                            .await;
                        if let Err(e) = this.update(cx, |ws, cx| {
                            ws.apply_dir_load_result(
                                wt_ref,
                                parent_id,
                                result,
                                DirLoadSource::WatcherReload,
                                cx,
                            );
                        }) {
                            daruda_store::observability::log_writer::LogWriter::log(
                                daruda_store::observability::error_report::ErrorReport::new(
                                    "File tree parent reload could not reach workspace",
                                )
                                .severity(daruda_store::observability::error_report::ErrorSeverity::Info)
                                .at(file!(), line!())
                                .with_context("error", format!("{e}"))
                                .dedup("file_tree.parent_reload.workspace_dropped")
                                .build(),
                            );
                        }
                    }
                }
            }
        })
        .detach();
    }

    /// Replay the queued `dirty` flag for `wt_ref` (called on
    /// activation). When the lane was modified while inactive, a
    /// single Bulk reload covers all changes.
    pub(in crate::workspace) fn replay_files_dirty(
        &mut self,
        wt_ref: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let dirty = self
            .file_tree
            .file_trees
            .get_mut(&wt_ref)
            .map(|t| {
                let was = t.dirty;
                t.dirty = false;
                was
            })
            .unwrap_or(false);
        if !dirty {
            return;
        }
        let q = self
            .file_tree
            .files_reload_queues
            .entry(wt_ref)
            .or_default();
        q.pending_bulk = true;
        q.pending_parents.clear();
        q.pending_seen.clear();
        self.kick_files_reload(wt_ref, cx);
        // Activation after dirty also catches up the Git Changes view.
        self.refresh_git_status(wt_ref, cx);
    }

    /// Open `path` from the active lane's tree in a new tab as a
    /// `PaneContent::File` viewer (Raw mode by default; Markdown opens
    /// in Preview). Same `(lane, path)` re-clicked activates the
    /// existing tab instead of opening another viewer (close via
    /// Cmd+W). Delegates to `open_pane_file_view` so the loading +
    /// invalidation logic stays in one place.
    pub(in crate::workspace) fn open_files_entry(
        &mut self,
        wt_ref: LaneRef,
        path: PathBuf,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.open_pane_file_view(
            wt_ref.lane,
            path,
            /* staged = */ false,
            /* file_status = */ None,
            FileViewMode::Raw,
            window,
            cx,
        );
    }

    pub(in crate::workspace) fn apply_dir_load_result(
        &mut self,
        wt_ref: LaneRef,
        parent_id: EntryId,
        result: Result<Vec<LoadedEntry>, FileTreeError>,
        source: DirLoadSource,
        cx: &mut Context<Self>,
    ) {
        let error_msg = match result {
            Ok(loaded) => {
                if let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref) {
                    tree.apply_dir_load(parent_id, loaded);
                }
                None
            }
            Err(e) => {
                let is_root = self
                    .file_tree
                    .file_trees
                    .get(&wt_ref)
                    .is_some_and(|t| t.root_id == parent_id);
                if let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref)
                    && let Some(entry) = tree.entry_mut(parent_id)
                    && matches!(entry.kind, EntryKind::PendingDir)
                {
                    entry.kind = EntryKind::UnloadedDir;
                }
                if is_root {
                    // The lane root failed to read. Classify the failure:
                    // only a genuine "gone / unusable" kind (NotFound /
                    // PermissionDenied / NotADir) flips the lane to
                    // non-Present and takes the silent teardown path — the
                    // feature's intended suppression of the per-tick toast
                    // spam. A transient/unknown I/O error maps to `Present`
                    // (`From<&FileTreeError>` yields `Present` for `Io`):
                    // the directory likely still exists, so we must NOT
                    // tear down. Surfacing it as a normal Error toast keeps
                    // a real I/O failure visible instead of silently
                    // swallowing it (the no-op `set_*(Present)` otherwise
                    // would).
                    let classified: LaneAvailability = (&e).into();
                    if classified == LaneAvailability::Present {
                        // Transient/unknown failure on a root that is
                        // still (probably) present — keep the lane as-is
                        // and report it like any other dir-read error.
                        Some((
                            format!("Cannot read directory: {e}"),
                            ErrorSeverity::Error,
                            "files.dir_read.root",
                        ))
                    } else {
                        // The root is genuinely gone / unreadable. This is
                        // a detection site: flip the lane's availability so
                        // the file-tree scan / watcher / PTY spawn all
                        // short-circuit, instead of escalating to a
                        // repeating Error toast (the spam this feature
                        // fixes).
                        self.set_lane_availability(wt_ref, classified);
                        // Tear down the watcher + tree immediately.
                        // `ensure_file_tree`'s teardown only runs on the
                        // lazy-create path (no tree yet); for an *active*
                        // lane whose tree already exists and vanishes
                        // mid-session, render never re-calls
                        // `ensure_file_tree`, so without this the watcher
                        // keeps firing bulk reloads → repeated
                        // `apply_dir_load_result` → repaint loop. Safe
                        // ordering: the `tree.entry_mut` borrow above has
                        // ended; the only tree access left
                        // (`invalidate_visible_files_cache` + `cx.notify()`
                        // below) tolerates a removed tree.
                        // The setter wrote non-Present, so this guard is
                        // true whenever the lane still exists; it also
                        // covers the
                        // lane-removed-between-schedule-and-apply case
                        // (`None != Some(Present)` → teardown is a no-op on
                        // already-absent keys).
                        if self.lane_for(wt_ref).map(|l| l.availability)
                            != Some(LaneAvailability::Present)
                        {
                            self.teardown_unavailable_lane_state(wt_ref);
                            // Reconcile the owning project's availability
                            // too: a root that died takes the project
                            // header's live `[+]` with it. Collect the root
                            // path before the `&mut self` setter to avoid a
                            // borrow conflict.
                            if let Some(root) =
                                self.project_for(wt_ref.project).map(|p| p.root.clone())
                            {
                                let a = classify_dir(&root);
                                self.set_project_availability(wt_ref.project, a);
                            }
                        }
                        None
                    }
                } else if matches!(source, DirLoadSource::WatcherReload)
                    && matches!(e, FileTreeError::NotFound)
                {
                    // Watcher-driven NotFound on a child directory is
                    // expected — the directory was deleted between the
                    // change event and the read. The fs watcher will send
                    // a parent-reload event next and the stale entry drops
                    // out naturally.
                    None
                } else if self.lane_for(wt_ref).map(|l| l.availability)
                    != Some(LaneAvailability::Present)
                {
                    // The lane is already non-Present (its root flipped on
                    // an earlier load failure and the tree was torn down).
                    // A child load that was in flight before teardown can
                    // still land here — suppress its Warning toast, since
                    // the empty-state already tells the user the lane is
                    // gone. A genuinely-Present lane with a transient child
                    // error still falls through to the Warning below.
                    None
                } else {
                    Some((
                        format!("Cannot read directory: {e}"),
                        ErrorSeverity::Warning,
                        "files.dir_read",
                    ))
                }
            }
        };
        if let Some((msg, severity, dedup_key)) = error_msg {
            let report = ErrorReport::new("Cannot read directory")
                .severity(severity)
                .at(file!(), line!())
                .with_context("detail", msg)
                .dedup(dedup_key)
                .build();
            self.report_error(report, cx);
        }
        // Trigger #2 — load result (success or revert-on-error).
        self.invalidate_visible_files_cache(wt_ref);
        cx.notify();
    }

    // ------------------------------------------------------------
    // Visible-list cache
    // ------------------------------------------------------------

    /// Drop the cached visible list for `wt_ref`. Trigger sites are
    /// enumerated in the module-level doc comment.
    pub(in crate::workspace) fn invalidate_visible_files_cache(&mut self, wt_ref: LaneRef) {
        self.file_tree.files_visible_cache.remove(&wt_ref);
    }

    /// Return the cached `Arc<Vec<VisibleEntry>>` for `wt_ref`,
    /// rebuilding it from the live tree + git status if missing.
    pub(in crate::workspace) fn cached_or_rebuild_visible(
        &mut self,
        wt_ref: LaneRef,
    ) -> Arc<Vec<VisibleEntry>> {
        if let Some(cached) = self.file_tree.files_visible_cache.get(&wt_ref) {
            return cached.clone();
        }
        let visible = self.build_visible_for(wt_ref);
        let arc = Arc::new(visible);
        self.file_tree
            .files_visible_cache
            .insert(wt_ref, arc.clone());
        arc
    }

    fn build_visible_for(&self, wt_ref: LaneRef) -> Vec<VisibleEntry> {
        let Some(tree) = self.file_tree.file_trees.get(&wt_ref) else {
            return Vec::new();
        };
        let status_index = build_status_index(self.git_status_cache.get(&wt_ref));
        // Keyboard cursor only counts on the active lane; switching
        // lanes clears the cursor.
        let keyboard_focus = if wt_ref == self.active {
            self.file_tree.files_selection
        } else {
            None
        };
        let gitignore = if self.mirrors.files_use_gitignore {
            self.file_tree.files_gitignore_index.get(&wt_ref)
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
        let refs: Vec<_> = self.file_tree.file_trees.keys().copied().collect();
        for wt_ref in refs {
            self.invalidate_visible_files_cache(wt_ref);
        }
        cx.notify();
    }

    // ------------------------------------------------------------
    // Keyboard navigation (W-7i)
    // ------------------------------------------------------------

    /// Move the keyboard cursor by `delta` rows (positive = down) in
    /// the current visible list of the active lane. Wraps to the
    /// first / last row at boundaries.
    pub(in crate::workspace) fn move_files_selection(
        &mut self,
        delta: isize,
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
        let new_index = match cur {
            Some(i) => {
                let len = visible.len() as isize;
                let mut next = (i as isize) + delta;
                next = next.rem_euclid(len);
                next as usize
            }
            None => {
                if delta >= 0 {
                    0
                } else {
                    visible.len() - 1
                }
            }
        };
        let new_id = visible[new_index].entry_id;
        if self.file_tree.files_selection != Some(new_id) {
            self.file_tree.files_selection = Some(new_id);
            self.invalidate_visible_files_cache(wt_ref);
            self.file_tree
                .files_scroll_handle
                .scroll_to_item(new_index, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    /// Enter on the cursor row: file → open in `PaneFileView::Raw`,
    /// directory → toggle expand. Mirrors single-click semantics.
    pub(in crate::workspace) fn activate_files_selection(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let (kind, path) = {
            let Some(tree) = self.file_tree.file_trees.get(&wt_ref) else {
                return;
            };
            let Some(entry) = tree.entry(sel) else {
                return;
            };
            (entry.kind, entry.path.clone())
        };
        if kind.is_dir() {
            self.toggle_files_expand(wt_ref, sel, cx);
        } else {
            self.open_files_entry(wt_ref, path, window, cx);
        }
    }

    /// Right-arrow on the cursor row: if the row is a collapsed dir,
    /// expand it. Otherwise no-op (parent navigation lives in W-7+).
    pub(in crate::workspace) fn expand_at_files_selection(&mut self, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let should_toggle = {
            let Some(tree) = self.file_tree.file_trees.get(&wt_ref) else {
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

    /// Left-arrow on the cursor row: if the row is an expanded dir,
    /// collapse it. Otherwise no-op (move-to-parent lives in W-7+).
    pub(in crate::workspace) fn collapse_at_files_selection(&mut self, cx: &mut Context<Self>) {
        let wt_ref = self.active_ref();
        let Some(sel) = self.file_tree.files_selection else {
            return;
        };
        let should_toggle = {
            let Some(tree) = self.file_tree.file_trees.get(&wt_ref) else {
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

    /// Recursive collapse: drop `entry_id` from the expanded set
    /// along with every descendant currently in it. Useful for
    /// cleaning up a deep subtree with one Alt+click. Recursive
    /// expand is more involved (each layer needs its own async
    /// load) and lands in W-7+; for now Alt+click on a collapsed
    /// dir falls through to the regular toggle.
    pub(in crate::workspace) fn collapse_files_subtree(
        &mut self,
        wt_ref: LaneRef,
        entry_id: EntryId,
        cx: &mut Context<Self>,
    ) {
        let invalidated = {
            let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref) else {
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

mod walker;
use walker::walk_into;
pub(in crate::workspace) use walker::{VisibleEntry, build_status_index};

/// True if any path in `ev` lies *outside* a `.git/` directory.
/// Bulk events default to true (path set unknown). Error events
/// never trigger a git refresh.
fn event_has_non_git_path(ev: &DebouncedEvent) -> bool {
    match ev {
        DebouncedEvent::Bulk => true,
        DebouncedEvent::Error(_) => false,
        DebouncedEvent::Changed { paths } | DebouncedEvent::Removed { paths } => {
            paths.iter().any(|p| !path_is_inside_git_dir(p))
        }
    }
}

fn path_is_inside_git_dir(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(".git"))
}

#[cfg(test)]
mod tests;
