//! The task's prompt file while its editor is open: watching it,
//! reconciling an external edit with the form, and opening it.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::tasks::Task;
use gpui::{Context, Window};

use super::TaskEditorId;
use super::state::normalize_newlines;
use crate::workspace::Workspace;

impl Workspace {
    /// Watch the prompt file of a task that just started while its editor
    /// was open — when the editor opened there was no lane and so no file.
    /// No-op when the editor is closed, already watching, or still laneless.
    pub(in crate::workspace) fn attach_prompt_watcher_if_editor_open(
        &mut self,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Starting a task activates its new lane; the editor stays on the
        // Tasks page, hidden until the user comes back to it.
        let Some(editor_id) = self.pages.tasks.detail.as_ref().and_then(|detail| {
            (detail.editor.task_id.as_deref() == Some(task_id)
                && detail.editor._prompt_watcher.is_none())
            .then_some(detail.id)
        }) else {
            return;
        };
        let task = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .get(task_id)
            .cloned();
        let (handle, pump) = install_prompt_watcher(task.as_ref(), editor_id, window, cx);
        if let Some(te) = self.task_editor_mut(editor_id) {
            te._prompt_watcher = handle;
            te._prompt_pump = pump;
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
    editor_id: TaskEditorId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> (
    Option<super::prompt_watcher::PromptFileWatcherHandle>,
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

    let (events_rx, handle) = super::prompt_watcher::spawn(path.clone());
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
                                    editor_id,
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
    /// silently when the form is clean; surfaces a conflict prompt
    /// (Use disk version / Keep my version / Diff) when it is dirty.
    pub(super) fn handle_prompt_file_changed(
        &mut self,
        editor_id: TaskEditorId,
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
                    ErrorReport::new(crate::surface::strings::error::prompt_watcher_read_failed())
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

        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let prompt_entity = te.prompt_state.clone();
        let title = te.cached_title.clone();
        let is_dirty = te.is_dirty(cx);

        // Disk content matching the editor (modulo CRLF) is the echo of
        // Start's own `write_prompt_file`: re-baseline instead of asking.
        let editor_normalized =
            normalize_newlines(prompt_entity.read(cx).text().to_string().as_str());
        let disk_normalized = normalize_newlines(&disk_content);
        if editor_normalized == disk_normalized {
            if let Some(te) = self.task_editor_mut(editor_id) {
                te.saved_snapshot.prompt = disk_content;
            }
            return;
        }

        if !is_dirty {
            self.reload_prompt_from_disk(editor_id, prompt_entity, disk_content, window, cx);
            return;
        }

        // Dirty — surface a 3-button platform prompt and route the
        // answer back into reload / no-op / diff.
        let heading = crate::surface::strings::task::watcher_heading(&title);
        let prompt_detail = crate::surface::strings::task::watcher_detail();
        let prompt_use_disk = crate::surface::strings::task::watcher_use_disk();
        let prompt_keep_mine = crate::surface::strings::task::watcher_keep_mine();
        let prompt_diff = crate::surface::strings::task::watcher_diff();
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

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    let Some(te) = this.task_editor(editor_id) else {
                        return;
                    };
                    let prompt_entity = te.prompt_state.clone();
                    this.reload_prompt_from_disk(
                        editor_id,
                        prompt_entity,
                        disk_content.clone(),
                        window,
                        cx,
                    );
                }
                1 => {} // Keep my version — leave editor untouched
                2 => this.show_prompt_disk_copy(editor_id, &disk_content, window, cx),
                _ => {}
            });
        })
        .detach();
    }

    /// Overwrite the prompt with `content` and rebaseline the dirty
    /// snapshot so the form no longer reads as dirty.
    fn reload_prompt_from_disk(
        &mut self,
        editor_id: TaskEditorId,
        prompt_entity: gpui::Entity<gpui_component::input::InputState>,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        prompt_entity.update(cx, |state, cx| state.set_value(content.clone(), window, cx));
        if let Some(te) = self.task_editor_mut(editor_id) {
            // The prompt now is the disk version; a copy beside it says nothing.
            te.disk_copy = None;
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
            let report = ErrorReport::new(crate::surface::strings::error::prompt_file_not_found())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("tasks.open_prompt_file.missing")
                .build();
            self.report_error(report, cx);
            return;
        }
        // A task's worktree may be any lane of any project, not the one on
        // screen: the prompt opens where it lives, or for reference if no
        // lane holds it any more.
        self.open_linked_file(path, window, cx);
    }

    /// The conflict prompt's `[Diff]`: put the disk version beside the
    /// prompt, read-only, so the two can be compared in place.
    pub(super) fn show_prompt_disk_copy(
        &mut self,
        editor_id: TaskEditorId,
        disk_content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.task_editor(editor_id).is_none() {
            return;
        }
        let copy = crate::ui::make_markdown_prose_state(
            disk_content,
            "",
            crate::ui::theme::TASK_EDIT_PROMPT_ROWS,
            window,
            cx,
        );
        copy.update(cx, |state, cx| state.set_disabled(true, cx));
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.disk_copy = Some(copy);
        }
        cx.notify();
    }

    pub(super) fn close_prompt_disk_copy(
        &mut self,
        editor_id: TaskEditorId,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.disk_copy = None;
        }
        cx.notify();
    }
}
