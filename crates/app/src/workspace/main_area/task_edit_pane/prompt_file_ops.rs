//! The task's prompt file while a TaskEdit pane is open: watching it,
//! reconciling an external edit with the form, and opening it.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::tasks::Task;
use gpui::{Context, Window};

use super::state::normalize_newlines;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Dynamically install the prompt-file FS watcher on a TaskEdit
    /// pane that's still open when its task transitions Backlog →
    /// Running. At pane-open time the lane didn't exist
    /// yet so `install_prompt_watcher` returned `None`; `start_task`
    /// just wrote the file, so the watcher can finally subscribe.
    /// No-op when the pane is closed, when there's already a watcher
    /// attached, or when the task still has no lane.
    pub(in crate::workspace) fn attach_prompt_watcher_if_pane_open(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Starting a task activates its new lane; the authoring tab stays
        // in the source lane, and may be open in more than one lane.
        let panes: Vec<_> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .filter_map(|pane| {
                let te = pane.task_edit_content()?;
                (te.task_id.as_deref() == Some(task_id) && te._prompt_watcher.is_none())
                    .then_some(pane.id)
            })
            .collect();
        let task = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned();
        for pane_id in panes {
            let (handle, pump) = install_prompt_watcher(task.as_ref(), pane_id, window, cx);
            if let Some(te) = self.task_edit_content_mut_for(pane_id) {
                te._prompt_watcher = handle;
                te._prompt_pump = pump;
            }
        }
    }
}

/// The on-disk prompt file for `task` — only meaningful once the task has
/// been started (i.e. has a lane). Returns `None` for Backlog / drafts.
fn prompt_file_path_for(task: &Task) -> Option<std::path::PathBuf> {
    let wt = task.state.worktree_path()?;
    Some(daruda_store::tasks::existing_prompt_file_path(task, wt))
}

