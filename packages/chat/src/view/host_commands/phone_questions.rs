//! The phone's side of an open question: which field to show it, and what
//! its tap or reply does. The phone answers one field at a time and commits
//! to the field its card named, never to whatever the desktop has focused;
//! the composer, its binding and its uncommitted text are left alone.

use daruda_acp::{QuestionAnswer, QuestionFieldKind};
use gpui::Context;

use crate::strings as s;
use crate::view::AgentChatView;
use crate::view::questions::{
    AnswerSource, OpenQuestion, QuestionCountdown, QuestionOutcome, field_header, primary_fields,
};

/// One field of an open question, as the phone shows it. Plain values: the
/// agent's words verbatim, and button labels in card order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneQuestion {
    pub id: u64,
    /// Index among the question's primary fields.
    pub field: usize,
    /// Distinct per card sent, so a re-sent field is a new card.
    pub step: u64,
    /// The field's title, or the question's message when it has none.
    pub header: String,
    pub message: String,
    /// Choice labels; a yes/no field offers Yes then No. Empty for a field
    /// answered only in text.
    pub options: Vec<String>,
    /// Indices into `options` already chosen.
    pub picked: Vec<usize>,
    /// Current free-text answer, including a schema default the phone must review.
    pub answer_text: Option<Box<str>>,
    /// Several choices toggle, then a send button ends the field.
    pub multi: bool,
    /// A text reply to the card answers this field.
    pub takes_text: bool,
    /// The field may be left blank on its own, without skipping the form.
    pub optional: bool,
    /// `(field + 1, primary field count)`.
    pub position: (usize, usize),
    pub asked_by: String,
    /// What the answer timeout leaves when the card is built. A card is not
    /// edited as the time runs down, so this is the time at sending.
    pub time_left: Option<QuestionCountdown>,
}

/// What the phone sent for one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneReply {
    /// Choice `n` of the field's `options`.
    Option(usize),
    Text(String),
    /// Done with a multi-select field.
    Send,
    /// Leave this optional field blank and move on.
    SkipField,
    /// Decline the whole question.
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneAnswerResult {
    /// The question still waits; show the phone this field next. The same
    /// field again after a toggle, with its new picks.
    Next(PhoneQuestion),
    Submitted,
    Skipped,
    /// The question is no longer open, or the card named nothing it asks.
    AlreadySettled,
    /// The field takes no text; nothing changed.
    TextNotTaken,
}

impl AgentChatView {
    /// One card per open question the phone has not been sent, on the field
    /// the desktop has focused. Steps are minted here, so call this once per
    /// relay and mark what was sent with [`Self::mark_question_told_to_phone`].
    pub fn take_questions_untold_to_phone(&mut self) -> Vec<PhoneQuestion> {
        let untold: Vec<(u64, usize)> = self
            .questions
            .open()
            .filter(|q| !q.told_to_phone)
            .map(|q| (q.id, q.focus.field))
            .collect();
        let agent = self.agent_name.clone();
        untold
            .into_iter()
            .filter_map(|(id, field)| {
                let step = self.questions.mint_phone_step();
                let question = self.questions.open().find(|q| q.id == id)?;
                let card = phone_question(question, field, step, &agent);
                let draft = question.draft.clone();
                self.questions.phone_drafts.insert((id, field), draft);
                card
            })
            .collect()
    }

    /// The phone has been sent a card for question `id`.
    pub fn mark_question_told_to_phone(&mut self, id: u64) {
        if let Some(question) = self.questions.get_mut(id) {
            question.told_to_phone = true;
        }
    }

    /// The phone recipient changed: forget which permission requests and
    /// questions the old one was told about, so the next sweep tells the
    /// new one.
    pub fn forget_told_to_phone(&mut self) {
        self.permissions_told_to_phone.clear();
        for question in self.questions.open_mut() {
            question.told_to_phone = false;
        }
    }

    /// Apply the phone's `reply` to primary field `field` of question `id`.
    /// A pick, text or [`PhoneReply::SkipField`] moves on to the next field
    /// neither answered nor left blank; when none is left the form is sent
    /// through the desktop's gate, so a field left blank goes as skipped.
    pub fn answer_question_from_phone(
        &mut self,
        id: u64,
        field: usize,
        reply: PhoneReply,
        cx: &mut Context<Self>,
    ) -> PhoneAnswerResult {
        self.announcing_question_composer(cx, |view, cx| {
            view.answer_from_phone(id, field, reply, cx)
        })
    }

