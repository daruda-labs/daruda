//! Streaming content updates, scoped resource invalidation and live tool clocks.

use super::super::AgentChatView;
use super::{EventChanges, PhoneTurnAction, ReconcileScope};
use daruda_acp::{ChatItem, apply_update_with, touched_tool_id};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

impl AgentChatView {
    pub(super) fn apply_compaction(
        &mut self,
        session_id: &str,
        update: &daruda_acp::SessionUpdate,
        changes: &mut EventChanges,
    ) {
        changes.compaction_index =
            daruda_acp::apply_compaction_update(&mut self.items, session_id, update);
        changes.touched_text = true;
        if let Some(ChatItem::Compaction(c)) =
            changes.compaction_index.and_then(|ix| self.items.get(ix))
        {
            changes.reconcile_scope = ReconcileScope::Tool(c.output_key());
        }
    }

    pub(super) fn apply_content_update(
        &mut self,
        update: &daruda_acp::SessionUpdate,
        changes: &mut EventChanges,
    ) {
        // Fold protocol traffic through this pane's per-agent strategy
        // (selected from its catalog id) so vendor-specific `_meta` is
        // read the way that agent emits it. See `daruda_acp::adapter`.
        let adapter =
            daruda_acp::adapter::adapter_for(self.agent_program.as_deref(), &self.agent_id);
        let effect = apply_update_with(&mut self.items, update, adapter.as_ref());
        changes.touched_tool = effect.touched_tool;
        changes.touched_text = effect.touched_text;
        // A settled tool call whose only content was an embedded
        // terminal handle we couldn't resolve from any channel — the
        // card is left empty. `daruda_acp` has no logger of its own, so
        // the drop is reported back through the effect and logged here.
        // Logged once per pane per session: this indicates a systemic
        // adapter contract mismatch (every command would trip it), so
        // one line is enough to diagnose. The `dedup` key is a grep tag
        // only — `LogWriter::log` writes unconditionally; it does not
        // itself suppress repeats.
        if effect.dropped_terminal_output && !self.warned_dropped_terminal_output {
            self.warned_dropped_terminal_output = true;
            let report = ErrorReport::new("Tool output dropped")
                .severity(ErrorSeverity::Warning)
                .message(
                    "A shell tool reported its result as an embedded terminal with no \
                     output on any side channel, so the card is empty.",
                )
                .with_context("agent", self.agent_id.clone())
                .at(file!(), line!())
                .dedup(format!(
                    "agent_chat.dropped_terminal_output.{}",
                    self.agent_id
                ))
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        if let Some(tool_id) = touched_tool_id(update) {
            // Narrow the reconciles to the one call this update replaced.
            // Without a named call (an update shape that touches tools
            // but reports no id) the scope stays `All` — always correct,
            // just more expensive.
            changes.reconcile_scope = ReconcileScope::Tool(tool_id.to_string());
            // Both wall-clock stamps below are skipped while replaying:
            // a replayed event did not happen now. Stamping them would
            // read the restored conversation as busy — and the pump's
            // `tick_activity` would then reproject mid-load, which is
            // exactly what the replay gate coalesces away.
            if !self.replay.is_loading() {
                // First sighting starts the call's clock; later progress
                // updates for the same id must not restart it.
                self.activity
                    .tool_started_at
                    .entry(tool_id.to_string())
                    .or_insert_with(std::time::Instant::now);
                // Bump the subagent (parent) whose child just produced
                // this event, so its run span stays "active" across the
                // gaps between its sequential child calls. Only child
                // tools carry a `parent_tool_id`; a top-level tool has
                // none, so nothing is bumped for the turn's own work.
                let parent = self.items.iter().rev().find_map(|it| match it {
                    ChatItem::ToolCall(tc) if tc.id == tool_id => tc.parent_tool_id.clone(),
                    _ => None,
                });
                if let Some(parent) = parent {
                    self.activity
                        .subagent_last_activity
                        .insert(parent, std::time::Instant::now());
                }
            }
        }
        changes.phone_turn_action = PhoneTurnAction::CheckUpdate;
    }
}

#[cfg(test)]
mod tests;
