//! The composer's keys for the question it answers: ↑↓ move the card's
//! highlight, an empty Enter or a digit chooses a row, Space toggles a
//! multi-select option, Tab moves between fields. Like the other question
//! verbs these act on the oldest open question only.

use daruda_acp::{QuestionField, QuestionFieldKind};
use gpui::Context;

use crate::view::AgentChatView;
use crate::view::questions::{AnswerSource, OpenQuestion, QuestionFocus, QuestionHighlight};

/// The rows a primary field shows on its card, in order: its choices
/// (options, or yes and no), then the write row when it takes text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FieldRows {
    choices: usize,
    write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Choice(usize),
    Write,
}

impl FieldRows {
    fn of(question: &OpenQuestion, field: &QuestionField) -> Self {
        let choices = match &field.kind {
            QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options) => {
                options.len()
            }
            QuestionFieldKind::Boolean => 2,
            QuestionFieldKind::Text => 0,
        };
        Self {
            choices,
            write: question.text_key(field).is_some(),
        }
    }

    fn len(self) -> usize {
        self.choices + usize::from(self.write)
    }

    fn row(self, index: usize) -> Option<Row> {
        if index < self.choices {
            Some(Row::Choice(index))
        } else if index == self.choices && self.write {
            Some(Row::Write)
        } else {
            None
        }
    }
}

impl OpenQuestion {
    fn focused_rows(&self) -> Option<FieldRows> {
        self.focused_field().map(|field| FieldRows::of(self, field))
    }

    /// Whether the card's send button follows the focused field: it is the
    /// last field and the card offers one.
    fn send_follows(&self) -> bool {
        self.focus.field + 1 == self.primary_count() && self.offers_send()
    }
}

