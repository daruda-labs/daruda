//! Questions the agent asked mid-turn, waiting on the person's answer, and
//! the settled ones the transcript still shows.
//!
//! The turn stays open while a question waits, so these live beside the
//! permission index rather than in the transcript. Each open question keeps
//! plain answer data ([`QuestionDraft`]); text is typed in the composer and
//! committed here, so no widget state lives in this module.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use daruda_acp::{QuestionField, QuestionFieldKind, QuestionRequest};

use super::AgentChatView;

mod countdown;
mod draft;
mod ops;
mod record;

pub use countdown::QuestionCountdown;
pub(crate) use draft::{QuestionDraft, companion_key, picks_answer, primary_fields};
pub(crate) use record::{QuestionOutcome, QuestionRecord};

/// Open questions, oldest first, and the settled ones by tool call id.
#[derive(Default)]
pub(crate) struct Questions {
    open: Vec<OpenQuestion>,
    /// The last step number a phone card was sent with.
    phone_steps: u64,
    /// Questions without a tool call id have no row to show a record in, so
    /// they leave none.
    records: HashMap<String, QuestionRecord>,
    /// How long a question may wait untouched, as the host last said; `None`
    /// waits however long it takes.
    pub(in crate::view) timeout: Option<Duration>,
}

/// One open question and what the person has done with it so far.
pub(crate) struct OpenQuestion {
    pub(crate) id: u64,
    pub(crate) request: QuestionRequest,
    pub(crate) draft: QuestionDraft,
    pub(crate) focus: QuestionFocus,
    pub(crate) composer: ComposerBinding,
    /// The field the card's write row asked the composer for, applied by the
    /// host's next bind once it has committed the text bound so far.
    pub(crate) requested_field: Option<usize>,
    /// The phone has been sent a card for this question; the periodic
    /// relay leaves it alone until the recipient changes.
    pub(crate) told_to_phone: bool,
    pub(crate) asked_at: Instant,
    /// When the opt-in answer timeout declines this question, `None` while
    /// the timeout is off. Cleared for good once the person touches it; read
    /// through [`OpenQuestion::countdown`].
    pub(crate) deadline: Option<Instant>,
}

/// Where the keyboard is within an open question.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct QuestionFocus {
    /// Index among the primary (non-companion) fields.
    pub(crate) field: usize,
    /// What the keyboard has lit; `None` lights nothing.
    pub(crate) highlight: Option<QuestionHighlight>,
}

/// What the keyboard can light on an open question's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuestionHighlight {
    /// Row `n` of the focused field, in card order: its choices, then the
    /// write row.
    Row(usize),
    /// The card's send button, past the last row of the last field. The
    /// focused field stays the last one, so stepping back returns there.
    Send,
}

/// Whether the composer's Enter answers this question's focused field.
/// Only the oldest open question is ever bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerBinding {
    Bound {
        /// The person bound the composer over a prompt draft, which now
        /// starts the answer. Shown once: the next commit or bind clears it.
        draft_became_answer: bool,
    },
    /// The composer stays for prompts, keeping whatever draft it holds.
    Released,
}

/// Which surface's answer settles a question: the phone card the other
/// surface left behind is told which one won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::view) enum AnswerSource {
    Desktop,
    Phone,
}

impl ComposerBinding {
    pub(crate) fn is_bound(self) -> bool {
        matches!(self, Self::Bound { .. })
    }
}

impl Questions {
    /// The oldest open question: the only one the composer may answer.
    pub(crate) fn first(&self) -> Option<&OpenQuestion> {
        self.open.first()
    }

    /// Open questions, oldest first.
    pub(crate) fn open(&self) -> impl Iterator<Item = &OpenQuestion> {
        self.open.iter()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.open.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub(crate) fn open_for_tool(&self, tool_call_id: &str) -> Option<&OpenQuestion> {
        self.open
            .iter()
            .find(|q| q.request.tool_call_id.as_deref() == Some(tool_call_id))
    }

    pub(crate) fn record_for_tool(&self, tool_call_id: &str) -> Option<&QuestionRecord> {
        self.records.get(tool_call_id)
    }

    /// The tool rows an open question keeps on screen.
    pub(crate) fn pinned_tools(&self) -> impl Iterator<Item = &str> {
        self.open
            .iter()
            .filter_map(|q| q.request.tool_call_id.as_deref())
    }

    pub(crate) fn recorded_tools(&self) -> impl Iterator<Item = &str> {
        self.records.keys().map(String::as_str)
    }

    /// Queue question `id`. Only the oldest question may hold the composer,
    /// so a later one waits Released even when the person released an
    /// earlier one.
    fn ask(&mut self, id: u64, request: QuestionRequest, may_bind: bool) {
        let composer = if may_bind && self.open.is_empty() {
            ComposerBinding::Bound {
                draft_became_answer: false,
            }
        } else {
            ComposerBinding::Released
        };
        let asked_at = Instant::now();
        self.open.push(OpenQuestion {
            id,
            draft: QuestionDraft::seeded(&request),
            request,
            focus: QuestionFocus::default(),
            composer,
            requested_field: None,
            told_to_phone: false,
            asked_at,
            deadline: self.timeout.map(|timeout| asked_at + timeout),
        });
    }

    pub(in crate::view) fn get(&self, id: u64) -> Option<&OpenQuestion> {
        self.open.iter().find(|q| q.id == id)
    }

    /// A step number no phone card of this pane has carried.
    pub(in crate::view) fn mint_phone_step(&mut self) -> u64 {
        self.phone_steps += 1;
        self.phone_steps
    }

    pub(in crate::view) fn open_mut(&mut self) -> impl Iterator<Item = &mut OpenQuestion> {
        self.open.iter_mut()
    }

    pub(in crate::view) fn get_mut(&mut self, id: u64) -> Option<&mut OpenQuestion> {
        self.open.iter_mut().find(|q| q.id == id)
    }

    /// The oldest open question, when it is `id`.
    pub(in crate::view) fn oldest_mut(&mut self, id: u64) -> Option<&mut OpenQuestion> {
        self.open.first_mut().filter(|q| q.id == id)
    }

    fn take(&mut self, id: u64) -> Option<OpenQuestion> {
        let at = self.open.iter().position(|q| q.id == id)?;
        Some(self.open.remove(at))
    }

    /// Keep `question`'s outcome, and pass a released binding on to the
    /// oldest open question when the composer may take it.
    fn settle(&mut self, question: OpenQuestion, outcome: QuestionOutcome, may_bind: bool) {
        if question.composer.is_bound()
            && may_bind
            && let Some(next) = self.open.first_mut()
        {
            next.composer = ComposerBinding::Bound {
                draft_became_answer: false,
            };
        }
        if let Some(tool) = question.request.tool_call_id.clone() {
            self.records.insert(
                tool,
                QuestionRecord {
                    request: question.request,
                    outcome,
                },
            );
        }
    }
}

impl OpenQuestion {
    pub(crate) fn tool_call_id(&self) -> Option<&str> {
        self.request.tool_call_id.as_deref()
    }

