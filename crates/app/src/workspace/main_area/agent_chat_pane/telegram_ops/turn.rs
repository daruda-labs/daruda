//! The phone's side of one agent turn.
//!
//! Two relays report a turn to the phone — its first response and its
//! completion — and they have to agree about what the sender already has, or a
//! turn whose opening message is also its closing one goes out twice. This is
//! where they agree: one value, owned by `AgentChatView`, that outlives the
//! first response and remembers which message it carried.
//!
//! The state is sealed. [`PhoneTurn`] is a newtype over a private [`State`],
//! so every transition goes through a method here and no caller can put a turn
//! into a shape the relays would misread. GPUI-free: it reads `ChatItem`s and
//! answers questions, and the [`super`] layer renders and sends.

use daruda_acp::ChatItem;

/// What a chat item appended since a [`PhoneTurn`] was armed resolves to, if
/// anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FirstResponseOutcome {
    /// The agent's first visible reply was text.
    ///
    /// `message_id` names the assistant message it came from, when the agent
    /// supplied one. It travels with the text because it identifies *that*
    /// message: the completion relay reports the turn's last message, and
    /// whether the sender already has it is a question about which message,
    /// not about whether two previews happen to render the same.
    Text {
        text: String,
        message_id: Option<String>,
    },
    /// The agent went straight to a tool call with no preceding text.
    /// `tool_title` names it when the agent supplied a non-empty title.
    Tool { tool_title: Option<String> },
}

impl FirstResponseOutcome {
    /// The assistant message this outcome put on the phone, when it named one.
    /// A tool ack carries no agent text, so it names nothing and the
    /// completion still owes the sender an answer.
    fn message_id(&self) -> Option<String> {
        match self {
            Self::Text { message_id, .. } => message_id.clone(),
            Self::Tool { .. } => None,
        }
    }
}

/// Private shape behind [`PhoneTurn`] — see the module docs for why it is not
/// reachable from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Nothing has gone out yet. `items_len_at_start` is where in `items` to
    /// resume scanning (the length when this turn's prompt was echoed, so the
    /// echoed `UserText` is never mistaken for a response); `started_at`
    /// drives the fallback pump's overdue check.
    Waiting {
        started_at: std::time::Instant,
        items_len_at_start: usize,
    },
    /// A report has gone out. `message_id` names the assistant message it
    /// carried, when the agent named one; `None` covers a tool ack, the fixed
    /// fallback ack, and an agent that omits ids — none of which put the
    /// turn's answer on the phone, so the completion still owes one.
    ///
    /// `items_len_at_start` is carried through unchanged: the completion
    /// relay still needs to know where this turn's own output begins.
    Answered {
        message_id: Option<String>,
        items_len_at_start: usize,
    },
}

/// The phone's side of one turn: what the sender is owed, and what they have
/// already been given.
///
/// Owned by `AgentChatView` as `Option<PhoneTurn>`; `None` means the in-flight
/// turn did not come from the phone, so nothing here applies.
///
/// # Known limitation
///
/// [`Self::answer`] records that a report was *composed*, not that Telegram
/// received it. A report can still be dropped downstream by the bridge gate
/// (feature off, nobody paired). That gate is shared with the completion
/// relay, so a closed gate drops both and the sender sees no difference; only
/// disabling Telegram mid-turn and re-enabling it before the turn ends can
/// lose an answer. Recording on delivery instead would split the decision and
/// the transition across two calls, and a caller that took the first without
/// the second would re-ack on every pump tick — a likelier failure than the
/// one it prevents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct PhoneTurn(State);

impl PhoneTurn {
    /// Arm a turn anchored at `now`, scanning `items` from `items_len` onward.
    pub(in crate::workspace) fn start(now: std::time::Instant, items_len: usize) -> Self {
        Self(State::Waiting {
            started_at: now,
            items_len_at_start: items_len,
        })
    }

    /// Whether the sender is still waiting for the first sign of life.
    pub(in crate::workspace) fn is_waiting(&self) -> bool {
        matches!(self.0, State::Waiting { .. })
    }

    /// Whether `timeout_secs` has elapsed with nothing qualifying found yet —
    /// the periodic fallback pump's readiness check. A turn already answered
    /// is never overdue.
    pub(in crate::workspace) fn is_overdue(
        &self,
        now: std::time::Instant,
        timeout_secs: u64,
    ) -> bool {
        match &self.0 {
            State::Waiting { started_at, .. } => {
                now.saturating_duration_since(*started_at).as_secs() >= timeout_secs
            }
            State::Answered { .. } => false,
        }
    }

