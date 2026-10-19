//! Keyboard entry uses the same operations as the rail's pointer triggers.

use crate::workspace::Workspace;
use daruda_store::project::{GroupId, LaneRef, ProjectId};
use gpui::{Bounds, Context, KeyDownEvent, Pixels, Window};

pub(super) enum RailItem {
    Group(GroupId),
    Project(ProjectId),
    Lane(LaneRef, Bounds<Pixels>),
    Pane(LaneRef, u64),
}

impl Workspace {
    pub(in crate::workspace::left_dock::rail) fn on_rail_key(
        &mut self,
        item: RailItem,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified()
            || !matches!(event.keystroke.key.as_str(), "enter" | "space")
        {
            return;
        }
        match item {
            RailItem::Group(group) => self.toggle_group_collapse(group, cx),
            RailItem::Project(project) => self.toggle_project_collapse(project, cx),
            RailItem::Lane(lane, bounds) => self.choose_lane_from_rail(lane, bounds, window, cx),
            RailItem::Pane(lane, pane) => self.activate_rail_pane(lane, pane, window, cx),
        }
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod tests {
    // Native project/group keyboard entry is covered by rail::tests.
}
