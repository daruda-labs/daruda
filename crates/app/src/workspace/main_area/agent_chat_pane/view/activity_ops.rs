//! Activity/status queries and edge-detection for one Agent chat pane: is it
//! busy, since when, and what completion is owed once it settles. Stay `impl
//! AgentChatView` (not [`ActivityTracker`](super::ActivityTracker)) since they
//! also need `queue.turn` and `items`.

use daruda_acp::{ChatItem, subagent_activity};

use super::{
    ActivityState, AgentChatView, AgentSessionStatus, SUBAGENT_QUIESCENCE, TurnOutcome,
    post_turn_delta,
};

impl AgentChatView {
    /// Map to a [`daruda_agent::SessionStatus`] for the lane indicator.
    /// `None` for states that shouldn't contribute one (dormant `Idle`, dead
    /// `Error`).
    pub(in crate::workspace) fn to_session_status(&self) -> Option<daruda_agent::SessionStatus> {
        use daruda_agent::SessionStatus;
        match &self.status {
            AgentSessionStatus::Idle | AgentSessionStatus::Error(_) => None,
            // Runtime prep and the handshake are both connecting sub-phases —
            // same pulsing badge.
            AgentSessionStatus::PreparingRuntime(_)
            | AgentSessionStatus::Connecting
            | AgentSessionStatus::Handshaking(_) => Some(SessionStatus::Connecting),
            AgentSessionStatus::Connected => Some(match self.activity_state() {
                ActivityState::AwaitingPermission => SessionStatus::NeedsAttention,
                ActivityState::Working => SessionStatus::Working,
                ActivityState::Idle => SessionStatus::Idle,
            }),
        }
    }

    /// Cheap O(1) pre-check: could this pane possibly be busy (turn in flight,
    /// or a subagent seen this session)? When false, `is_busy()` is guaranteed
    /// false without scanning `items` — the gate the pulse uses to skip
    /// idle/terminated conversations. Animation-liveness only, distinct from
    /// [`Self::activity_state`]: a pending permission changes the badge label
    /// but must not stop a still-live subagent badge from animating.
    pub(in crate::workspace) fn maybe_active(&self) -> bool {
        self.queue.turn.is_in_flight()
            || !self.activity.subagent_last_activity.is_empty()
            || self.activity.post_turn_dirty_at.is_some()
    }

    /// On the pulse tick: if a post-turn follow-up has quiesced (no in-flight turn
    /// and `POST_TURN_QUIESCENCE` elapsed since the last post-turn update), return
    /// the new assistant text to relay and advance the marker. `None` otherwise.
    pub(in crate::workspace) fn reconcile_post_turn(
        &mut self,
        now: std::time::Instant,
        quiescence: std::time::Duration,
    ) -> Option<String> {
        if self.queue.turn.is_in_flight() {
            return None;
        }
        let dirty_at = self.activity.post_turn_dirty_at?;
        if now.saturating_duration_since(dirty_at) < quiescence {
            return None;
        }
        self.activity.post_turn_dirty_at = None;
        let (delta, new_count) =
            post_turn_delta(&self.items, self.activity.post_turn_relayed_assistant_texts)?;
        self.activity.post_turn_relayed_assistant_texts = new_count;
        Some(delta)
    }

    /// Force-flush a not-yet-quiesced post-turn follow-up (called when a new prompt
    /// is about to subsume it). `None` when nothing is pending.
    pub(in crate::workspace) fn take_pending_post_turn(&mut self) -> Option<String> {
        self.activity.post_turn_dirty_at.take()?;
        let (delta, new_count) =
            post_turn_delta(&self.items, self.activity.post_turn_relayed_assistant_texts)?;
        self.activity.post_turn_relayed_assistant_texts = new_count;
        Some(delta)
    }

    /// Sync the post-turn baseline to the current `AssistantText` count and
    /// clear the dirty clock. Called wherever `items` is bulk-set to a known
    /// baseline, so only messages arriving *after* count as a follow-up.
    pub(super) fn snap_post_turn_baseline(&mut self) {
        self.activity.post_turn_relayed_assistant_texts = self
            .items
            .iter()
            .filter(|it| matches!(it, ChatItem::AssistantText { .. }))
            .count();
        self.activity.post_turn_dirty_at = None;
    }

