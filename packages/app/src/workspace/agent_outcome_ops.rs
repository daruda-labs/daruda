//! What a finished agent turn leaves behind: the lane's unread mark and, for
//! a turn that ended out of view, the tab's unseen outcome.
//!
//! One entry for both kinds of session — the chat pane's settle edge and a
//! terminal Claude's `Stop` hook — and one place that drops an unseen
//! outcome once its pane is in front of the user.

use daruda_agent::AgentOutcome;
use gpui::{Context, Window};

use super::Workspace;
use super::main_area::pane_tree::PaneId;

impl Workspace {
    /// Record a turn on `pane_id` that ran to its end. The lane mark follows
    /// its own rule (any lane but the active one); the tab keeps the outcome
    /// only while the pane is not seen.
    pub(in crate::workspace) fn record_agent_outcome(
        &mut self,
        pane_id: PaneId,
        outcome: AgentOutcome,
        cx: &mut Context<Self>,
    ) {
        self.mark_lane_unread_for_pane(pane_id, cx);
        if self.pane_seen(pane_id) {
            return;
        }
        self.unseen_outcomes.record(pane_id, outcome);
        cx.notify();
    }

    /// Install the two triggers that can bring a marked pane into view: any
    /// change to this workspace and the window coming forward. Relies on
    /// every change to `pane_on_screen`'s inputs (tab, lane, page, Settings,
    /// zoom, availability) notifying the Workspace, since it repaints them;
    /// one that does not leaves the mark until the next notify.
    pub(in crate::workspace) fn observe_outcome_visibility(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.observe_self(|ws, cx| ws.acknowledge_seen_outcomes(cx))
            .detach();
        cx.observe_window_activation(window, |ws, window, cx| {
            ws.set_window_active(window.is_window_active(), cx);
        })
        .detach();
    }

    /// The single writer of `window_active`.
    pub(in crate::workspace) fn set_window_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.window_active = active;
        self.acknowledge_seen_outcomes(cx);
    }

    /// Drop the outcomes whose panes are now seen. Notifies only when one
    /// went, so the `observe_self` call it triggers finds nothing and stops.
    fn acknowledge_seen_outcomes(&mut self, cx: &mut Context<Self>) {
        let seen: Vec<PaneId> = self
            .unseen_outcomes
            .panes()
            .filter(|pane| self.pane_seen(*pane))
            .collect();
        if self.unseen_outcomes.forget(&seen) {
            cx.notify();
        }
    }

    /// In front of the user: on screen in a window that is in the foreground.
    /// A turn that ends in the active tab of a background window is still
    /// news when the user comes back.
    fn pane_seen(&self, pane_id: PaneId) -> bool {
        self.window_active && self.pane_on_screen(pane_id)
    }

    /// Mark the lane holding `pane_id` unread unless it is the one on screen.
    /// [`Self::activate_lane`] clears the mark.
    fn mark_lane_unread_for_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(lane_ref) = self.lane_ref_for_pane(pane_id) else {
            return;
        };
        if lane_ref == self.active {
            return;
        }
        let Some(lane) = self.lane_for_mut(lane_ref) else {
            return;
        };
        if lane.is_unread {
            return;
        }
        lane.is_unread = true;
        self.mutate_durable(cx, |_, _| {});
        self.notify_status_docks(cx);
    }
}

#[cfg(feature = "screenshot")]
impl Workspace {
    /// One tab per status dot behind the first tab, each named for what it
    /// shows. Live states are agent-chat panes parked in that state with no
    /// session behind them — a terminal's Claude binding belongs to the PTY
    /// tracker, which clears any it did not find itself. Unseen outcomes go
    /// through the store the tab strip reads.
    pub(in crate::workspace) fn seed_tab_indicators_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::main_area::agent_chat_pane::shot_transcript::working_transcript;

        let first_tab = self.active_runtime().active_tab_index;
        let label_last_tab = |ws: &mut Self, label: &'static str| {
            if let Some(tab) = ws.active_runtime_mut().tabs.last_mut() {
                tab.user_label = Some(label.into());
            }
        };

        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| v.seed_working_transcript(working_transcript(), window, cx),
            window,
            cx,
        );
        label_last_tab(self, "working");
        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_working_transcript(working_transcript(), window, cx);
                v.hold_permission_for_shot(0);
            },
            window,
            cx,
        );
        label_last_tab(self, "attention");
        self.open_agent_chat_pane_seeded(
            None,
            |v, _, cx| v.set_error("failed".into(), daruda_acp::Remedy::NoneAvailable, cx),
            window,
            cx,
        );
        label_last_tab(self, "failed");
        for (label, outcome) in [
            ("errored", AgentOutcome::Errored),
            ("done", AgentOutcome::Completed),
        ] {
            self.add_tab(window, cx);
            let pane = self.active_runtime().focused_pane_id;
            self.unseen_outcomes.record(pane, outcome);
            label_last_tab(self, label);
        }
        self.activate_tab(first_tab, window, cx);
    }
}
