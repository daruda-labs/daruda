//! Window-local flow browsing state and its display snapshot.

pub(super) mod listing;
mod ops;
#[cfg(feature = "screenshot")]
pub(in crate::workspace) mod screenshot;
mod state;

pub(in crate::workspace) use state::{
    FlowBrowserState, FlowGrouping, FlowScope, FlowTab, RunFilter,
};

use crate::ui::InputState;
use crate::workspace::{Workspace, layout::diff_policy::Handle};
use gpui::{AppContext as _, Context, Entity, ScrollHandle, Window};

pub(in crate::workspace) struct FlowBrowser {
    pub state: FlowBrowserState,
    pub searches: [Entity<InputState>; 2],
    pub scrolls: [ScrollHandle; 2],
    pub page_snapshot: Option<FlowPageSnapshot>,
}

impl FlowBrowser {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            state: FlowBrowserState::default(),
            searches: std::array::from_fn(|_| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(crate::surface::strings::flow::search_placeholder())
                })
            }),
            scrolls: std::array::from_fn(|_| ScrollHandle::new()),
            page_snapshot: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct FlowTarget {
    pub lane: daruda_store::project::LaneRef,
    pub project: String,
    pub label: String,
    pub current: bool,
}

#[derive(PartialEq)]
pub(in crate::workspace) struct FlowBrowserSnapshot {
    pub state: FlowBrowserState,
    pub targets: Vec<FlowTarget>,
    pub search: Handle<Entity<InputState>>,
    pub query: String,
    pub modified: Vec<(std::path::PathBuf, std::time::SystemTime)>,
}

/// Data staged for the central Flows page, independent of the utility dock.
pub(in crate::workspace) struct FlowPageSnapshot {
    pub workspace: gpui::WeakEntity<Workspace>,
    pub flows: Vec<crate::workspace::flow_rows::FlowRunRow>,
    pub flow_lane: daruda_store::project::LaneRef,
    pub flow_history: Option<crate::workspace::flow_history::FlowHistory>,
    pub flow_files: Vec<crate::workspace::flow_paths::FoundFlow>,
    pub flow_browser: FlowBrowserSnapshot,
    pub flows_with_unsaved_edits: Vec<std::path::PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_have_independent_input_and_scroll_slots() {
        assert_ne!(FlowTab::Definitions.index(), FlowTab::Runs.index());
    }
}
