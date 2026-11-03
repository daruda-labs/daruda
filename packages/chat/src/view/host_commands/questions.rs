//! What the host tells a pane about the question its composer answers.
//! Every verb acts on the oldest open question only, the one the composer
//! may take; naming a newer one does nothing.

use std::time::{Duration, Instant};

use daruda_acp::QuestionAnswer;
use gpui::Context;

use crate::view::AgentChatView;
use crate::view::questions::{AnswerSource, ComposerBinding, QuestionFocus, QuestionOutcome};

impl AgentChatView {
    /// Whether the composer holds no prompt draft, kept current by the host:
    /// an arriving question may take an empty composer only. Text in a
    /// composer bound to a question is its answer being typed, which stops
    /// that question's countdown for good.
    pub fn set_composer_hint_empty(&mut self, empty: bool, cx: &mut Context<Self>) {
        self.composer_hint_empty = empty;
        if !empty
            && let Some(question) = self.questions.open_mut().next()
            && question.composer.is_bound()
            && question.countdown().is_some()
        {
            question.stop_countdown();
            cx.notify();
        }
    }

    /// How long a question may wait untouched before it is declined as
    /// expired; `None` turns the timeout off. Set at pane creation and on
    /// every config reload; an open question nobody touched follows it.
    pub fn set_question_timeout(&mut self, timeout: Option<Duration>, cx: &mut Context<Self>) {
        if self.questions.timeout == timeout {
            return;
        }
        self.questions.set_timeout(timeout, Instant::now());
        cx.notify();
    }

    /// Enter in a bound composer: commit `text` to the focused field and
    /// move on. The last field sends once every question is answered; until
    /// then focus goes to one still unanswered. `false` when the text was
    /// not taken (a yes/no field, or not the oldest question), so the host
    /// keeps the composer as it is.
    pub fn answer_question_text(&mut self, id: u64, text: String, cx: &mut Context<Self>) -> bool {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return false;
            };
            let field = question.focus.field;
            if !question.commit_text_for(field, &text) {
                return false;
            }
            // The host empties the composer once the text is taken, after this
            // returns, so the next question may bind it.
            if field + 1 == question.primary_count() && question.all_answered() {
                view.submit_with(id, AnswerSource::Desktop, true, cx);
                return true;
            }
            question.advance_after_text(field);
            view.remeasure_question(id, "question-answered-text");
            cx.notify();
            true
        })
    }

    /// Commit `text` to primary field `field` without moving focus. The host
    /// names the field its text was typed for: a card pick may already have
    /// moved focus on, and the text must not follow it. `false` when the
    /// field takes no text or the question is not the oldest.
    pub fn commit_question_text(
        &mut self,
        id: u64,
        field: usize,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return false;
            };
            if !question.commit_text_for(field, &text) {
                return false;
            }
            view.remeasure_question(id, "question-committed-text");
            cx.notify();
            true
        })
    }

    /// Bind the composer to the oldest open question, on `field` or else the
    /// one its card asked for. `draft_present` says the composer holds a
    /// prompt draft, which then starts the answer. Pending attachments or a
    /// queued prompt being edited leave it unbound.
    pub fn bind_question_composer(
        &mut self,
        id: u64,
        field: Option<usize>,
        draft_present: bool,
        cx: &mut Context<Self>,
    ) {
        self.announcing_question_composer(cx, |view, cx| {
            let composer_free = view.composer_free(true);
            let Some(question) = view.questions.oldest_mut(id) else {
                return;
            };
            // Taken even when the bind is refused, so a later bind cannot
            // jump to a field the person asked for some time ago.
            let requested = question.requested_field.take();
            if !composer_free {
                return;
            }
            let target = field.or(requested).unwrap_or(question.focus.field);
            if target >= question.primary_count() {
                return;
            }
            question.composer = ComposerBinding::Bound {
                draft_became_answer: draft_present && !question.composer.is_bound(),
            };
            // The person asked for the composer: they are answering.
            question.stop_countdown();
            if target != question.focus.field {
                question.focus = QuestionFocus {
                    field: target,
                    highlight: None,
                };
            }
            view.remeasure_question(id, "question-bound");
            cx.notify();
        });
    }

    /// Give the composer back to prompts. Text not yet committed stays in the
    /// composer as a prompt draft; committed answers stay on the card.
    pub fn release_question_composer(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.oldest_mut(id) else {
                return;
            };
            if question.composer.is_bound() {
                question.composer = ComposerBinding::Released;
                cx.notify();
            }
        });
    }

    /// Scroll the transcript so question `id`'s card starts at the top. One
    /// without a row is the panel below the list, always in view.
    pub fn reveal_question(&mut self, id: u64, cx: &mut Context<Self>) {
        let tool = self
            .questions
            .open()
            .find(|q| q.id == id)
            .and_then(|q| q.tool_call_id())
            .map(str::to_owned);
        if let Some(row) = self.question_row(tool.as_deref()) {
            self.list_state.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item: gpui::px(0.),
            });
            cx.notify();
        }
    }

    /// "Skip and send this": decline question `id` so the prompt queued
    /// behind it can go. The record says the person answered by prompt.
    pub fn skip_question_for_prompt(&mut self, id: u64, cx: &mut Context<Self>) {
        self.resolve_question(
            id,
            QuestionAnswer::Decline,
            QuestionOutcome::SkippedForPrompt,
            cx,
        );
    }
}
