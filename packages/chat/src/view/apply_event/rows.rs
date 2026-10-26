//! Row projection, virtual-list synchronization and its diagnostic trace.

use super::super::super::agent_chat_helpers::{TurnBoundary, activity_bar_title};
use super::super::super::rows::{
    FilterMatchIndex, LiveSubagentUnits, RowKind, project_with_filter_index,
};
use super::super::super::tool_hierarchy::ToolHierarchy;
use super::super::list_sync::ListSync;
use super::super::{ActivityState, AgentChatView, debug_list_trace_enabled};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

impl AgentChatView {
    /// Recompute the projected render rows from `items` + `fold` and sync the
    /// virtualized `list` to them. The single rebuild site; call after any
    /// `items` or `fold` mutation. Diffs old vs new rows by `same_slot`:
    /// structural changes `splice` from the first divergent slot (scroll above
    /// it preserved); same slots & count (a fold flip or streamed tail growth)
    /// `remeasure_items` over just the changed span with an `Absolute` anchor
    /// so reading history during streaming never drifts.
    pub(in crate::view) fn rebuild_rows(&mut self) {
        self.command_analysis.reconcile(&self.items);
        let old = std::mem::take(&mut self.rows);
        // The inline working indicator means "answering" — suppress it while
        // blocked on a permission prompt (the card + footer already say so).
        // Recorded as the projection's key, not just consumed: see
        // `rows_activity`.
        self.rows_activity = self.activity_state();
        let awaiting_response = matches!(self.rows_activity, ActivityState::Working);
        // Single rebuild site for the subagent-liveness index too: the projection
        // and every tool card's badge read this one, instead of each re-deriving
        // it by scanning `items`.
        // One parent/child pass feeds every hierarchy question below, so the
        // liveness index, the filter index, and the projection all read the
        // same structure instead of each re-deriving it.
        let hierarchy = ToolHierarchy::build(&self.items);
        self.live_units = LiveSubagentUnits::build(&hierarchy, &self.items);
        self.filter_matches =
            FilterMatchIndex::build(&hierarchy, &self.items, self.display_filter.value());
        self.turn_boundary = TurnBoundary::of(&self.items);
        self.activity_title = activity_bar_title(self.session_title.as_deref(), &self.items);
        self.rows = project_with_filter_index(
            &self.items,
            &hierarchy,
            &self.fold,
            awaiting_response,
            &self.live_units,
            self.step_windows(),
            &self.filter_matches,
        );

        if let Some(at) = old
            .iter()
            .zip(&self.rows)
            .position(|(a, b)| !a.same_slot(b))
        {
            self.apply_list_sync(
                ListSync::Structure {
                    from: at,
                    old_len: old.len(),
                },
                "splice-divergent",
            );
            return;
        }
        if old.len() != self.rows.len() {
            let at = old.len().min(self.rows.len());
            self.apply_list_sync(
                ListSync::Structure {
                    from: at,
                    old_len: old.len(),
                },
                "splice-count",
            );
            return;
        }
        // Same slots & count: only `hidden` flipped or item content grew.
        // Remeasure the span whose `hidden` changed; if none did, a streamed
        // tail grow → remeasure the last row.
        let (lo, hi) = old
            .iter()
            .zip(&self.rows)
            .enumerate()
            .filter(|(_, (a, b))| a.hidden != b.hidden)
            .fold((usize::MAX, 0usize), |(lo, hi), (i, _)| {
                (lo.min(i), hi.max(i))
            });
        if lo == usize::MAX {
            // No `hidden` flip: a streamed chunk grew a row's content in place.
            // During an active turn the *last* row is a fixed-height
            // `WorkingIndicator` (`rows::project` pins it to the tail), so the row
            // that actually grew is the last non-indicator row. Remeasuring only
            // `n-1` would re-measure the indicator and leave the grown content row
            // at its stale (shorter) cached height — which then inflates the
            // scroll geometry the moment that row scrolls into the overdraw zone
            // (the intermittent oversized-gap bug). Remeasure from the last
            // content row through the end so both the grown row and the indicator
            // are covered.
            let n = self.rows.len();
            if n > 0 {
                let start = self.rows[..n]
                    .iter()
                    .rposition(|r| !matches!(r.kind, RowKind::WorkingIndicator))
                    .unwrap_or(n - 1);
                self.apply_list_sync(ListSync::Rows(start..n), "tail-grow");
            }
        } else {
            self.apply_list_sync(ListSync::Rows(lo..hi + 1), "hidden-span");
        }
    }

    /// Trace one list-sync decision (splice or remeasure) to the NDJSON log,
    /// silent unless `DARUDA_DEBUG_AGENT_LIST` is set. Kept compiled in
    /// (near-zero cost off) to capture the sync timeline on recurrence of the
    /// intermittent oversized-gap bug (a row's height changes without a
    /// matching remeasure). `prev_rows` is the count *before* this sync, so a
    /// splice's count delta is visible in the trace.
    pub(in crate::view) fn trace_list_sync(
        &self,
        branch: &str,
        from: usize,
        to: usize,
        prev_rows: usize,
    ) {
        if !debug_list_trace_enabled() {
            return;
        }
        daruda_store::observability::log_writer::LogWriter::log(
            ErrorReport::new("agent-chat list sync")
                .severity(ErrorSeverity::Info)
                .with_context("pane", self.pane_id.to_string())
                .with_context("branch", branch.to_string())
                .with_context("from", from.to_string())
                .with_context("to", to.to_string())
                .with_context("rows", self.rows.len().to_string())
                .with_context("prev_rows", prev_rows.to_string())
                .at(file!(), line!())
                .dedup("agent_chat.list_sync_trace")
                .build(),
        );
    }
}

#[cfg(test)]
mod tests;
