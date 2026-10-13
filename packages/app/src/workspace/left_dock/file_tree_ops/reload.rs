//! Reload-queue drain, dirty-flag replay, and applying background
//! directory-load results to the lane's file tree.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::files::load::load_dir;
use crate::files::tree::{EntryId, EntryKind, FileTreeError, LoadedEntry};
use crate::lane::availability::{LaneAvailability, classify_dir};
use crate::workspace::Workspace;

use super::{DirLoadSource, ReloadTask};

impl Workspace {
    /// Drive the reload queue for `wt_ref`. Idempotent — calling
    /// while a drain task is already running is a no-op (the running
    /// task picks up the new entries on its next iteration).
    pub(in crate::workspace::left_dock) fn kick_files_reload(
        &mut self,
        wt_ref: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let q = self
            .lane_scoped_mut(wt_ref)
            .files
            .reload_queue
            .get_or_insert_default();
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
                        let q = ws.lane_scoped.get_mut(&wt_ref)?.files.reload_queue.as_mut()?;
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
                                let tree = ws.lane_file_tree(wt_ref)?;
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
                                let tree = ws.lane_file_tree(wt_ref)?;
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
            .lane_file_tree_mut(wt_ref)
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
            .lane_scoped_mut(wt_ref)
            .files
            .reload_queue
            .get_or_insert_default();
        q.pending_bulk = true;
        q.pending_parents.clear();
        q.pending_seen.clear();
        self.kick_files_reload(wt_ref, cx);
        // Activation after dirty also catches up the Git Changes view.
        self.refresh_git_status(wt_ref, cx);
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
                if let Some(tree) = self.lane_file_tree_mut(wt_ref) {
                    tree.apply_dir_load(parent_id, loaded);
                }
                None
            }
            Err(e) => {
                let is_root = self
                    .lane_file_tree(wt_ref)
                    .is_some_and(|t| t.root_id == parent_id);
                if let Some(tree) = self.lane_file_tree_mut(wt_ref)
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
                            crate::surface::strings::error::cannot_read_directory_detail(
                                e.to_string(),
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
                        crate::surface::strings::error::cannot_read_directory_detail(e.to_string()),
                        ErrorSeverity::Warning,
                        "files.dir_read",
                    ))
                }
            }
        };
        if let Some((msg, severity, dedup_key)) = error_msg {
            let report = ErrorReport::new(crate::surface::strings::error::cannot_read_directory())
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
}
