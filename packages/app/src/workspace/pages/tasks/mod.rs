//! The Tasks page's detail: the one Task editor the window holds.
//!
//! The editor lives here rather than in a lane's runtime — a task belongs to
//! no lane, and hiding the page (a lane switch, focusing a pane) must not
//! drop unsaved edits. Only leaving the detail clears it, through
//! [`crate::workspace::Workspace::leave_page_detail_then`].

pub(in crate::workspace) mod editor;

use daruda_store::project::TaskDetailTarget;

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

impl crate::workspace::Workspace {
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
        self.close_page_detail_now(super::detail::PageDetailId::TaskEditor(id), cx);
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