    /// The primary field the keyboard is on.
    pub(crate) fn focused_field(&self) -> Option<&QuestionField> {
        primary_fields(&self.request).nth(self.focus.field)
    }

    /// The key free text answering primary field `field` is kept under: its
    /// own for a text field, its "Other" box for a select, none for yes/no.
    pub(crate) fn text_key<'a>(&'a self, field: &'a QuestionField) -> Option<&'a str> {
        match field.kind {
            QuestionFieldKind::Text => Some(field.key.as_str()),
            QuestionFieldKind::Boolean => None,
            QuestionFieldKind::SingleSelect(_) | QuestionFieldKind::MultiSelect(_) => {
                companion_key(&self.request, &field.key)
            }
        }
    }

    /// Whether text typed for `field` rides on a single pick as a note
    /// rather than standing as the answer.
    pub(crate) fn is_note(&self, field: &QuestionField) -> bool {
        match &field.kind {
            QuestionFieldKind::SingleSelect(options) => options
                .iter()
                .any(|o| self.draft.is_picked(&field.key, &o.value)),
            _ => false,
        }
    }

    pub(in crate::view) fn primary_count(&self) -> usize {
        primary_fields(&self.request).count()
    }

    pub(in crate::view) fn all_answered(&self) -> bool {
        primary_fields(&self.request).all(|f| self.draft.is_answered(&self.request, &f.key))
    }

    /// Commit `text` as primary field `field`'s free text; `false` when that
    /// field takes none. A commit ends the "draft became the answer" hint.
    pub(in crate::view) fn commit_text_for(&mut self, field: usize, text: &str) -> bool {
        let Some(key) = primary_fields(&self.request)
            .nth(field)
            .and_then(|field| self.text_key(field))
            .map(str::to_owned)
        else {
            return false;
        };
        self.draft.commit_text(&key, text);
        if self.composer.is_bound() {
            self.composer = ComposerBinding::Bound {
                draft_became_answer: false,
            };
        }
        true
    }

    /// Move focus on after text answered field `from`: to the next
    /// unanswered field, else to the next field, so Enter walks toward the
    /// last field, which sends.
    pub(in crate::view) fn advance_after_text(&mut self, from: usize) {
        self.advance_focus(from);
        if self.focus.field == from && self.all_answered() && from + 1 < self.primary_count() {
            self.focus.field = from + 1;
        }
    }

    /// Move focus on from primary field `from` to the next unanswered one,
    /// wrapping; stay on `from` when every other field is answered.
    fn advance_focus(&mut self, from: usize) {
        let keys: Vec<&str> = primary_fields(&self.request)
            .map(|f| f.key.as_str())
            .collect();
        let next = (from + 1..keys.len())
            .chain(0..from)
            .find(|&ix| !self.draft.is_answered(&self.request, keys[ix]))
            .unwrap_or(from);
        self.focus = QuestionFocus {
            field: next,
            highlight: None,
        };
    }

    /// Whether the card has a send button: anything but a lone select, or a
    /// lone select whose "Other" text is committed beside it (a pick then no
    /// longer sends on its own).
    pub(crate) fn offers_send(&self) -> bool {
        !picks_answer(&self.request)
            || primary_fields(&self.request).any(|field| {
                companion_key(&self.request, &field.key)
                    .is_some_and(|key| self.draft.text(key).is_some())
            })
    }

    /// Whether the keyboard is on the card's send button.
    pub(crate) fn send_highlighted(&self) -> bool {
        self.focus.highlight == Some(QuestionHighlight::Send)
    }
}

impl AgentChatView {
    pub(crate) fn questions(&self) -> &Questions {
        &self.questions
    }

    pub fn has_pending_question(&self) -> bool {
        !self.questions.is_empty()
    }
}

#[cfg(test)]
mod tests;
