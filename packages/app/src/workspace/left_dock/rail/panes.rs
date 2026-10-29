use std::collections::{HashMap, HashSet};

use daruda_agent::{AgentOutcome, SessionStatus};
use daruda_store::project::LaneRef;
use gpui::{App, Context, SharedString, Window};

use crate::ui::icons;
use crate::workspace::{
    Workspace,
    main_area::{pane::PaneContent, tab_ops::PaneEntry},
};

#[derive(Clone, PartialEq)]
pub(in crate::workspace) struct PaneSnapshot {
    pub pane: u64,
    pub title: SharedString,
    pub neutral_icon: &'static str,
    pub status: Option<SessionStatus>,
    pub outcome: Option<AgentOutcome>,
}

impl PaneSnapshot {
    pub(in crate::workspace::left_dock) fn unconnected_icon(&self) -> Option<&'static str> {
        (self.status.is_none() && self.outcome.is_none()).then_some(self.neutral_icon)
    }
}

impl Workspace {
    pub(in crate::workspace) fn rail_pane_snapshots(
        &self,
        acp: &[(u64, SessionStatus)],
        cx: &App,
    ) -> HashMap<LaneRef, Vec<PaneSnapshot>> {
        let compact = !self.docks.left.read(cx).is_open;
        self.main_area
            .runtimes
            .keys()
            .map(|target| {
                let project = self.project_for(target.project);
                let individual = if compact {
                    project
                        .is_some_and(|project| !project.rail_grouped_lanes.contains(&target.lane))
                } else {
                    self.lane_scoped
                        .get(target)
                        .is_some_and(|state| state.show_individual_panes)
                };
                let visible = individual
                    && project.is_some_and(|project| {
                        !project.is_collapsed
                            && !project.group_id.is_some_and(|group| {
                                self.groups
                                    .iter()
                                    .any(|entry| entry.id == group && entry.is_collapsed)
                            })
                    });
                (*target, self.lane_pane_snapshots(*target, acp, visible, cx))
            })
            .collect()
    }

    pub(in crate::workspace) fn rail_chooser_panes(
        &self,
        lane: LaneRef,
        cx: &App,
    ) -> Vec<PaneSnapshot> {
        let acp = self.agent_chat_badge_statuses(cx);
        self.lane_pane_snapshots(lane, &acp, true, cx)
    }

    fn lane_pane_snapshots(
        &self,
        lane: LaneRef,
        acp: &[(u64, SessionStatus)],
        include_titles: bool,
        cx: &App,
    ) -> Vec<PaneSnapshot> {
        let Some(runtime) = self.main_area.runtimes.get(&lane) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        runtime
            .tabs
            .iter()
            .filter(|tab| !self.is_orchestrator_tab(tab))
            .flat_map(|tab| tab.layout.pane_ids())
            .filter(|pane| seen.insert(*pane))
            .filter_map(|id| runtime.panes.iter().find(|pane| pane.id == id))
            .map(|pane| {
                let ids = [pane.id];
                PaneSnapshot {
                    pane: pane.id,
                    neutral_icon: match &pane.content {
                        PaneContent::Terminal(_) => icons::TERMINAL,
                        PaneContent::AgentChat(_) => icons::SESSION,
                        PaneContent::File(_) => icons::RADIO_OFF,
                    },
                    title: if include_titles {
                        pane.title(cx)
                    } else {
                        SharedString::default()
                    },
                    status: crate::workspace::claude_status_aggregate::tab_session_status(
                        &ids,
                        &self.claude_sessions.pty_bindings,
                        &self.claude_sessions.status,
                        acp,
                    ),
                    outcome: self.unseen_outcomes.for_panes(&ids).next(),
                }
            })
            .collect()
    }

    pub(in crate::workspace) fn activate_rail_pane(
        &mut self,
        lane: LaneRef,
        pane: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.main_area.runtimes.get(&lane).is_some_and(|runtime| {
            runtime
                .tabs
                .iter()
                .any(|tab| !self.is_orchestrator_tab(tab) && tab.layout.contains(pane))
                && runtime.panes.iter().any(|entry| entry.id == pane)
        }) {
            self.close_session_chooser(true, window, cx);
            return;
        }
        self.close_session_chooser(false, window, cx);
        self.activate_lane_as(lane, PaneEntry::Preview, window, cx);
        if self.active != lane {
            return;
        }
        let Some(tab) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(pane))
        else {
            return;
        };
        self.activate_tab_as(tab, PaneEntry::Preview, window, cx);
        if self
            .main_area
            .zoomed_pane_id
            .is_some_and(|zoomed| zoomed != pane)
        {
            self.main_area.zoomed_pane_id = None;
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane, window, cx);
        if let Some(tab) = self.active_tab_mut() {
            tab.last_focused_pane = pane;
        }
        self.bump_activity(pane);
        self.focus_pane(pane, window, cx);
        self.mutate_durable(cx, |_, _| {});
    }
}

#[cfg(test)]
mod tests {}