impl AgentChatView {
    /// Up (`down == false`) or down in an empty bound composer. With nothing
    /// lit, down lights the first row and up the last; after that the
    /// highlight moves one row and stops at either end. Past the last row of
    /// the last field it lands on the card's send button. Moving is working
    /// on the question, so it stops the countdown, as Tab does.
    pub fn move_question_row(&mut self, id: u64, down: bool, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return;
            };
            let last = question
                .focused_rows()
                .and_then(|rows| rows.len().checked_sub(1));
            let send = question.send_follows().then_some(QuestionHighlight::Send);
            let row = |n: Option<usize>| n.map(QuestionHighlight::Row);
            let next = match (question.focus.highlight, down) {
                (None, true) => row(last.map(|_| 0)).or(send),
                (None, false) => row(last).or(send),
                (Some(QuestionHighlight::Row(n)), true) if Some(n) >= last => send.or(row(last)),
                (Some(QuestionHighlight::Row(n)), true) => row(Some(n + 1)),
                (Some(QuestionHighlight::Row(n)), false) => row(Some(n.saturating_sub(1))),
                (Some(QuestionHighlight::Send), true) => send,
                (Some(QuestionHighlight::Send), false) => row(last),
            };
            if next.is_some() && question.focus.highlight != next {
                question.focus.highlight = next;
                question.stop_countdown();
                cx.notify();
            }
        });
    }

    /// An empty Enter in the bound composer: choose the lit row, or send from
    /// the lit send button as the button does. With nothing lit, a form with
    /// every field answered is sent.
    pub fn choose_question_row(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return;
            };
            match question.focus.highlight {
                Some(QuestionHighlight::Row(row)) => view.choose_row(id, row, true, cx),
                Some(QuestionHighlight::Send) => view.submit_filled(id, AnswerSource::Desktop, cx),
                None if question.all_answered() => {
                    view.submit_filled(id, AnswerSource::Desktop, cx)
                }
                None => {}
            }
        });
    }

    /// Digit `n` in an empty bound composer: choose row `n` as the card
    /// numbers it. A pick never sends here, since the digit may be the
    /// start of a prompt; the empty Enter that follows sends. `false` when the
    /// field has no such row, or no choices at all (a text field), so the
    /// digit is typed instead.
    pub fn choose_question_number(&mut self, id: u64, n: usize, cx: &mut Context<Self>) -> bool {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return false;
            };
            let Some(rows) = question.focused_rows().filter(|rows| rows.choices > 0) else {
                return false;
            };
            let Some(row) = n.checked_sub(1).filter(|&row| rows.row(row).is_some()) else {
                return false;
            };
            question.focus.highlight = Some(QuestionHighlight::Row(row));
            let field = question.focus.field;
            view.choose_row(id, row, false, cx);
            if let Some(question) = view.questions.oldest_mut(id)
                && question.focus.field == field
                && !question.offers_send()
            {
                question.focus.highlight = Some(QuestionHighlight::Row(row));
            }
            true
        })
    }

    /// Space in an empty bound composer: toggle the lit option of a
    /// multi-select. `false` on any other field, so the space is typed.
    pub fn toggle_question_row(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return false;
            };
            if !question
                .focused_field()
                .is_some_and(|f| matches!(f.kind, QuestionFieldKind::MultiSelect(_)))
            {
                return false;
            }
            let rows = question.focused_rows();
            let lit = match question.focus.highlight {
                Some(QuestionHighlight::Row(row)) => rows.and_then(|rows| rows.row(row)),
                _ => None,
            };
            if let Some(Row::Choice(option)) = lit {
                let field = question.focus.field;
                view.pick_option(id, field, option, false, cx);
            }
            true
        })
    }

    /// Typing in the bound composer ends keyboard choosing: the
    /// lit row or send button goes dark, so a later empty Enter picks nothing
    /// the person has since typed over.
    pub fn clear_question_highlight(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return;
            };
            if question.focus.highlight.take().is_some() {
                cx.notify();
            }
        });
    }

    /// Tab (`forward`) or Shift+Tab in the bound composer. `composer_text`
    /// was typed for the field being left, so it is committed there first;
    /// a field that takes no text refuses it and focus stays. Blank text
    /// leaves the field's committed answer as it is. Tab past the last field
    /// lights the send button; Shift+Tab from there returns to that field.
    /// `false` when focus did not move.
    pub fn move_question_field(
        &mut self,
        id: u64,
        forward: bool,
        composer_text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return false;
            };
            let from = question.focus.field;
            let on_send = question.send_highlighted();
            let target = match (forward, on_send) {
                (true, true) => return false,
                (false, true) => QuestionFocus {
                    field: from,
                    highlight: None,
                },
                (true, false) if from + 1 < question.primary_count() => QuestionFocus {
                    field: from + 1,
                    highlight: None,
                },
                (true, false) if question.send_follows() => QuestionFocus {
                    field: from,
                    highlight: Some(QuestionHighlight::Send),
                },
                (true, false) => return false,
                (false, false) => match from.checked_sub(1) {
                    Some(field) => QuestionFocus {
                        field,
                        highlight: None,
                    },
                    None => return false,
                },
            };
            let text = composer_text.trim();
            if !on_send && !text.is_empty() && !question.commit_text_for(from, text) {
                return false;
            }
            question.focus = target;
            question.stop_countdown();
            view.remeasure_question(id, "question-field-moved");
            cx.notify();
            true
        })
    }

    /// Choose row `row` of the focused field: pick or toggle an option,
    /// answer a yes/no, or, on the write row, ask the host for the composer.
    /// `may_send` lets a lone select's pick send the form.
    fn choose_row(&mut self, id: u64, row: usize, may_send: bool, cx: &mut Context<Self>) {
        let Some(question) = self.questions.oldest_mut(id) else {
            return;
        };
        let field = question.focus.field;
        let Some(focused) = question.focused_field() else {
            return;
        };
        let boolean = matches!(focused.kind, QuestionFieldKind::Boolean);
        match FieldRows::of(question, focused).row(row) {
            Some(Row::Choice(choice)) if boolean => self.set_flag(id, field, choice == 0, cx),
            Some(Row::Choice(option)) => self.pick_option(id, field, option, may_send, cx),
            Some(Row::Write) => self.request_question_composer(id, field, cx),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use daruda_acp::{QuestionDelivery, QuestionOption, QuestionRequest};

    use super::*;
    use crate::view::questions::{ComposerBinding, QuestionDraft};

    fn field(key: &str, kind: QuestionFieldKind, companion_of: Option<&str>) -> QuestionField {
        QuestionField {
            key: key.into(),
            title: None,
            description: None,
            required: false,
            kind,
            companion_of: companion_of.map(str::to_owned),
            default: None,
        }
    }

    fn open(fields: Vec<QuestionField>) -> OpenQuestion {
        let request = QuestionRequest {
            message: "?".into(),
            tool_call_id: None,
            delivery: QuestionDelivery::Blocking,
            fields,
        };
        OpenQuestion {
            id: 1,
            draft: QuestionDraft::seeded(&request),
            request,
            focus: QuestionFocus::default(),
            message_expanded: false,
            composer: ComposerBinding::Released,
            requested_field: None,
            told_to_phone: false,
            asked_at: std::time::Instant::now(),
            deadline: None,
        }
    }

    #[test]
    fn rows_follow_the_card_choices_then_the_write_row() {
        let option = |v: &str| QuestionOption {
            value: v.into(),
            label: v.into(),
            description: None,
        };
        let question = open(vec![
            field(
                "pick",
                QuestionFieldKind::SingleSelect(vec![option("a"), option("b")]),
                None,
            ),
            field("pick_other", QuestionFieldKind::Text, Some("pick")),
            field("ok", QuestionFieldKind::Boolean, None),
            field("why", QuestionFieldKind::Text, None),
        ]);
        let rows: Vec<FieldRows> = crate::view::questions::primary_fields(&question.request)
            .map(|f| FieldRows::of(&question, f))
            .collect();
        assert_eq!(
            rows,
            [
                FieldRows {
                    choices: 2,
                    write: true
                },
                FieldRows {
                    choices: 2,
                    write: false
                },
                FieldRows {
                    choices: 0,
                    write: true
                },
            ]
        );
        assert_eq!(rows[0].row(2), Some(Row::Write));
        assert_eq!(rows[1].row(2), None);
        assert_eq!(rows[2].row(0), Some(Row::Write));
    }
}
