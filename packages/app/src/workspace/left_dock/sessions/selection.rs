//! Live session identity, display names, and entry into the selected pane.

use super::{RowTarget, SessionRow};
use crate::workspace::Workspace;
use daruda_agent::SessionStatus;
use daruda_store::project::LaneRef;
use gpui::{Context, Window};

impl Workspace {
    pub(super) fn on_pane_badge_key(
        &mut self,
        target: &RowTarget,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.keystroke.modifiers.modified()
            && matches!(event.keystroke.key.as_str(), "enter" | "space")
        {
            self.activate_dock_row(target, window, cx);
            cx.stop_propagation();
        }
    }

    pub(in crate::workspace) fn set_lane_panes_grouped(
        &mut self,
        lane: LaneRef,
        grouped: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.docks.left.read(cx).is_open {
            self.set_lane_rail_grouped(lane, grouped, cx);
            return;
        }
        if self.lane_for(lane).is_none() {
            return;
        }
        let state = self.lane_scoped_mut(lane);
        if state.show_individual_panes == grouped {
            state.show_individual_panes = !grouped;
            cx.notify();
        }
    }

    pub(super) fn activate_dock_row(
        &mut self,
        target: &RowTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            RowTarget::Session(lane, row) => self.activate_dock_session(*lane, row, window, cx),
            RowTarget::Pane(lane, row) => self.activate_rail_pane(*lane, row.pane, window, cx),
        }
    }

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
        self.activate_rail_pane(lane, target.pane, window, cx);
    }
}

#[cfg(test)]
mod tests {
    // Selection regressions use real Workspace fixtures in the sibling tests module.
}
