//! The composer answering the focused agent pane's pending question: Enter,
//! Esc, the card's write row, the strip's actions, and the prompt a released
//! question would hold back. The binding is the view's; the host keeps
//! only what lets composer text land on the field it was typed for, plus the
//! strip's own marks.

mod enter;
mod strip;
mod sync;

use std::collections::HashMap;

use daruda_chat::view::QuestionComposer;
use gpui::App;

use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// What the host remembers about questions between syncs.
#[derive(Default)]
pub(in crate::workspace) struct QuestionMarks {
    /// The field each pane's composer was bound to at its last sync. A new
    /// field means a card moved focus while the text was typed for this one.
    bound: HashMap<PaneId, BoundField>,
    /// A prompt Enter held back because a question waits released.
    blocked_prompt: Option<BlockedPrompt>,
    /// The pane whose binding the last Esc released; its next Esc stops.
    esc_released: Option<PaneId>,
    /// The field an Enter last failed to answer with the composer's text,
    /// so the strip says why. Kept while that field stays bound and the
    /// composer holds text.
    text_refused: Option<(PaneId, BoundField)>,
    /// The placeholder last applied, so a keystroke does not reapply it.
    placeholder: QuestionPlaceholder,
    /// Panes whose composer text was typed as the answer to this question,
    /// so binding it again is no "draft became the answer".
    answer_text: HashMap<PaneId, AnsweredQuestion>,
    /// The strip's width at its last paint, for the narrow layout.
    strip_width: Option<f32>,
    /// The composer held text at the last sync. The arrow-key hook runs
    /// inside the composer's own update and cannot read it there.
    composer_has_text: bool,
    /// The composer's owner at the last hint sync. Saved drafts change only
    /// when the owner does, or through `set_pane_composer_text`, so a
    /// keystroke refreshes the owner's hint alone.
    hinted_owner: Option<PaneId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BoundField {
    question_id: u64,
    field: usize,
    /// The keyboard is on the card's send button, past `field`; the
    /// composer's text then belongs to no field.
    send: bool,
}

impl BoundField {
    fn of(q: &QuestionComposer) -> Self {
        Self {
            question_id: q.question_id,
            field: q.field,
            send: q.send_highlighted,
        }
    }
}

/// A question by its session as well as its id: ids restart with every
/// connection, so an id alone could name a question of an earlier session.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AnsweredQuestion {
    session: u64,
    question_id: u64,
}

struct BlockedPrompt {
    pane: PaneId,
    question_id: u64,
    text: String,
}

/// Which placeholder the composer shows for the focused pane's question.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::workspace) enum QuestionPlaceholder {
    #[default]
    Prompt,
    Answer,
    Note,
}

impl QuestionMarks {
    pub(in crate::workspace) fn forget_pane(&mut self, pane: PaneId) {
        self.bound.remove(&pane);
        self.answer_text.remove(&pane);
        if self.blocked_prompt.as_ref().is_some_and(|b| b.pane == pane) {
            self.blocked_prompt = None;
        }
        if self.esc_released == Some(pane) {
            self.esc_released = None;
        }
        if self.text_refused.is_some_and(|(p, _)| p == pane) {
            self.text_refused = None;
        }
    }
}

impl Workspace {
    pub(super) fn focused_question(&self, cx: &App) -> Option<(PaneId, QuestionComposer)> {
        let pane = self.active_runtime().focused_pane_id;
        let view = self.agent_chat_view(pane)?.read(cx);
        if view.is_read_only() {
            return None;
        }
        Some((pane, view.question_composer()?))
    }

    /// Whether the composer's Enter answers a question. Read from the
    /// composer's own hooks, so it only reads.
    pub(in crate::workspace) fn focused_question_bound(&self, cx: &App) -> bool {
        self.focused_question(cx).is_some_and(|(_, q)| q.bound)
    }

    pub(in crate::workspace) fn focused_question_placeholder(
        &self,
        cx: &App,
    ) -> QuestionPlaceholder {
        QuestionPlaceholder::of(self.focused_question(cx).as_ref().map(|(_, q)| q))
    }
}

impl QuestionPlaceholder {
    fn of(question: Option<&QuestionComposer>) -> Self {
        match question {
            Some(q) if q.bound && q.note_mode => Self::Note,
            Some(q) if q.bound => Self::Answer,
            _ => Self::Prompt,
        }
    }
}

#[cfg(test)]
mod tests;
