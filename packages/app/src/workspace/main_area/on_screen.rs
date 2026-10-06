//! Whether a pane is in view: the question the "skip the pane you are looking
//! at" notification rule asks of every channel.

use super::pane_tree::PaneId;
use crate::workspace::Workspace;

impl Workspace {
    /// `true` when `pane_id` is drawn: in the active lane's active tab, not
    /// hidden behind another pane zoomed in that tab, and not covered by
    /// Settings, a page or an unavailable lane's empty state. Every pane of a
    /// split counts, not only the focused one. Mirrors `render::center`.
    pub(in crate::workspace) fn pane_on_screen(&self, pane_id: PaneId) -> bool {
        if self.settings.is_some() || self.workspace_page.is_some() {
            return false;
        }
        if self.active_lane().is_some_and(|lane| {
            lane.availability != crate::lane::availability::LaneAvailability::Present
        }) {
            return false;
        }
        let runtime = self.active_runtime();
        let Some(tab) = runtime.tabs.get(runtime.active_tab_index) else {
            return false;
        };
        if !tab.layout.contains(pane_id) {
            return false;
        }
        // A zoom only hides this tab's other panes when the zoomed pane is in
        // it; one left over from another lane zooms nothing here.
        match self.main_area.zoomed_pane_id {
            Some(zoomed) if tab.layout.contains(zoomed) => zoomed == pane_id,
            _ => true,
        }
    }
}