    pub(in crate::workspace) fn is_busy(&self) -> bool {
        self.queue.turn.is_in_flight()
            || subagent_activity(
                &self.items,
                &self.activity.subagent_last_activity,
                std::time::Instant::now(),
                SUBAGENT_QUIESCENCE,
            )
            .any_running
    }

    /// Advance the activity span for `now`, returning the pending completion
    /// outcome exactly on the busy→idle edge (else `None`). Drives the
    /// working-indicator elapsed anchor; called from the send path, the
    /// event-pump tail, the pulse tick, and the user-cancel path.
    pub(in crate::workspace) fn reconcile_activity(
        &mut self,
        now: std::time::Instant,
    ) -> Option<TurnOutcome> {
        let busy = self.queue.turn.is_in_flight()
            || subagent_activity(
                &self.items,
                &self.activity.subagent_last_activity,
                now,
                SUBAGENT_QUIESCENCE,
            )
            .any_running;
        let edge = match (self.activity.was_busy, busy) {
            (false, true) => {
                self.activity.activity_started_at = Some(now);
                None
            }
            (true, false) => {
                self.activity.activity_started_at = None;
                // The run is over: the `subagent_last_activity` map is only
                // meaningful during an active run (the `subagent N/M` indicator
                // is hidden when idle, and a later subagent event re-populates
                // it), so clear it. This bounds the map and makes `maybe_active`
                // return false once the pane is truly idle, so `pulse_agent_chats`
                // stops re-scanning a finished-subagent pane every tick. Safe for
                // `is_busy`: this arm is only reached when `busy` is false — no
                // child is live — so clearing the timestamps cannot change
                // `any_running`.
                self.activity.subagent_last_activity.clear();
                self.activity.pending_completion.take()
            }
            _ => None,
        };
        self.activity.was_busy = busy;
        edge
    }

    /// Elapsed time since the current activity span began (busy→…), or `None` when
    /// idle. Anchors the working-indicator timer to the whole activity span
    /// (turn + trailing subagents), replacing the turn-scoped `turn.started_at()`.
    pub(in crate::workspace) fn activity_elapsed(&self) -> Option<std::time::Duration> {
        self.activity.activity_started_at.map(|t| t.elapsed())
    }

    /// Count of subagents running *right now* (`total - settled`), for the
    /// working-indicator label; `None` when none are running. Live count, not
    /// a cumulative tally — the chip disappears once the last one settles.
    pub(in crate::workspace) fn subagent_progress(&self) -> Option<usize> {
        let a = subagent_activity(
            &self.items,
            &self.activity.subagent_last_activity,
            std::time::Instant::now(),
            SUBAGENT_QUIESCENCE,
        );
        let running = a.total - a.settled;
        (running > 0).then_some(running)
    }

    /// Whether any permission card is still awaiting a host decision. O(1) —
    /// reads the outstanding-id index, so the render hot path (activity badge,
    /// working indicator) never rescans `items`.
    pub(in crate::workspace) fn has_pending_permission(&self) -> bool {
        !self.pending_permissions.is_empty()
    }

    /// Whether the permission request `id` is still outstanding (its card
    /// unresolved). Used by the Telegram relay to drop a phone decision for a
    /// request the user already answered in-app (or that was cancelled).
    pub(in crate::workspace) fn is_permission_outstanding(&self, id: u64) -> bool {
        self.pending_permissions.contains(&id)
    }

    /// The pane's derived activity — the single source of the badge label. A
    /// pending permission takes precedence (it needs the user, not the agent);
    /// otherwise the agent is [`ActivityState::Working`] while [`Self::is_busy`].
    pub(in crate::workspace) fn activity_state(&self) -> ActivityState {
        if self.has_pending_permission() {
            return ActivityState::AwaitingPermission;
        }
        if self.is_busy() {
            ActivityState::Working
        } else {
            ActivityState::Idle
        }
    }
}
