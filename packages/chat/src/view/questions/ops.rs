//! What the person and the agent do to open questions: ask, pick, answer,
//! skip, withdraw, cancel. Each operation settles through one path so the
//! record, fold and row heights stay in step, and each tells the host once
//! when the composer summary it reads has changed.

use daruda_acp::{QuestionAnswer, QuestionFieldKind, QuestionRequest};
use gpui::Context;

use super::draft::{companion_key, picks_answer, primary_fields};
use super::{AnswerSource, QuestionOutcome};
use crate::agent_chat_helpers::fold_key_item_index;
use crate::fold::FoldKey;
use crate::view::host_surface::question_summary;
use crate::view::list_sync::ListSync;
use crate::view::{AgentChatEvent, AgentChatView};

impl AgentChatView {
    /// Whether a question may take the composer: it holds no prompt draft
    /// (the host's hint), no attachments, and no queued prompt being edited.
    pub(in crate::view) fn composer_free(&self, composer_empty: bool) -> bool {
        composer_empty && self.draft_attachments.is_empty() && self.queue.editing().is_none()
    }

    /// Run one operation, then tell the host if what `question_composer`
    /// reports changed. Public operations wrap their body once and share
    /// unwrapped steps, so one operation announces at most once.
    pub(in crate::view) fn announcing_question_composer<R>(
        &mut self,
        cx: &mut Context<Self>,
        op: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> R {
        let before = self.question_composer();
        let result = op(self, cx);
        if self.question_composer() != before {
            cx.emit(AgentChatEvent::QuestionComposerChanged);
        }
        result
    }

    pub(in crate::view) fn ask_question(
        &mut self,
        id: u64,
        request: QuestionRequest,
        composer_empty: bool,
        cx: &mut Context<Self>,
    ) {
        self.announcing_question_composer(cx, |view, _| {
            view.enqueue_question(id, request, composer_empty)
        });
    }

    /// [`Self::ask_question`] without the announcement, for a caller that
    /// queues several in one operation.
    pub(in crate::view) fn enqueue_question(
        &mut self,
        id: u64,
        request: QuestionRequest,
        composer_empty: bool,
    ) {
        let may_bind = self.composer_free(composer_empty);
        self.questions.ask(id, request, may_bind);
    }

    /// The agent took question `id` back. It already settled the request,
    /// so nothing is sent; the record says it was withdrawn.
    pub(in crate::view) fn withdraw_question(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            view.settle_question(
                id,
                None,
                QuestionOutcome::Withdrawn,
                AnswerSource::Desktop,
                cx,
            )
        });
    }

    /// The card's write row: answer field `field` in the composer. The view
    /// cannot see the composer, so it notes the field and asks the host to
    /// focus it; the host commits any bound text, then binds with its word on
    /// the draft, and that bind lands on `field`.
    pub(crate) fn request_question_composer(
        &mut self,
        id: u64,
        field: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.oldest_mut(id) else {
            return;
        };
        let Some(target) = primary_fields(&question.request).nth(field) else {
            return;
        };
        let prefill = question
            .text_key(target)
            .and_then(|key| question.draft.text(key))
            .map(str::to_owned);
        question.requested_field = Some(field);
        cx.emit(AgentChatEvent::FocusComposer {
            question_id: id,
            prefill,
        });
    }

    /// The card's option click: pick option `option` of primary field
    /// `field`. A single select keeps one pick and moves focus on; a
    /// multi-select toggles in place. A form that is one select sends on the
    /// click, unless an "Other" answer is already committed beside it.
    pub fn pick_question_option(
        &mut self,
        id: u64,
        field: usize,
        option: usize,
        cx: &mut Context<Self>,
    ) {
        self.announcing_question_composer(cx, |view, cx| {
            view.pick_option(id, field, option, true, cx)
        });
    }

    /// Pick an option as [`Self::pick_question_option`] does. `may_send`
    /// lets a lone select's pick send the form; a digit typed into an empty
    /// composer only picks, since it may be the start of a prompt.
    pub(in crate::view) fn pick_option(
        &mut self,
        id: u64,
        field: usize,
        option: usize,
        may_send: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.get_mut(id) else {
            return;
        };
        let Some(target) = primary_fields(&question.request).nth(field) else {
            return;
        };
        let (QuestionFieldKind::SingleSelect(options) | QuestionFieldKind::MultiSelect(options)) =
            &target.kind
        else {
            return;
        };
        let Some(value) = options.get(option).map(|o| o.value.clone()) else {
            return;
        };
        let single = matches!(target.kind, QuestionFieldKind::SingleSelect(_));
        let typed_beside = companion_key(&question.request, &target.key)
            .is_some_and(|key| question.draft.text(key).is_some());
        question.draft.pick(target, &value);
        if single {
            if may_send && picks_answer(&question.request) && !typed_beside {
                self.submit_filled(id, AnswerSource::Desktop, cx);
                return;
            }
            question.advance_focus(field);
        }
        self.remeasure_question(id, "question-picked");
        cx.notify();
    }

    /// Answer the yes/no primary field `field` and move focus on, as a
    /// single-select pick does.
    pub fn set_question_flag(
        &mut self,
        id: u64,
        field: usize,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        self.announcing_question_composer(cx, |view, cx| view.set_flag(id, field, value, cx));
    }

    pub(in crate::view) fn set_flag(
        &mut self,
        id: u64,
        field: usize,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.get_mut(id) else {
            return;
        };
        let Some(target) = primary_fields(&question.request).nth(field) else {
            return;
        };
        if !matches!(target.kind, QuestionFieldKind::Boolean) {
            return;
        }
        question.draft.set_flag(&target.key, value);
        question.advance_focus(field);
        self.remeasure_question(id, "question-flag");
        cx.notify();
    }

    /// The card's send button. Text typed in the composer bound to this
    /// question is part of the answer, but only the host can read it: the
    /// host is asked to commit it and then call [`Self::submit_question`].
    pub fn submit_question_from_card(&mut self, id: u64, cx: &mut Context<Self>) {
        let typed_in_composer = !self.composer_hint_empty
            && self
                .questions
                .first()
                .is_some_and(|q| q.id == id && q.composer.is_bound());
        if typed_in_composer {
            cx.emit(AgentChatEvent::SubmitQuestion { question_id: id });
        } else {
            self.submit_question(id, cx);
        }
    }

    /// Send the filled form. Does nothing while a required field is empty.
    pub fn submit_question(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            view.submit_filled(id, AnswerSource::Desktop, cx)
        });
    }

    pub(in crate::view) fn submit_filled(
        &mut self,
        id: u64,
        from: AnswerSource,
        cx: &mut Context<Self>,
    ) {
        self.submit_with(id, from, self.composer_hint_empty, cx);
    }

    /// [`Self::submit_filled`] with what the composer will hold once this
    /// returns, which decides whether the next question may bind it.
    pub(in crate::view) fn submit_with(
        &mut self,
        id: u64,
        from: AnswerSource,
        composer_empty: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(values) = self
            .questions
            .get(id)
            .and_then(|q| q.draft.answer(&q.request))
        else {
            return;
        };
        self.settle_with(
            id,
            Some(QuestionAnswer::Submit(values.clone())),
            QuestionOutcome::Answered(values),
            from,
            composer_empty,
            cx,
        );
    }

    /// Answer "skipped": the agent is told the person chose not to answer.
    pub fn skip_question(&mut self, id: u64, cx: &mut Context<Self>) {
        self.resolve_question(id, QuestionAnswer::Decline, QuestionOutcome::Skipped, cx);
    }

    /// Send `answer` for question `id` and keep `outcome` as its record. A
    /// question already settled is left alone, so a late answer is a no-op.
    pub(crate) fn resolve_question(
        &mut self,
        id: u64,
        answer: QuestionAnswer,
        outcome: QuestionOutcome,
        cx: &mut Context<Self>,
    ) {
        self.announcing_question_composer(cx, |view, cx| {
            view.settle_question(id, Some(answer), outcome, AnswerSource::Desktop, cx)
        });
    }

    /// Cancel every open question: the turn they belonged to is over. The
    /// caller is a teardown path that rebuilds the rows itself.
    pub(in crate::view) fn cancel_pending_questions(&mut self, cx: &mut Context<Self>) {
        self.cancel_open_questions(true, cx);
    }

    /// Cancel every open question and forget every record: the transcript
    /// their tool rows belong to is being replaced, so nothing is folded.
    pub(in crate::view) fn reset_questions(&mut self, cx: &mut Context<Self>) {
        self.cancel_open_questions(false, cx);
        self.questions.records.clear();
    }

    fn cancel_open_questions(&mut self, fold_records: bool, cx: &mut Context<Self>) {
        if self.questions.is_empty() {
            return;
        }
        for question in std::mem::take(&mut self.questions.open) {
            if let Some(handle) = self.any_handle() {
                handle.respond_question(question.id, QuestionAnswer::Cancel);
            }
            if fold_records && let Some(tool) = question.tool_call_id() {
                self.fold_settled_question(tool);
            }
            cx.emit(AgentChatEvent::QuestionSettled {
                session: self.permission_session,
                question_id: question.id,
                question: question_summary(&question.request),
                end: QuestionOutcome::Cancelled.end(),
                from_phone: false,
            });
            self.questions
                .settle(question, QuestionOutcome::Cancelled, false);
        }
        cx.emit(AgentChatEvent::QuestionComposerChanged);
        cx.notify();
    }

    /// Close question `id`, sending `reply` unless the agent already
    /// settled it, and reflow the row that shows it. The host hears which
    /// surface settled it, so the phone card left behind can say so.
    pub(in crate::view) fn settle_question(
        &mut self,
        id: u64,
        reply: Option<QuestionAnswer>,
        outcome: QuestionOutcome,
        from: AnswerSource,
        cx: &mut Context<Self>,
    ) {
        self.settle_with(id, reply, outcome, from, self.composer_hint_empty, cx);
    }

    /// [`Self::settle_question`], where `composer_empty` says whether the
    /// composer is free for the next question to bind.
    fn settle_with(
        &mut self,
        id: u64,
        reply: Option<QuestionAnswer>,
        outcome: QuestionOutcome,
        from: AnswerSource,
        composer_empty: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.take(id) else {
            return;
        };
        cx.emit(AgentChatEvent::QuestionSettled {
            session: self.permission_session,
            question_id: id,
            question: question_summary(&question.request),
            end: outcome.end(),
            from_phone: from == AnswerSource::Phone,
        });
        if let Some(answer) = reply
            && let Some(handle) = self.live_handle()
        {
            handle.respond_question(id, answer);
        }
        let tool = question.tool_call_id().map(str::to_owned);
        let may_bind = self.composer_free(composer_empty);
        self.questions.settle(question, outcome, may_bind);
        // Before the rebuild, so the remeasure below sees the folded height.
        if let Some(tool) = tool.as_deref() {
            self.fold_settled_question(tool);
        }
        self.rebuild_rows();
        // The row trades its open form for a record of another height.
        match self.question_row(tool.as_deref()) {
            Some(row) => self.apply_list_sync(ListSync::Rows(row..row + 1), "question-resolved"),
            None => self.resync_all_row_heights("question-resolved"),
        }
        cx.notify();
    }

    /// Expand only the message, leaving the form and its keyboard focus intact.
    pub(crate) fn toggle_question_message(&mut self, id: u64, cx: &mut Context<Self>) {
        self.announcing_question_composer(cx, |view, cx| {
            let Some(question) = view.questions.get_mut(id) else {
                return;
            };
            question.message_expanded = !question.message_expanded;
            question.stop_countdown();
            view.remeasure_question(id, "question-message-toggled");
            cx.notify();
        });
    }

    /// Remeasure the row that shows open question `id`, if it has one; one
    /// without a row is shown outside the list.
    pub(in crate::view) fn remeasure_question(&mut self, id: u64, reason: &str) {
        let tool = self
            .questions
            .get(id)
            .and_then(|q| q.tool_call_id())
            .map(str::to_owned);
        if let Some(row) = self.question_row(tool.as_deref()) {
            self.apply_list_sync(ListSync::Rows(row..row + 1), reason);
        }
    }

    /// The transcript row holding tool call `tool`: its own card, or the
    /// parent card a nested call renders inside.
    pub(crate) fn question_row(&self, tool: Option<&str>) -> Option<usize> {
        let item = fold_key_item_index(&FoldKey::Tool(tool?.to_owned()), &self.items)?;
        crate::rows::item_row(&self.rows, item)
    }
}