/// Install the watcher and pump when the task has a prompt file on disk.
pub(super) fn install_prompt_watcher(
    initial: Option<&Task>,
    pane_id: PaneId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> (
    Option<crate::workspace::main_area::prompt_watcher::PromptFileWatcherHandle>,
    Option<gpui::Task<()>>,
) {
    let Some(task) = initial else {
        return (None, None);
    };
    let Some(path) = prompt_file_path_for(task) else {
        return (None, None);
    };
    if !path.exists() {
        return (None, None);
    }

    let (events_rx, handle) = crate::workspace::main_area::prompt_watcher::spawn(path.clone());
    let path_for_pump = path.clone();
    let pump = cx.spawn_in(window, async move |this, cx| {
        const POLL: std::time::Duration = std::time::Duration::from_millis(100);
        'outer: loop {
            cx.background_executor().timer(POLL).await;
            loop {
                match events_rx.try_recv() {
                    Ok(()) => {
                        // Coalesce multiple debounce-window signals so a
                        // burst still results in a single dispatch.
                        while events_rx.try_recv().is_ok() {}
                        let path_for_dispatch = path_for_pump.clone();
                        if this
                            .update_in(cx, |ws, window, cx| {
                                ws.handle_prompt_file_changed(
                                    pane_id,
                                    path_for_dispatch,
                                    window,
                                    cx,
                                );
                            })
                            .is_err()
                        {
                            break 'outer;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'outer,
                }
            }
        }
    });

    (Some(handle), Some(pump))
}

impl Workspace {
    /// Dispatched by the prompt-file watcher when an external editor
    /// rewrites `<wt>/.daruda/task-<id>.md`. Reloads the editor
    /// silently when the pane is clean; surfaces a conflict prompt
    /// (Use disk version / Keep my version / Diff) when the pane is
    /// dirty.
    pub(super) fn handle_prompt_file_changed(
        &mut self,
        pane_id: PaneId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A transient atomic-rename mid-flight is normal here, but a
        // persistent failure (permission flip, unmount) silently
        // wedges the watcher — leave a single Info breadcrumb so the
        // condition is visible in the NDJSON log without yelling at
        // the user via toast.
        let disk_content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new(crate::surface::strings::error_prompt_watcher_read_failed())
                        .severity(ErrorSeverity::Info)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path))
                        .dedup("tasks.prompt_watcher.read")
                        .build(),
                );
                return;
            }
        };

        let Some(pane) = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .find(|p| p.id == pane_id)
        else {
            return;
        };
        let Some(te) = pane.task_edit_content() else {
            return;
        };
        let prompt_entity = te.prompt_state.clone();
        let title = pane.title(cx);
        let is_dirty = te.is_dirty(cx);

        // If the disk content already matches what's in the editor
        // (modulo CRLF), this is almost certainly a save-side echo
        // from our own `write_prompt_file`. Don't bother the user —
        // just re-baseline so the pane stays clean.
        let editor_normalized =
            normalize_newlines(prompt_entity.read(cx).text().to_string().as_str());
        let disk_normalized = normalize_newlines(&disk_content);
        if editor_normalized == disk_normalized {
            if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
                te.saved_snapshot.prompt = disk_content;
            }
            return;
        }

        if !is_dirty {
            self.reload_prompt_from_disk(pane_id, prompt_entity, disk_content, window, cx);
            return;
        }

        // Dirty — surface a 3-button platform prompt and route the
        // answer back into reload / no-op / diff.
        let heading = format!(
            "{}{}{}",
            crate::surface::strings::PROMPT_WATCHER_HEADING_PREFIX,
            title,
            crate::surface::strings::prompt_watcher_heading_suffix(),
        );
        let prompt_detail = crate::surface::strings::prompt_watcher_detail();
        let prompt_use_disk = crate::surface::strings::prompt_watcher_use_disk();
        let prompt_keep_mine = crate::surface::strings::prompt_watcher_keep_mine();
        let prompt_diff = crate::surface::strings::prompt_watcher_diff();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &heading,
            Some(prompt_detail.as_str()),
            &[
                prompt_use_disk.as_str(),
                prompt_keep_mine.as_str(),
                prompt_diff.as_str(),
            ],
            cx,
        );

        let path_for_diff = path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    let Some(pane) = this.active_runtime().panes.iter().find(|p| p.id == pane_id)
                    else {
                        return;
                    };
                    let Some(te) = pane.task_edit_content() else {
                        return;
                    };
                    let prompt_entity = te.prompt_state.clone();
                    this.reload_prompt_from_disk(
                        pane_id,
                        prompt_entity,
                        disk_content.clone(),
                        window,
                        cx,
                    );
                }
                1 => {} // Keep my version — leave editor untouched
                2 => {
                    // Split the TaskEdit pane's tab to the right with
                    // the disk version so the user sees both at once.
                    this.open_disk_file_for_diff(pane_id, path_for_diff.clone(), window, cx);
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Overwrite the pane's prompt editor with `content` and rebaseline
    /// the dirty snapshot so the pane no longer reads as dirty.
    fn reload_prompt_from_disk(
        &mut self,
        pane_id: PaneId,
        prompt_entity: gpui::Entity<gpui_component::input::InputState>,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        prompt_entity.update(cx, |state, cx| state.set_value(content.clone(), window, cx));
        if let Some(te) = self.task_edit_content_mut_for_pane(pane_id) {
            te.saved_snapshot.prompt = content;
        }
        cx.notify();
    }

    /// Open `<wt>/.daruda/task-<id>.md` in a fresh file viewer
    /// tab (`[📄 Open file]` button). No-op for tasks that
    /// haven't been started yet — Backlog tasks have no lane
    /// path, and Started tasks whose prompt file disappeared (e.g.
    /// manual delete) silently bail rather than open a viewer onto a
    /// non-existent file. The button itself is disabled in those
    /// states so this is defensive only.
    pub(super) fn open_task_prompt_file(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned()
        else {
            return;
        };
        let Some(path) = prompt_file_path_for(&task) else {
            return;
        };
        if !path.exists() {
            let report = ErrorReport::new(crate::surface::strings::error_prompt_file_not_found())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("tasks.open_prompt_file.missing")
                .build();
            self.report_error(report, cx);
            return;
        }
        let Some(wt_id) = self.lane_containing(&path) else {
            let report =
                ErrorReport::new(crate::surface::strings::error_prompt_file_outside_lanes())
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&path))
                    .dedup("tasks.open_prompt_file.no_lane")
                    .build();
            self.report_error(report, cx);
            return;
        };
        let wt_ref = daruda_store::project::LaneRef {
            project: self.active.project,
            lane: wt_id,
        };
        self.open_files_entry(
            wt_ref,
            path,
            crate::workspace::main_area::tab_ops::OpenIntent::Enter,
            window,
            cx,
        );
    }

    /// Helper used by the conflict prompt's `[Diff]` branch.
    /// Opens `path` in a file viewer pane *split to the right of* the
    /// owning TaskEdit pane so the user sees the in-pane editor on
    /// the left and the disk version on the right simultaneously
    /// The two-pane layout lets the user compare in-pane edits against
    /// the on-disk version side-by-side. Falls back silently when the
    /// path isn't inside any known lane.
    fn open_disk_file_for_diff(
        &mut self,
        pane_id: PaneId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_containing(&path) else {
            return;
        };
        self.open_file_split_right(wt, path, pane_id, window, cx);
    }
}

impl Workspace {
    /// The active lane `path` lies in — the deepest, so a worktree nested
    /// inside another lane's checkout wins over its parent. Spellings are
    /// compared as one place, since a prompt path may come through a symlink.
    fn lane_containing(&self, path: &std::path::Path) -> Option<daruda_store::project::LaneId> {
        self.active_lanes()
            .iter()
            .filter(|w| daruda_core::path::is_within(path, &w.path))
            .max_by_key(|w| w.path.components().count())
            .map(|w| w.id)
    }
}
