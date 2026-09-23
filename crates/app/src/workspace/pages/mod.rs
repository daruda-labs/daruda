//! Task and flow pages, independent of the right utility dock.

use daruda_store::project::RightDockView;
use gpui::{Context, ScrollHandle, Window};

use super::Workspace;
use crate::surface::strings;
use crate::ui::icons;

pub(super) mod render;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Page {
    Tasks,
    Flows,
}

impl Page {
    pub fn from_legacy(view: RightDockView) -> Option<Self> {
        match view {
            RightDockView::Tasks => Some(Self::Tasks),
            RightDockView::Flows => Some(Self::Flows),
            _ => None,
        }
    }

    /// Retain the existing serialized discriminants so saved layouts migrate
    /// without rewriting task data or widening the workspace storage schema.
    pub fn persisted_view(self) -> RightDockView {
        match self {
            Self::Tasks => RightDockView::Tasks,
            Self::Flows => RightDockView::Flows,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Tasks => strings::right_panel_tab_tasks(),
            Self::Flows => strings::right_panel_tab_flows(),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Tasks => icons::TASKS,
            Self::Flows => icons::FLOWS,
        }
    }
}

pub(in crate::workspace) struct PageState {
    pub page: Page,
    pub scroll: ScrollHandle,
}

impl PageState {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            scroll: ScrollHandle::new(),
        }
    }
}

impl Workspace {
    pub(in crate::workspace) fn active_page(&self) -> Option<Page> {
        self.workspace_page.as_ref().map(|state| state.page)
    }

    pub(in crate::workspace) fn show_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.active_page() == Some(page) {
            return;
        }
        self.mutate_durable(cx, |ws, _| {
            ws.workspace_page = Some(PageState::new(page));
            ws.main_area.pending_resize = true;
        });
        cx.notify();
    }

    pub(in crate::workspace) fn open_page(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_page(page, cx);
        self.focus_handle.focus(window, cx);
    }

    pub(in crate::workspace) fn close_page(&mut self, cx: &mut Context<Self>) {
        if self.workspace_page.is_some() {
            self.mutate_durable(cx, |ws, _| {
                ws.workspace_page = None;
                ws.main_area.pending_resize = true;
            });
            cx.notify();
        }
    }

    /// Consume a page close before a command can reach the hidden tab or pane.
    pub(in crate::workspace) fn return_to_worktree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_page.is_none() {
            return false;
        }
        self.close_page(cx);
        let focused = self.active_runtime().focused_pane_id;
        if self
            .active_runtime()
            .panes
            .iter()
            .any(|pane| pane.id == focused)
        {
            self.focus_pane(focused, window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_page_selection_round_trips_without_changing_utility_views() {
        for page in [Page::Tasks, Page::Flows] {
            assert_eq!(Page::from_legacy(page.persisted_view()), Some(page));
        }
        for view in [
            RightDockView::Usage,
            RightDockView::Skills,
            RightDockView::Tools,
        ] {
            assert_eq!(Page::from_legacy(view), None);
        }
    }
}
