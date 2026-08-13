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
//! `uniform_list` consumes, memoised in `files_visible_cache`. The cache
//! is invalidated only at fixed trigger points (toggle expand, load
//! result, watcher event, focused-file-viewer change, activate_lane, git
//! status update, config change); other `cx.notify()` calls read the
//! cached `Arc` directly.

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

        // Start a watcher on first touch. The watcher is GPUI-free; the
        // polling task belongs to Workspace and is created lazily.
        if !self.file_tree.file_watchers.contains_key(&wt_ref) {
            self.spawn_files_watcher(wt_ref, root.clone(), cx);
        }
        // Build the gitignore matcher once on a background thread;
        // rebuilt when `.gitignore` changes.
        if !self.file_tree.files_gitignore_index.contains_key(&wt_ref) {
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

    /// Rebuild the gitignore matcher for `wt_ref` on a background thread.
    /// The existing entry stays in place until the new one is ready, so
    /// filtering never lapses during the build.
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
    // Watcher / event queue / serial reload
    // ------------------------------------------------------------

    /// Create a `FileTreeWatcher` for `wt_ref` and start (or reuse) the
    /// workspace-level polling task that drains every watcher's
    /// `events_rx` once per tick.
    pub(in crate::workspace) fn spawn_files_watcher(
        &mut self,
        wt_ref: LaneRef,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let watcher = match FileTreeWatcher::new(root.clone()) {
            Ok(w) => w,
            Err(e) => {
                let report =
                    ErrorReport::new(crate::surface::strings::error_file_watcher_init_failed())
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
        // Decide whether to refresh git status *before* the match
        // consumes `ev`. Pure-`.git/` events are skipped: `git status`
        // writes `.git/index`, which would re-fire fsevents into a
        // self-sustaining poll loop (~3.5% idle CPU). Meaningful git
        // activity (commit, checkout) also touches the working tree, so
        // this doesn't blind us; a rare `.git/HEAD`-only change is
        // recovered via the manual "Refresh Git Status" command.
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
                // Apply Remove events directly to avoid the NotFound
                // race a follow-up parent reload would hit. Sibling
                // Modify events arrive as a separate Changed event and
                // reload the parent normally.
                let Some(root) = root.clone() else { return };
                if let Some(tree) = self.file_tree.file_trees.get_mut(&wt_ref) {
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
                let report = ErrorReport::new(crate::surface::strings::error_file_watcher_error())
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
        // Watcher events also stale the Git Changes view. The refresh's
        // in-flight guard collapses bursts; the `should_refresh_git_status`
        // gate skips pure `.git/` noise so it never re-triggers itself.
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
                        // Reload root + every expanded dir sequentially so
                        // the queue's serial guarantee holds.
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

    /// Open `path` in a new tab as a `PaneContent::File` viewer (Raw by
    /// default; Markdown in Preview). Re-clicking the same `(lane, path)`
    /// reactivates the existing tab. Delegates to `open_pane_file_view`,
    /// which derives the pane's git `file_status` itself.
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
                    // Root read failed. Classify: only a genuine "gone /
                    // unusable" kind (NotFound / PermissionDenied /
                    // NotADir) flips the lane to non-Present and takes the
                    // silent teardown path (suppressing per-tick toast
                    // spam). A transient/unknown I/O error maps to
                    // `Present`, so we must NOT tear down — surface it as a
                    // normal Error toast so a real failure stays visible.
                    let classified: LaneAvailability = (&e).into();
                    if classified == LaneAvailability::Present {
                        // Transient/unknown failure on a still-present
                        // root — keep the lane and report like any
                        // dir-read error.
                        Some((
                            crate::surface::strings::error_cannot_read_directory_detail(
                                &e.to_string(),
                            ),
                            ErrorSeverity::Error,
                            "files.dir_read.root",
                        ))
                    } else {
                        // Root genuinely gone/unreadable: flip
                        // availability so file-tree scan / watcher / PTY
                        // spawn all short-circuit instead of repeating an
                        // Error toast.
                        self.set_lane_availability(wt_ref, classified);
                        // Tear down watcher + tree now: for an active lane
                        // whose tree exists, render never re-calls
                        // `ensure_file_tree`, so skipping this leaves the
                        // watcher looping reload → repaint. Safe — the
                        // `entry_mut` borrow above has ended; the guard
                        // also covers the lane-removed-between-schedule-
                        // and-apply race (`None != Some(Present)`).
                        if self.lane_for(wt_ref).map(|l| l.availability)
                            != Some(LaneAvailability::Present)
                        {
                            self.teardown_unavailable_lane_state(wt_ref);
                            // Reconcile the owning project's availability
                            // too — a dead root takes the project header's
                            // live `[+]` with it. Collect the root path
                            // before the `&mut self` setter (borrow).
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
                    // Watcher-driven NotFound on a child is expected — it
                    // was deleted between the change event and the read.
                    // A parent-reload event follows and the stale entry
                    // drops out naturally.
                    None
                } else if self.lane_for(wt_ref).map(|l| l.availability)
                    != Some(LaneAvailability::Present)
                {
                    // Lane already non-Present (root flipped earlier, tree
                    // torn down). A child load in flight before teardown
                    // can still land here — suppress its Warning toast
                    // since the empty-state already says the lane is gone.
                    // A Present lane with a transient child error still
                    // falls through to the Warning below.
                    None
                } else {
                    Some((
                        crate::surface::strings::error_cannot_read_directory_detail(&e.to_string()),
                        ErrorSeverity::Warning,
                        "files.dir_read",
                    ))
                }
            }
        };
        if let Some((msg, severity, dedup_key)) = error_msg {
            let report = ErrorReport::new(crate::surface::strings::error_cannot_read_directory())
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
    // Keyboard navigation
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
        let (kind, path, tree_root) = {
            let Some(tree) = self.file_tree.file_trees.get(&wt_ref) else {
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
            self.open_files_entry(wt_ref, tree_root.join(&path), window, cx);
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

    /// Left-arrow on the cursor row: collapse it if it's an expanded dir,
    /// else no-op.
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
