//! Live session identity, display names, and entry into the selected pane.

use super::SessionRow;
use crate::workspace::{Workspace, main_area::tab_ops::PaneEntry};
use daruda_agent::SessionStatus;
use daruda_store::project::LaneRef;
use gpui::{Context, Window};

impl Workspace {
    pub(in crate::workspace) fn named_lane_sessions(
        &self,
        lane: LaneRef,
        sessions: &[(String, SessionStatus)],
        cx: &gpui::App,
    ) -> Vec<SessionRow> {
        let Some(runtime) = self.main_area.runtimes.get(&lane) else {
            return Vec::new();
        };
        let mut rows: Vec<SessionRow> = sessions
            .iter()
            .filter_map(|(id, status)| {
                let matches = |pane: &crate::workspace::main_area::pane::Pane| {
                    self.claude_sessions
                        .pty_bindings
                        .get(&pane.id)
                        .is_some_and(|b| b.session_id == *id)
                        || id == &format!("acp:{}", pane.id)
                };
                let pane = runtime
                    .panes
                    .iter()
                    .find(|p| p.id == runtime.focused_pane_id && matches(p))
                    .or_else(|| {
                        runtime
                            .tabs
                            .iter()
                            .flat_map(|t| t.layout.pane_ids())
                            .find_map(|pid| {
                                runtime.panes.iter().find(|p| p.id == pid && matches(p))
                            })
                    })?;
                Some(SessionRow {
                    id: id.clone(),
                    pane: pane.id,
                    title: pane.title(cx),
                    status: *status,
                })
            })
            .collect();
        let mut counts = std::collections::HashMap::new();
        for row in &rows {
            *counts.entry(row.title.clone()).or_insert(0usize) += 1;
        }
        for row in &mut rows {
            if counts[&row.title] > 1 {
                row.title =
                    crate::surface::strings::dock::session_title(row.pane, &row.title).into();
            }
        }
        rows
    }

    pub(in crate::workspace) fn activate_dock_session(
        &mut self,
        lane: LaneRef,
        row: &SessionRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let acp = self.agent_chat_statuses(cx);
        let pane_ids = self
            .main_area
            .runtimes
            .get(&lane)
            .map(|runtime| {
                runtime
                    .tabs
                    .iter()
                    .flat_map(|t| t.layout.pane_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let sessions = crate::workspace::claude_status_aggregate::distinct_sessions(
            pane_ids,
            &self.claude_sessions.pty_bindings,
            &self.claude_sessions.status,
            &acp,
        );
        // Resolve the binding again: the session may have closed while its row was visible.
        let live = self.named_lane_sessions(lane, &sessions, cx);
        let Some(target) = live.iter().find(|r| r.id == row.id) else {
            self.close_session_chooser(true, window, cx);
            return;
        };
        let pane = target.pane;
        self.close_session_chooser(false, window, cx);
        self.activate_lane_as(lane, PaneEntry::Preview, window, cx);
        if self.active != lane {
            return;
        }
        let Some(tab) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.layout.contains(pane))
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
        // Keyboard focus alone does not move the shared input's draft owner
        // or the tab's return target; keep both aligned with this session.
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
mod tests {
    // Selection regressions use real Workspace fixtures in the sibling tests module.
}
