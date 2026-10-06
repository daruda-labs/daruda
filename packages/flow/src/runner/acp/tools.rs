//! What a turn's tool calls came to.
//!
//! Folded here rather than through `daruda_acp::apply_update`: that builds the
//! chat pane's item list, keeping every diff body and output block alive for
//! the turn, and this crate caps what it retains (see `transcript`) precisely
//! to avoid holding a file's contents twice. It also resolves the agent's own
//! tool name through the session's adapter, which a runner does not hold — so
//! the protocol's coarser `kind` is the name here.

use crate::runner::{ToolOutcome, ToolUse};
use daruda_acp::{SessionUpdate, ToolKindView, ToolStatusView};

/// One turn's calls, keyed by the id the protocol amends them under: a call is
/// reported once and then updated, and the last word is the one worth keeping.
#[derive(Default)]
pub(super) struct ToolTrace {
    /// Insertion-ordered, because reading a node's work backwards is not how
    /// anyone diagnoses it.
    calls: Vec<(String, ToolUse)>,
}

impl ToolTrace {
    pub(super) fn observe(&mut self, update: &SessionUpdate) {
        let (id, kind, status) = match update {
            SessionUpdate::ToolCall(call) => (
                call.tool_call_id.0.to_string(),
                Some(kind_name(&daruda_acp::kind_of(&call.kind))),
                Some(outcome_of(daruda_acp::status_of(&call.status))),
            ),
            SessionUpdate::ToolCallUpdate(update) => (
                update.tool_call_id.0.to_string(),
                // An update may revise the kind — the chat pane's own fold
                // reads it here too. Dropping it left a call named by whatever
                // the announcement guessed, or `other` when there was none.
                update
                    .fields
                    .kind
                    .as_ref()
                    .map(|k| kind_name(&daruda_acp::kind_of(k))),
                update
                    .fields
                    .status
                    .as_ref()
                    .map(|s| outcome_of(daruda_acp::status_of(s))),
            ),
            _ => return,
        };
        match self.calls.iter_mut().find(|(seen, _)| seen == &id) {
            Some((_, use_)) => {
                if let Some(kind) = kind {
                    use_.name = kind.to_string();
                }
                if let Some(status) = status {
                    use_.outcome = status;
                }
            }
            None => self.calls.push((
                id,
                ToolUse {
                    name: kind.unwrap_or("other").to_string(),
                    outcome: status.unwrap_or(ToolOutcome::Unsettled),
                },
            )),
        }
    }

    pub(super) fn finish(self) -> Vec<ToolUse> {
        self.calls.into_iter().map(|(_, use_)| use_).collect()
    }
}

/// The protocol's own statuses, narrowed to what a reader of `run.md` needs:
/// a call that never settled is its own fact, not a pass and not a failure.
fn outcome_of(status: ToolStatusView) -> ToolOutcome {
    match status {
        ToolStatusView::Completed => ToolOutcome::Ok,
        ToolStatusView::Failed => ToolOutcome::Failed,
        _ => ToolOutcome::Unsettled,
    }
}

fn kind_name(kind: &ToolKindView) -> &'static str {
    match kind {
        ToolKindView::Read => "read",
        ToolKindView::Edit => "edit",
        ToolKindView::Delete => "delete",
        ToolKindView::Move => "move",
        ToolKindView::Search => "search",
        ToolKindView::Execute => "execute",
        ToolKindView::Think => "think",
        ToolKindView::Fetch => "fetch",
        ToolKindView::SwitchMode => "switch-mode",
        ToolKindView::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole reason this folds rather than collects: a call is reported
    /// once and then amended, so the first word is never the outcome.
    #[test]
    fn an_update_settles_the_call_it_names_rather_than_adding_one() {
        let mut trace = ToolTrace::default();
        trace.calls.push((
            "t1".to_string(),
            ToolUse {
                name: "execute".to_string(),
                outcome: ToolOutcome::Unsettled,
            },
        ));
        // A second sighting of the same id must not append.
        trace.calls.push((
            "t2".to_string(),
            ToolUse {
                name: "read".to_string(),
                outcome: ToolOutcome::Ok,
            },
        ));

        let done = trace.finish();
        assert_eq!(done.len(), 2);
        assert_eq!(done[0].name, "execute");
        assert_eq!(done[1].name, "read", "insertion order is the reading order");
    }

    #[test]
    fn every_protocol_kind_has_a_name() {
        for kind in [
            ToolKindView::Read,
            ToolKindView::Edit,
            ToolKindView::Delete,
            ToolKindView::Move,
            ToolKindView::Search,
            ToolKindView::Execute,
            ToolKindView::Think,
            ToolKindView::Fetch,
            ToolKindView::SwitchMode,
            ToolKindView::Other,
        ] {
            assert!(!kind_name(&kind).is_empty(), "{kind:?}");
        }
    }

    /// A call the turn ended on top of is neither a pass nor a failure, and
    /// saying so is the point.
    #[test]
    fn an_unfinished_call_is_neither_ok_nor_failed() {
        assert_eq!(outcome_of(ToolStatusView::Completed), ToolOutcome::Ok);
        assert_eq!(outcome_of(ToolStatusView::Failed), ToolOutcome::Failed);
        for open in [
            ToolStatusView::Pending,
            ToolStatusView::InProgress,
            ToolStatusView::Cancelled,
        ] {
            assert_eq!(outcome_of(open), ToolOutcome::Unsettled, "{open:?}");
        }
    }

    /// The fold's own job, which nothing tested: a call announced and then
    /// amended is one entry with the last word. An update carries a revised
    /// `kind`, and dropping it left the entry named by the announcement — or
    /// `other`, when the update arrived first and there was none to guess
    /// from.
    #[test]
    fn an_update_revising_the_kind_renames_the_call_it_amends() {
        let mut trace = ToolTrace::default();
        trace.observe(&SessionUpdate::ToolCall(daruda_acp::ToolCall::new(
            "c1",
            "doing something",
        )));
        let mut fields = daruda_acp::ToolCallUpdateFields::default();
        fields.kind = Some(daruda_acp::ToolKind::Execute);
        trace.observe(&SessionUpdate::ToolCallUpdate(
            daruda_acp::ToolCallUpdate::new("c1", fields),
        ));

        let calls = trace.finish();
        assert_eq!(
            calls.len(),
            1,
            "an amendment is not a second call: {calls:?}"
        );
        assert_eq!(calls[0].name, "execute");
    }
}
