//! The Tasks page's detail: the one Task editor the window holds.
//!
//! The editor lives here rather than in a lane's runtime — a task belongs to
//! no lane, and hiding the page (a lane switch, focusing a pane) must not
//! drop unsaved edits. Only leaving the detail clears it, through
//! [`Workspace::leave_task_detail_then`].

pub(in crate::workspace) mod editor;

use daruda_store::project::TaskDetailTarget;

use crate::surface::strings;
use editor::state::TaskEditContent;

/// Names the Task editor. Distinct from a pane id: the editor is not a pane,
/// and a late callback holding an older id finds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct TaskEditorId(pub u64);

pub(in crate::workspace) struct TaskDetail {
    pub id: TaskEditorId,
    pub editor: TaskEditContent,
}

/// At most one editor: opening another goes through the leave guard first.
#[derive(Default)]
pub(in crate::workspace) struct TasksPage {
    pub detail: Option<TaskDetail>,
}

/// What each page remembers while it is not on screen.
#[derive(Default)]
pub(in crate::workspace) struct PageDetails {
    pub tasks: TasksPage,
}

impl crate::workspace::Workspace {
    /// Leave the Tasks page's detail, then run `next`. Unsaved edits ask
    /// first — Save (Save Draft for a new task) goes on only if the save
    /// landed, Discard drops them, Cancel stays. The one way an editor closes,
    /// so Back, Escape, closing and opening another task all ask the same.
    pub(in crate::workspace) fn leave_task_detail_then(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
        next: impl FnOnce(&mut Self, &mut gpui::Window, &mut gpui::Context<Self>) + 'static,
    ) {
        let Some(detail) = self.pages.tasks.detail.as_ref() else {
            next(self, window, cx);
            return;
        };
        if !detail.editor.is_dirty(cx) {
            self.drop_task_detail(cx);
            next(self, window, cx);
            return;
        }
        let id = detail.id;
        let is_draft = detail.editor.task_id.is_none();
        let can_save = detail.editor.can_save(cx);
        let heading = if is_draft {
            strings::task::edit_discard_draft_prompt().to_string()
        } else {
            strings::task::edit_save_prompt(detail.editor.title())
        };
        let save_label = if is_draft {
            strings::task::edit_save_draft()
        } else {
            strings::common::btn_save()
        };
        let discard = strings::task::edit_discard();
        let cancel = strings::common::btn_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &heading,
            None,
            &[save_label.as_str(), discard.as_str(), cancel.as_str()],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: the window may close before the answer arrives
            let _ = this.update_in(cx, |ws, window, cx| {
                // Answered for an editor since replaced: nothing to leave.
                if ws.pages.tasks.detail.as_ref().map(|d| d.id) != Some(id) {
                    return;
                }
                let leave = match answer {
                    // An invalid form cannot save; the editor stays.
                    0 => can_save && ws.commit_task_editor(id, window, cx).is_some(),
                    1 => true,
                    _ => false,
                };
                if leave {
                    ws.drop_task_detail(cx);
                    next(ws, window, cx);
                }
            });
        })
        .detach();
    }

    /// Close what the Tasks page shows on top of its list — the editor —
    /// asking first if it holds edits. `false` when the page is not on
    /// screen or shows only the list, so the caller closes what it would.
    pub(in crate::workspace) fn close_page_detail(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.active_page() != Some(super::Page::Tasks) || self.pages.tasks.detail.is_none() {
            return false;
        }
        self.leave_task_detail_then(window, cx, |_, _, _| {});
        true
    }

    /// The editor the Tasks page holds, if any.
    pub(in crate::workspace) fn task_detail_id(&self) -> Option<TaskEditorId> {
        self.pages.tasks.detail.as_ref().map(|detail| detail.id)
    }

    /// What the editor shows, as the workspace file records it. A draft
    /// whose form names no project has nothing to reopen into.
    pub(in crate::workspace) fn task_detail_target(
        &self,
        cx: &gpui::App,
    ) -> Option<TaskDetailTarget> {
        let editor = &self.pages.tasks.detail.as_ref()?.editor;
        match &editor.task_id {
            Some(id) => Some(TaskDetailTarget::Task { id: id.clone() }),
            None => editor
                .project(cx)
                .map(|project| TaskDetailTarget::NewDraft { project }),
        }
    }

    /// Close editor `id` without asking — for a save that just landed, so
    /// there is nothing to lose. A stale id closes nothing.
    pub(in crate::workspace) fn close_task_detail(
        &mut self,
        id: TaskEditorId,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.task_detail_id() == Some(id) {
            self.drop_task_detail(cx);
        }
    }

    /// Drop the editor — its watcher and subscriptions go with it.
    fn drop_task_detail(&mut self, cx: &mut gpui::Context<Self>) {
        self.mutate_durable(cx, |ws, _| ws.pages.tasks.detail = None);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use daruda_store::tasks::Task;
    use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

    use crate::agent::tasks_global::GlobalTasks;
    use crate::workspace::tests::build_workspace;

    fn add_task(title: &str, cx: &mut gpui::Context<crate::workspace::Workspace>) -> String {
        let task = Task::new(
            daruda_store::project::ProjectUuid::default(),
            title.into(),
            String::new(),
            None,
        );
        let id = task.id.clone();
        cx.update_global::<GlobalTasks, _>(|g, _| {
            g.add(task);
        });
        id
    }

    #[gpui::test]
    fn reopening_the_same_task_shows_the_same_editor(cx: &mut TestAppContext) {
        let (window, ws) = build_workspace(cx);
        cx.update_window(window.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let task = add_task("Kept", cx);
                ws.open_task_editor(Some(task.clone()), window, cx);
                let first = ws.task_detail_id().expect("the editor opened");
                ws.open_task_editor(Some(task), window, cx);
                assert_eq!(ws.task_detail_id(), Some(first), "not rebuilt");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn opening_another_task_over_unsaved_edits_asks_first(cx: &mut TestAppContext) {
        let (window, ws) = build_workspace(cx);
        let draft = cx
            .update_window(window.into(), |_, window, cx| {
                ws.update(cx, |ws, cx| {
                    ws.open_task_editor(None, window, cx);
                    let draft = ws.task_detail_id().expect("the draft opened");
                    let title = ws.task_editor(draft).unwrap().title_input.clone();
                    title.update(cx, |s, cx| s.set_value("Unsaved", window, cx));
                    let other = add_task("Other", cx);
                    ws.open_task_editor(Some(other), window, cx);
                    draft
                })
            })
            .unwrap();
        assert!(cx.has_pending_prompt(), "unsaved edits are asked about");
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.task_detail_id(), Some(draft), "nothing replaced it yet");
        });
    }
}