    fn answer_from_phone(
        &mut self,
        id: u64,
        field: usize,
        reply: PhoneReply,
        cx: &mut Context<Self>,
    ) -> PhoneAnswerResult {
        if !matches!(reply, PhoneReply::Skip)
            && self.questions.get(id).is_some_and(|q| {
                self.questions
                    .phone_drafts
                    .get(&(id, field))
                    .is_some_and(|shown| shown != &q.draft)
            })
        {
            if let Some(question) = self.questions.get_mut(id) {
                question.draft.require_review();
            }
            return self.phone_card_for(id, field, cx);
        }
        let Some(question) = self.questions.get_mut(id) else {
            return PhoneAnswerResult::AlreadySettled;
        };
        let Some(target) = primary_fields(&question.request).nth(field).cloned() else {
            return PhoneAnswerResult::AlreadySettled;
        };
        match reply {
            PhoneReply::Skip => {
                self.settle_question(
                    id,
                    Some(QuestionAnswer::Decline),
                    QuestionOutcome::Skipped,
                    AnswerSource::Phone,
                    cx,
                );
                return PhoneAnswerResult::Skipped;
            }
            // Straight to the draft: the composer's binding and its hint
            // belong to the desktop.
            PhoneReply::Text(text) => match question.text_key(&target).map(str::to_owned) {
                Some(key) => question.draft.commit_text(&key, &text),
                None => return PhoneAnswerResult::TextNotTaken,
            },
            PhoneReply::Send => {}
            PhoneReply::SkipField if !target.required => {
                question.draft.leave_blank(&question.request, &target.key);
            }
            PhoneReply::SkipField => return PhoneAnswerResult::AlreadySettled,
            PhoneReply::Option(ix) => match &target.kind {
                QuestionFieldKind::SingleSelect(options)
                | QuestionFieldKind::MultiSelect(options) => {
                    let Some(value) = options.get(ix).map(|o| o.value.clone()) else {
                        return PhoneAnswerResult::AlreadySettled;
                    };
                    question.draft.pick(&target, &value);
                    if matches!(target.kind, QuestionFieldKind::MultiSelect(_)) {
                        return self.phone_card_for(id, field, cx);
                    }
                }
                QuestionFieldKind::Boolean if ix < 2 => {
                    question.draft.set_flag(&target.key, ix == 0);
                }
                QuestionFieldKind::Boolean | QuestionFieldKind::Text => {
                    return PhoneAnswerResult::AlreadySettled;
                }
            },
        }
        // What the card showed is what the person answered, default included.
        question.draft.confirm(&target.key);
        self.phone_move_on(id, field, cx)
    }

    /// After field `from`: the next field neither answered by the person nor
    /// left blank, wrapping; else send the form; else `from` again, when it is a required
    /// field still empty. A form left wholly blank has nothing to send, so it
    /// is declined, as if the person skipped every question.
    fn phone_move_on(&mut self, id: u64, from: usize, cx: &mut Context<Self>) -> PhoneAnswerResult {
        let Some(question) = self.questions.get(id) else {
            return PhoneAnswerResult::AlreadySettled;
        };
        let keys: Vec<&str> = primary_fields(&question.request)
            .map(|f| f.key.as_str())
            .collect();
        // A field holding only its default was never shown on the phone, so
        // it is asked rather than sent unseen.
        let next = (from + 1..keys.len()).chain(0..from).find(|&ix| {
            !question
                .draft
                .is_settled_by_person(&question.request, keys[ix])
        });
        if let Some(next) = next {
            return self.phone_card_for(id, next, cx);
        }
        if question.draft.answer(&question.request).is_some() {
            self.submit_filled(id, AnswerSource::Phone, cx);
            return PhoneAnswerResult::Submitted;
        }
        let required_missing = primary_fields(&question.request)
            .any(|f| f.required && !question.draft.is_answered(&question.request, &f.key));
        if !required_missing {
            self.settle_question(
                id,
                Some(QuestionAnswer::Decline),
                QuestionOutcome::Skipped,
                AnswerSource::Phone,
                cx,
            );
            return PhoneAnswerResult::Skipped;
        }
        self.phone_card_for(id, from, cx)
    }

    /// Redraw the card the draft just changed and hand back field `field`'s
    /// phone card.
    fn phone_card_for(
        &mut self,
        id: u64,
        field: usize,
        cx: &mut Context<Self>,
    ) -> PhoneAnswerResult {
        self.remeasure_question(id, "question-phone-answer");
        cx.notify();
        let step = self.questions.mint_phone_step();
        let agent = self.agent_name.clone();
        if let Some(question) = self.questions.get(id) {
            let draft = question.draft.clone();
            self.questions.phone_drafts.insert((id, field), draft);
        }
        self.questions
            .open()
            .find(|q| q.id == id)
            .and_then(|q| phone_question(q, field, step, &agent))
            .map_or(PhoneAnswerResult::AlreadySettled, PhoneAnswerResult::Next)
    }
}

/// Primary field `field` of `question` as the phone shows it.
fn phone_question(
    question: &OpenQuestion,
    field: usize,
    step: u64,
    agent: &str,
) -> Option<PhoneQuestion> {
    let target = primary_fields(&question.request).nth(field)?;
    let draft = &question.draft;
    let (options, picked): (Vec<String>, Vec<usize>) = match &target.kind {
        QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options) => (
            options.iter().map(|o| o.label.clone()).collect(),
            options
                .iter()
                .enumerate()
                .filter(|(_, o)| draft.is_picked(&target.key, &o.value))
                .map(|(ix, _)| ix)
                .collect(),
        ),
        QuestionFieldKind::Boolean => (
            vec![s::agent_chat::question_yes(), s::agent_chat::question_no()],
            draft
                .flag(&target.key)
                .map(|yes| vec![usize::from(!yes)])
                .unwrap_or_default(),
        ),
        QuestionFieldKind::Text => (Vec::new(), Vec::new()),
    };
    let header = field_header(&question.request, Some(target));
    Some(PhoneQuestion {
        id: question.id,
        field,
        step,
        header: header.to_owned(),
        message: question.request.message.clone(),
        options,
        picked,
        answer_text: question
            .text_key(target)
            .and_then(|key| draft.text(key))
            .map(Box::<str>::from),
        multi: matches!(target.kind, QuestionFieldKind::MultiSelect(_)),
        takes_text: question.text_key(target).is_some(),
        optional: !target.required,
        position: (field + 1, question.primary_count()),
        asked_by: agent.to_owned(),
        time_left: question
            .countdown()
            .map(|deadline| QuestionCountdown::until(deadline, std::time::Instant::now())),
    })
}

#[cfg(test)]
mod tests;
