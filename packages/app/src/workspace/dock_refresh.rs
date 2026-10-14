//! Dock view switching and targeted dock invalidation: left / right dock
//! view setters, cached-dock notifies, and the agent-chat status pulse.

use gpui::Context;

use super::{Workspace, main_area};

impl Workspace {
    pub(in crate::workspace) fn set_left_dock_view(
        &mut self,
        view: daruda_store::project::LeftDockView,
        cx: &mut Context<Self>,
    ) {
        self.close_page(cx);
        if self.docks.left_view == view {
            return;
        }
        self.mutate_durable(cx, |ws, _| {
            ws.docks.left_view = view;
        });
        if view == daruda_store::project::LeftDockView::GitChanges {
            let target = self.active;
            self.refresh_git_status(target, cx);
        }
        cx.notify();
    }

    /// Invalidate the right dock's `.cached()` element so it re-renders
    /// from the freshly staged snapshot. The right dock renders only from
    /// `Workspace::render`'s staged snapshot and never self-notifies for
    /// Workspace-owned state (usage, claude status, the MCP/skills/tasks
    /// globals, tab/filter setters, the status + task-live pulses), so
    /// every mutation of a right-dock source must call this — otherwise
    /// the cache shows stale data (root AGENTS.md Pitfall #10). Embedded
    /// input entities (the search boxes, the usage dropdown) self-notify
    /// and dirty this dock as an ancestor, so they need no explicit call.
    /// Lease-free (`App::notify`) — dock event listeners run inside a
    /// `Context<Dock>` lease, so leasing the dock here would double-lease.
    pub(crate) fn notify_right_dock(&self, cx: &mut Context<Self>) {
        let dock_id = self.docks.right.entity_id();
        gpui::App::notify(cx, dock_id);
    }

    /// Invalidate the left dock's `.cached()` element directly. The sole
    /// caller is the status pulse, which advances badge animation frames
    /// that are *not* part of `LeftDockSnapshot` (so the staging diff reports
    /// no change and would not refresh the dock).
    /// Lease-free (`App::notify`) — dock event listeners run inside a
    /// `Context<Dock>` lease, so leasing the dock here would double-lease.
    /// Whether a status-pulse tick can change anything the left dock paints.
    ///
    /// The animated badge exists only in the Lanes view's rows
    /// (`left_dock/projects/agent_badges.rs`). Pulsing the dock while it shows
    /// Git Changes or Files repaints a view holding no badge — and Git Changes
    /// builds one row element per changed file, so on a large branch that is a
    /// full rebuild of the list four times a second to animate nothing.
    pub(crate) fn left_dock_paints_status_pulse(&self, cx: &gpui::App) -> bool {
        self.docks.left.read(cx).is_open
            && self.docks.left_view == daruda_store::project::LeftDockView::Lanes
    }

    pub(crate) fn notify_left_dock(&self, cx: &mut Context<Self>) {
        let dock_id = self.docks.left.entity_id();
        gpui::App::notify(cx, dock_id);
    }

    /// Refresh both dock badges after an agent session-status change.
    /// One notify re-stages both snapshots; each dock's staging diff
    /// repaints it only on a real change. The app's Dock badge is recounted
    /// with them, and keep-awake re-decides.
    pub(crate) fn notify_status_docks(&self, cx: &mut Context<Self>) {
        cx.notify();
        Self::refresh_dock_badge(cx);
        crate::keep_awake::refresh(cx);
    }

    /// Drive the AgentChat status pulse and paint one final settled frame
    /// for a pane that just left the Working state. Called every
    /// status-pulse tick.
    ///
    /// Scans every lane, not just the active one — a subagent can keep
    /// ticking in a parked lane, and its pane still needs a trailing
    /// settle frame when the run quiesces.
    ///
    /// A busy [`AgentChatView`] is dirtied each tick to animate its pulse
    /// (root AGENTS.md Pitfall #10). A view that just went idle gets one
    /// more dirty here: its own settling self-notify can miss a cached
    /// view that fell out of the window's tracked set (gpui
    /// `detect_accessed_entities` lost-wakeup, see
    /// `lane_switch_scroll_dead_rootcause`), which would otherwise freeze
    /// its last "running" frame.
    pub(crate) fn pulse_agent_chats(&mut self, cx: &mut Context<Self>) {
        let tick_now = std::time::Instant::now();
        // Collect the candidate panes (clone the entity + its pane id) first so
        // the `self` borrow is dropped before any `view.update` / `self.fire_*`
        // below. A candidate is any pane that was busy at the last tick (so its
        // settle edge is still owed a fire) or could be busy now (`maybe_active`
        // — the cheap O(1) pre-check); idle-and-quiet panes are skipped so the
        // per-tick cost stays bounded.
        type PulseCandidate = (
            main_area::pane_tree::PaneId,
            gpui::Entity<daruda_chat::view::AgentChatView>,
        );
        let candidates: Vec<PulseCandidate> = self
            .every_agent_chat()
            .map(|(id, view)| (id, view.clone()))
            .filter(|(_, v)| {
                let vr = v.read(cx);
                vr.last_reconciled_busy() || vr.maybe_active()
            })
            .collect();

        // Reconcile each candidate: an edge fires completion; the post-reconcile
        // busy level drives the repaint set. The pulse tick is the time-driven
        // settle driver — it catches a background subagent's quiescence that no
        // ACP event announces.
        let mut busy_ids: Vec<gpui::EntityId> = Vec::new();
        let mut completions = Vec::new();
        for (pane_id, view) in &candidates {
            let edge = view.update(cx, |v, cx| v.tick_activity(tick_now, cx));
            // `tick_activity` just recomputed the busy level with `tick_now`;
            // read that instead of `is_busy()`, a second O(items) scan with a
            // fresh `Instant::now()`, so the whole tick uses one `now`.
            if view.read(cx).last_reconciled_busy() {
                busy_ids.push(view.entity_id());
            }
            if let Some(outcome) = edge {
                completions.push((*pane_id, outcome));
            }
        }
        for (pane_id, outcome) in completions {
            self.fire_activity_completion(pane_id, outcome, cx);
        }
        // Nothing animating and nothing just settled — fully at rest, no work.
        if busy_ids.is_empty() && self.agent_pulse_prev.is_empty() {
            return;
        }
        // Re-render the workspace so the snapshot re-reads (re-tracks) the
        // cached AgentChat views before we dirty them by id — mirrors the
        // animating path's workspace notify, and is what lets the trailing
        // settled frame reach a view that had fallen out of the tracked set.
        cx.notify();
        for id in busy_ids.iter().chain(self.agent_pulse_prev.iter()) {
            gpui::App::notify(cx, *id);
        }
        self.agent_pulse_prev = busy_ids;
    }

    pub(in crate::workspace) fn set_right_dock_view(
        &mut self,
        view: daruda_store::project::RightDockView,
        cx: &mut Context<Self>,
    ) {
        if self.docks.right_view == view {
            return;
        }
        self.mutate_durable(cx, |ws, _| {
            ws.docks.right_view = view;
        });
        cx.notify();
    }
}
