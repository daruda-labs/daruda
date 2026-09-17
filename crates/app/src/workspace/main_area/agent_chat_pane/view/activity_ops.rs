//! Activity/status queries and edge-detection for one Agent chat pane: is it
//! busy, since when, and what completion is owed once it settles. Stay `impl
//! AgentChatView` (not [`ActivityTracker`](super::ActivityTracker)) since they
//! also need `queue.turn` and `items`.

use daruda_acp::{ChatItem, subagent_activity};

use gpui::Context;

use super::Turn;
use super::{
    ActivitySpan, ActivityState, AgentChatView, AgentSessionStatus, SUBAGENT_QUIESCENCE,
    TurnOutcome,
};

impl AgentChatView {
    /// Map to a [`daruda_agent::SessionStatus`] for the lane indicator.
    /// `None` for states that shouldn't contribute one (dormant `Idle`, dead
    /// `Error`).
    pub(in crate::workspace) fn to_session_status(&self) -> Option<daruda_agent::SessionStatus> {
        use daruda_agent::SessionStatus;
        match &self.status {
            AgentSessionStatus::Idle | AgentSessionStatus::Error { .. } => None,
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
        self.queue.turn.is_in_flight() || !self.activity.subagent_last_activity.is_empty()
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
    /// outcome exactly on the busy→idle edge (else `None`).
    ///
    /// Module-private: every caller outside `view/` goes through
    /// [`Self::tick_activity`], which pairs this with the projection restore it
    /// owes. Reachable here only for the capture seed, which reprojects itself.
    pub(super) fn reconcile_activity(&mut self, now: std::time::Instant) -> Option<TurnOutcome> {
        let busy = self.queue.turn.is_in_flight()
            || subagent_activity(
                &self.items,
                &self.activity.subagent_last_activity,
                now,
                SUBAGENT_QUIESCENCE,
            )
            .any_running;
        match (self.activity.span, busy) {
            (ActivitySpan::Idle, true) => {
                self.activity.span = ActivitySpan::Busy { started_at: now };
                None
            }
            (ActivitySpan::Busy { .. }, false) => {
                self.activity.span = ActivitySpan::Idle;
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
                // Bound the start-time map by what is still live, not by this
                // edge: the edge is a subagent's window lapsing, and a
                // *top-level* call is invisible to `is_busy`, so one can still
                // be running here. Dropping its clock would blank the counter
                // its card is showing and restart it at `0s`.
                let live: std::collections::HashSet<&str> = self
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        ChatItem::ToolCall(tc) if tc.status.is_live() => Some(tc.id.as_str()),
                        _ => None,
                    })
                    .collect();
                self.activity
                    .tool_started_at
                    .retain(|id, _| live.contains(id.as_str()));
                // The adapter can't be the only source of "last active": it
                // sends `updatedAt` only alongside a *changed* session title,
                // so the value would freeze once the title settles.
                self.session_updated_at = Some(crate::surface::timestamp::now_rfc3339());
                self.activity.pending_completion.take()
            }
            // Level unchanged: a running span keeps its original start instant.
            (ActivitySpan::Idle, false) | (ActivitySpan::Busy { .. }, true) => None,
        }
    }

    /// Advance the activity span and restore what is projected from it — one
    /// entry point, so neither half can be done without the other. The working
    /// indicator is projected from a *clock*-dependent level (a trailing
    /// subagent stays busy until its window lapses, with nothing to announce
    /// it), so reconciling alone leaves the row outliving its run while the
    /// footer's Stop button, reading `is_busy()` live, has flipped back to Send.
    pub(in crate::workspace) fn tick_activity(
        &mut self,
        now: std::time::Instant,
        cx: &mut Context<Self>,
    ) -> Option<TurnOutcome> {
        let edge = self.reconcile_activity(now);
        if self.rows_activity != self.settled_activity_state() {
            self.reproject(cx);
        }
        edge
    }

    /// [`Self::activity_state`] read off the span [`Self::reconcile_activity`]
    /// just stored — O(1) where the live form is an O(items) scan with its own
    /// `now`. Only the *reader* may use it: `rebuild_rows` writes
    /// `rows_activity` from the live form because it also runs where no
    /// reconcile preceded it (`respond_permission`, `abort_restore`) and the
    /// span is stale there. That asymmetry is what makes the two converge.
    fn settled_activity_state(&self) -> ActivityState {
        if self.has_pending_permission() {
            return ActivityState::AwaitingPermission;
        }
        if self.activity.span.is_busy() {
            ActivityState::Working
        } else {
            ActivityState::Idle
        }
    }

    /// Elapsed time since the current activity span began (busy→…), or `None` when
    /// idle. Anchors the working-indicator timer to the whole activity span
    /// (turn + trailing subagents), replacing the turn-scoped `turn.started_at()`.
    pub(in crate::workspace) fn activity_elapsed(&self) -> Option<std::time::Duration> {
        self.activity.span.elapsed()
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

impl AgentChatView {
    /// File what the turn that just ended cost, under the run it belongs to.
    ///
    /// Keyed by the run's first item so the record and the response bar above it
    /// name one thing. A run that put nothing on screen gets no key and no
    /// record — there is no row for it to label.
    pub(super) fn record_turn(&mut self, output_tokens: Option<u64>) {
        let Some(run_start) = Self::run_start_of(&self.items) else {
            return;
        };
        let worked_for = match self.queue.turn {
            Turn::InFlight { started_at, .. } => started_at.elapsed(),
            Turn::Idle => return,
        };
        self.activity.turn_records.insert(
            run_start,
            super::TurnRecord {
                worked_for,
                finished_at: chrono::Local::now(),
                output_tokens,
            },
        );
    }

    /// Where the transcript's last agent run begins — the item after the last
    /// user prompt. `None` when nothing follows it, which is the empty-reply
    /// case that projects no response bar either.
    ///
    /// Pure and free-standing so the keying rule is assertable without a window.
    pub(in crate::workspace) fn run_start_of(items: &[ChatItem]) -> Option<usize> {
        let after_prompt = items
            .iter()
            .rposition(|item| matches!(item, ChatItem::UserText(_)))
            .map_or(0, |ix| ix + 1);
        (after_prompt < items.len()).then_some(after_prompt)
    }
}

#[cfg(test)]
mod turn_record_tests {
    use super::*;

    fn asst(text: &str) -> ChatItem {
        ChatItem::AssistantText {
            text: text.to_owned(),
            streaming: false,
            message_id: None,
            phase: Default::default(),
        }
    }

    /// The record is keyed by the run's first item — the same index the run's
    /// fold uses — so a later turn files under its own key instead of
    /// overwriting the previous one.
    #[test]
    fn a_runs_key_is_the_item_after_its_prompt() {
        let cases: [(Vec<ChatItem>, Option<usize>); 4] = [
            (vec![], None),
            (vec![ChatItem::UserText("q".into())], None),
            (vec![ChatItem::UserText("q".into()), asst("a")], Some(1)),
            (
                vec![
                    ChatItem::UserText("q1".into()),
                    asst("a1"),
                    ChatItem::UserText("q2".into()),
                    asst("a2"),
                ],
                Some(3),
            ),
        ];
        for (items, expected) in cases {
            assert_eq!(super::super::AgentChatView::run_start_of(&items), expected);
        }
    }
}