    /// The first item appended since this turn was armed that resolves it, if
    /// any — `None` means keep waiting, and a turn already answered resolves
    /// nothing further.
    ///
    /// A still-streaming `AssistantText` and any `Thinking` item are skipped:
    /// reasoning isn't the visible reply, and a text reply must be complete
    /// before it's worth sending (see `daruda_acp::ChatItem::AssistantText`'s
    /// `streaming` field). A settled message with no text is skipped too —
    /// `daruda_acp` collapses a content block it cannot render to an empty
    /// string, and an empty notification is worse than a late one.
    ///
    /// A query: it decides nothing. [`Self::answer`] is what records.
    pub(in crate::workspace) fn first_response(
        &self,
        items: &[ChatItem],
    ) -> Option<FirstResponseOutcome> {
        let State::Waiting {
            items_len_at_start, ..
        } = &self.0
        else {
            return None;
        };
        items
            .get(*items_len_at_start..)?
            .iter()
            .find_map(|item| match item {
                ChatItem::AssistantText {
                    text,
                    streaming: false,
                    message_id,
                    ..
                } if !text.trim().is_empty() => Some(FirstResponseOutcome::Text {
                    text: text.clone(),
                    message_id: message_id.clone(),
                }),
                ChatItem::ToolCall(tool) => Some(FirstResponseOutcome::Tool {
                    tool_title: Some(tool.title.clone()).filter(|t| !t.is_empty()),
                }),
                _ => None,
            })
    }

    /// Record that a report went out, naming the assistant message it carried
    /// if any. The one transition into `Answered`, so a turn cannot be marked
    /// answered field-wise from several call paths.
    ///
    /// Returns whether this call is the one that answered a waiting turn, so
    /// the caller emits its ack exactly once. Already-answered turns return
    /// `false` and keep the message they first reported: the completion asks
    /// about *that* message, and letting a later ack overwrite it would hand
    /// the sender a report they already have.
    #[must_use = "the caller emits its ack only when this call is the one that answered"]
    pub(in crate::workspace) fn answer(&mut self, message_id: Option<String>) -> bool {
        let State::Waiting {
            items_len_at_start, ..
        } = self.0
        else {
            return false;
        };
        self.0 = State::Answered {
            message_id,
            items_len_at_start,
        };
        true
    }

    /// Record the first response `outcome` as sent. Sugar over [`Self::answer`]
    /// so the id is never pulled out of the outcome at a call site.
    #[must_use = "the caller emits its ack only when this call is the one that answered"]
    pub(in crate::workspace) fn answer_with(&mut self, outcome: &FirstResponseOutcome) -> bool {
        self.answer(outcome.message_id())
    }

    /// Where this turn's own output begins in `items`.
    ///
    /// The completion relay bounds its scan to it, so a turn that produced no
    /// text of its own reports exactly that instead of handing the sender an
    /// earlier turn's answer as if it were this one's.
    pub(in crate::workspace) fn items_anchor(&self) -> usize {
        match &self.0 {
            State::Waiting {
                items_len_at_start, ..
            }
            | State::Answered {
                items_len_at_start, ..
            } => *items_len_at_start,
        }
    }

