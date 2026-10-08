//! Unsaved work a close has to ask about. A lane's file panes and the pages'
//! details are kept in different places, so each item names what holds it;
//! saving one dispatches on that.

use gpui::{App, SharedString};

use super::main_area::pane::Pane;
use super::main_area::pane_tree::PaneId;
use super::pages::flows::detail::FlowDetailId;
use super::pages::flows::graph::FlowGraphView;
use super::pages::tasks::editor::TaskEditorId;
use super::pages::tasks::editor::state::TaskEditContent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum DirtyTarget {
    /// A file pane in a lane.
    Pane(PaneId),
    /// A Task editor.
    TaskEditor(TaskEditorId),
    /// The Flows page's graph.
    FlowGraph(FlowDetailId),
}

#[derive(Debug, Clone)]
pub(in crate::workspace) struct DirtyItem {
    pub target: DirtyTarget,
    pub title: SharedString,
    /// A Task editor for a task not yet created: its save makes one.
    pub is_draft: bool,
}

impl DirtyItem {
    /// `pane` as unsaved work, or `None` when it has none.
    pub(in crate::workspace) fn of_pane(pane: &Pane, cx: &App) -> Option<Self> {
        if !pane.is_dirty(cx) {
            return None;
        }
        Some(Self {
            target: DirtyTarget::Pane(pane.id),
            title: pane.title(cx),
            is_draft: false,
        })
    }

    /// The Task editor `id` as unsaved work, or `None` when it has none.
    pub(in crate::workspace) fn of_task_editor(
        id: TaskEditorId,
        editor: &TaskEditContent,
        cx: &App,
    ) -> Option<Self> {
        editor.is_dirty(cx).then(|| Self {
            target: DirtyTarget::TaskEditor(id),
            title: editor.title(),
            is_draft: editor.task_id.is_none(),
        })
    }

    /// The Flows page's graph `id` as unsaved work, or `None` when it has none.
    pub(in crate::workspace) fn of_flow_graph(
        id: FlowDetailId,
        view: &gpui::Entity<FlowGraphView>,
        cx: &App,
    ) -> Option<Self> {
        let view = view.read(cx);
        view.has_unsaved_form(cx).then(|| Self {
            target: DirtyTarget::FlowGraph(id),
            title: view.name().to_owned().into(),
            is_draft: false,
        })
    }

    /// The heading of the prompt that asks before this item is left.
    pub(in crate::workspace) fn leave_heading(&self) -> String {
        if self.is_draft {
            crate::surface::strings::task::edit_discard_draft_prompt()
        } else {
            crate::surface::strings::task::edit_save_prompt(&self.title)
        }
    }

    /// The label of that prompt's save button.
    pub(in crate::workspace) fn save_label(&self) -> String {
        if self.is_draft {
            crate::surface::strings::task::edit_save_draft()
        } else {
            crate::surface::strings::common::btn_save()
        }
    }

    /// The prompt's line for this item.
    pub(in crate::workspace) fn line(&self) -> String {
        crate::surface::strings::task::close_dirty_line(&self.title, self.is_draft)
    }
}

/// The prompt detail listing `items`, one per line.
pub(in crate::workspace) fn dirty_listing(items: &[DirtyItem]) -> String {
    items
        .iter()
        .map(DirtyItem::line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_has_one_line_per_item_and_marks_drafts() {
        let items = [
            DirtyItem {
                target: DirtyTarget::Pane(1),
                title: "notes.md".into(),
                is_draft: false,
            },
            DirtyItem {
                target: DirtyTarget::TaskEditor(TaskEditorId(2)),
                title: "New task".into(),
                is_draft: true,
            },
        ];
        let listing = dirty_listing(&items);
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], items[0].line());
        assert_eq!(lines[1], items[1].line());
        assert_ne!(
            items[1].line(),
            crate::surface::strings::task::close_dirty_line("New task", false),
            "a draft reads differently from a saved task",
        );
    }
}
