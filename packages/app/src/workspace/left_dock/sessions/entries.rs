//! Worktree and session choices retain their own typed selection identity.

use super::super::rail::PaneSnapshot;
use crate::workspace::{Workspace, layout::DockSnapshot, main_area::pane_tree::PaneId};
use daruda_agent::SessionStatus;
use daruda_store::project::{LaneId, LaneRef, ProjectId};
use gpui::SharedString;

#[derive(Clone, Copy)]
pub(super) enum Target {
    Panes {
        lane: LaneRef,
        selected: PaneId,
    },
    Worktrees {
        project: ProjectId,
        selected: Option<LaneId>,
    },
}

impl Target {
    pub(super) fn same_list(self, other: Self) -> bool {
        match (self, other) {
            (Self::Panes { lane: a, .. }, Self::Panes { lane: b, .. }) => a == b,
            (Self::Worktrees { project: a, .. }, Self::Worktrees { project: b, .. }) => a == b,
            _ => false,
        }
    }

    pub(super) fn select(&mut self, entry: &Entry) {
        match (self, entry) {
            (Self::Panes { selected, .. }, Entry::Pane { row, .. }) => *selected = row.pane,
            (Self::Worktrees { selected, .. }, Entry::Worktree { lane, .. }) => {
                *selected = Some(lane.lane)
            }
            _ => {}
        }
    }

    pub(super) fn is_selected(self, entry: &Entry) -> bool {
        match (self, entry) {
            (Self::Panes { lane: a, selected }, Entry::Pane { lane: b, row }) => {
                a == *b && selected == row.pane
            }
            (Self::Worktrees { project, selected }, Entry::Worktree { lane, .. }) => {
                project == lane.project && selected == Some(lane.lane)
            }
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(super) enum Entry {
    Pane {
        lane: LaneRef,
        row: PaneSnapshot,
    },
    Worktree {
        lane: LaneRef,
        title: SharedString,
        status: Option<SessionStatus>,
        outcome: Option<daruda_agent::AgentOutcome>,
        open_tabs: usize,
    },
}

impl Workspace {
    pub(super) fn rail_chooser_entries(&self, target: Target, cx: &gpui::App) -> Vec<Entry> {
        match target {
            Target::Panes { lane, .. } => self
                .rail_chooser_panes(lane, cx)
                .into_iter()
                .map(|row| Entry::Pane { lane, row })
                .collect(),
            Target::Worktrees { project, .. } => {
                let dock = self.docks.left.read(cx);
                let DockSnapshot::Left(snap) = &dock.snap else {
                    return Vec::new();
                };
                let Some(project) = snap.projects.iter().find(|p| p.id == project) else {
                    return Vec::new();
                };
                let mut lanes: Vec<_> = project.lanes.iter().collect();
                lanes.sort_by_key(|lane| lane.tab_order);
                lanes
                    .into_iter()
                    .map(|lane| {
                        let lane_ref = LaneRef {
                            project: project.id,
                            lane: lane.id,
                        };
                        Entry::Worktree {
                            lane: lane_ref,
                            title: lane.display_name().into(),
                            status: snap.agent_status_per_lane.get(&lane_ref).copied(),
                            outcome: snap.agent_outcome_per_lane.get(&lane_ref).copied(),
                            open_tabs: snap.open_tabs.get(&lane_ref).copied().unwrap_or_default(),
                        }
                    })
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Native selection and stale-row behavior use the sibling Workspace tests.
}
