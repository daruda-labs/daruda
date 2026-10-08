//! What a page shows over its list — its detail — and the one way to leave
//! one. A detail outlives the page being hidden; only leaving it clears it,
//! and leaving asks first when it holds unsaved edits.

use gpui::{App, Context, Window};

use super::Page;
use super::flows::detail::{FlowDetailId, FlowsPage};
use super::tasks::{TaskEditorId, TasksPage};
use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::dirty_items::DirtyItem;

/// What each page remembers while it is not on screen.
#[derive(Default)]
pub(in crate::workspace) struct PageDetails {
    pub tasks: TasksPage,
    pub flows: FlowsPage,
}

/// Names the detail a page shows. Ids are never reused, so a callback that
/// arrives after its detail was replaced finds nothing to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PageDetailId {
    TaskEditor(TaskEditorId),
    Flow(FlowDetailId),
}

impl PageDetailId {
    fn page(self) -> Page {
        match self {
            Self::TaskEditor(_) => Page::Tasks,
            Self::Flow(_) => Page::Flows,
        }
    }
}

impl Workspace {
    /// The detail `page` shows over its list, if any.
    pub(in crate::workspace) fn page_detail_id(&self, page: Page) -> Option<PageDetailId> {
        match page {
            Page::Tasks => self.task_detail_id().map(PageDetailId::TaskEditor),
            Page::Flows => self.flow_detail_id().map(PageDetailId::Flow),
        }
    }

    /// Detail `id` as unsaved work, or `None` when it holds none.
    fn page_detail_dirty(&self, id: PageDetailId, cx: &App) -> Option<DirtyItem> {
        match id {
            PageDetailId::TaskEditor(editor) => {
                DirtyItem::of_task_editor(editor, self.task_editor(editor)?, cx)
            }
            PageDetailId::Flow(detail) => {
                let (_, view) = self.flow_graph(detail)?;
                DirtyItem::of_flow_graph(detail, &view, cx)
            }
        }
    }

    /// Leave `page`'s detail, then run `next`. Unsaved edits ask first —
    /// Save goes on only if the save landed, Discard drops them, Cancel
    /// stays. Back, Escape, closing and opening another detail all come here.
    pub(in crate::workspace) fn leave_page_detail_then(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
        next: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(id) = self.page_detail_id(page) else {
            next(self, window, cx);
            return;
        };
        let Some(item) = self.page_detail_dirty(id, cx) else {
            self.close_page_detail_now(id, cx);
            next(self, window, cx);
            return;
        };
        let save = item.save_label();
        let discard = strings::task::edit_discard();
        let cancel = strings::common::btn_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &item.leave_heading(),
            None,
            &[save.as_str(), discard.as_str(), cancel.as_str()],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: the window may close before the answer arrives
            let _ = this.update_in(cx, |ws, window, cx| {
                // Answered for a detail since replaced: nothing to leave.
                if ws.page_detail_id(page) != Some(id) {
                    return;
                }
                let leave = match answer {
                    0 => ws.save_dirty_target(item.target, window, cx),
                    1 => true,
                    _ => false,
                };
                if leave {
                    ws.close_page_detail_now(id, cx);
                    next(ws, window, cx);
                }
            });
        })
        .detach();
    }

    /// Leave the detail of the page on screen, asking first if it holds
    /// edits. `false` when no page is shown or it shows only its list, so
    /// the caller closes what it would.
    pub(in crate::workspace) fn close_page_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(page) = self.active_page() else {
            return false;
        };
        if self.page_detail_id(page).is_none() {
            return false;
        }
        self.leave_page_detail_then(page, window, cx, |_, _, _| {});
        true
    }

    /// Drop detail `id` without asking. A stale id drops nothing.
    pub(in crate::workspace) fn close_page_detail_now(
        &mut self,
        id: PageDetailId,
        cx: &mut Context<Self>,
    ) {
        if self.page_detail_id(id.page()) != Some(id) {
            return;
        }
        self.mutate_durable(cx, |ws, _| match id {
            PageDetailId::TaskEditor(_) => ws.pages.tasks.detail = None,
            PageDetailId::Flow(_) => ws.pages.flows.detail = None,
        });
        cx.notify();
    }
}