    /// Whether the phone already has `message_id` — the completion relay's
    /// question, asked of the message it is about to report.
    ///
    /// An unnamed message never matches: without identity the honest answer is
    /// "cannot tell", and repeating an answer is the lesser failure against
    /// swallowing one.
    pub(in crate::workspace) fn already_sent(&self, message_id: Option<&str>) -> bool {
        match (&self.0, message_id) {
            (
                State::Answered {
                    message_id: Some(sent),
                    ..
                },
                Some(reporting),
            ) => sent == reporting,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A turn that has already reported, built through the transition rather
    /// than by naming the private state — which is the point of sealing it.
    fn already_answered(message_id: Option<&str>) -> PhoneTurn {
        let mut turn = PhoneTurn::start(std::time::Instant::now(), 0);
        assert!(turn.answer(message_id.map(str::to_string)));
        turn
    }

    fn tool_call(title: &str) -> ChatItem {
        use daruda_acp::{ToolCallItem, ToolKindView, ToolStatusView};
        ChatItem::ToolCall(ToolCallItem {
            id: "tool-1".to_string(),
            title: title.to_string(),
            kind: ToolKindView::Edit,
            tool_name: None,
            status: ToolStatusView::InProgress,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
            exit: None,
        })
    }

    fn assistant_text(text: &str, streaming: bool) -> ChatItem {
        ChatItem::AssistantText {
            text: text.to_string(),
            streaming,
            message_id: None,
            phase: Default::default(),
        }
    }

    fn thinking(text: &str) -> ChatItem {
        ChatItem::Thinking {
            text: text.to_string(),
            streaming: false,
            message_id: None,
        }
    }

    #[test]
    fn first_response_covers_text_tool_anchor_and_ignored_items() {
        let turn = PhoneTurn::start(std::time::Instant::now(), 0);
        let items = vec![thinking("pondering"), assistant_text("partial", true)];
        assert_eq!(turn.first_response(&items), None);

        let turn = PhoneTurn::start(std::time::Instant::now(), 0);
        let items = vec![thinking("hmm"), assistant_text("done", false)];
        assert_eq!(
            turn.first_response(&items),
            Some(FirstResponseOutcome::Text {
                text: "done".to_string(),
                message_id: None,
            })
        );

        let turn = PhoneTurn::start(std::time::Instant::now(), 0);
        let items = vec![thinking("hmm"), tool_call("Write /tmp/x.rs")];
        assert_eq!(
            turn.first_response(&items),
            Some(FirstResponseOutcome::Tool {
                tool_title: Some("Write /tmp/x.rs".to_string())
            })
        );

        // A prior turn's completed AssistantText, present *before* the watch's
        // anchor point, must not be mistaken for this turn's first response.
        let items = vec![assistant_text("previous turn's answer", false)];
        let turn = PhoneTurn::start(std::time::Instant::now(), items.len());
        assert_eq!(turn.first_response(&items), None);
    }

    /// The ledger's whole reason to exist: two relays report one turn, and this
    /// is where they agree. Identity, not text — the completion reports the last
    /// message and the question is whether that is the one already sent.
    #[test]
    fn already_sent_answers_only_for_the_very_message_that_went_out() {
        let waiting = PhoneTurn::start(std::time::Instant::now(), 0);
        assert!(
            !waiting.already_sent(Some("m1")),
            "nothing has gone out yet"
        );

        let answered = already_answered(Some("m1"));
        assert!(answered.already_sent(Some("m1")), "the same message");
        assert!(
            !answered.already_sent(Some("m2")),
            "a later message in the same turn is news"
        );
        assert!(
            !answered.already_sent(None),
            "an unnamed message cannot be matched, so it is reported"
        );

        // A tool ack, the fixed fallback ack, or an agent that omits ids: no agent
        // text reached the phone, so the completion still owes one.
        let acked_without_text = already_answered(None);
        assert!(!acked_without_text.already_sent(Some("m1")));
        assert!(!acked_without_text.already_sent(None));
    }

    /// A turn already answered is not waiting, so neither pump can ack it twice.
    #[test]
    fn an_answered_turn_resolves_nothing_and_is_never_overdue() {
        let answered = already_answered(Some("m1"));
        assert_eq!(
            answered.first_response(&[assistant_text("more", false)]),
            None
        );
        assert!(!answered.is_overdue(std::time::Instant::now(), 0));
    }

    #[test]
    fn is_overdue_boundary_at_exactly_the_timeout() {
        let started = std::time::Instant::now();
        let turn = PhoneTurn::start(started, 0);
        assert!(!turn.is_overdue(started + std::time::Duration::from_secs(59), 60));
        assert!(turn.is_overdue(started + std::time::Duration::from_secs(60), 60));
    }

    /// A message can carry no text at all — `daruda_acp` collapses a content block
    /// it cannot render to an empty string. Such a message must not become the
    /// reply the turn reports, or a notification arrives with nothing in it.
    #[test]
    fn a_reply_with_no_text_does_not_resolve_the_turn() {
        let turn = PhoneTurn::start(std::time::Instant::now(), 0);
        let items = [assistant_text("", false)];
        assert!(turn.first_response(&items).is_none());

        let items = [
            assistant_text("", false),
            assistant_text("real answer", false),
        ];
        assert_eq!(
            turn.first_response(&items),
            Some(FirstResponseOutcome::Text {
                text: "real answer".to_string(),
                message_id: None,
            }),
            "the turn waits for a message that has something to say"
        );
    }
}
