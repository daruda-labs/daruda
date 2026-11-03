//! The strip's snapshot and actions, Esc, and the card's write row.

use daruda_chat::view::QuestionCountdown;
use gpui::{App, Context, Window};

use super::{AnsweredQuestion, BlockedPrompt, BoundField};
use crate::workspace::Workspace;
use crate::workspace::layout::QuestionStripSnapshot;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    pub(in crate::workspace) fn question_strip_snapshot(
        &self,
        cx: &App,
    ) -> Option<QuestionStripSnapshot> {
        let (pane, composer) = self.focused_question(cx)?;
        let marks = &self.input_dock.question;
        Some(QuestionStripSnapshot {
            pane_id: pane,
            pending_prompt: marks
                .blocked_prompt
                .as_ref()
                .filter(|b| b.pane == pane && b.question_id == composer.question_id)
                .map(|b| b.text.clone()),
            esc_again: !composer.bound && marks.esc_released == Some(pane),
            text_not_taken: composer.bound
                && marks.text_refused == Some((pane, BoundField::of(&composer))),
            // Read live rather than from the last sync: IME marked text sits in
            // the value without a `Change`, and must keep the digit keys off.
            composer_empty: self.input_dock.input.read(cx).value().is_empty(),
            time_left: composer
                .deadline
                .map(|deadline| QuestionCountdown::until(deadline, std::time::Instant::now())),
            narrow: marks
                .strip_width
                .is_some_and(|w| w <= crate::ui::theme::agent_chat_compact_options_w(cx)),
            composer,
        })
    }

    /// The strip's painted width, layout geometry from its `canvas`. The
    /// workspace repaints only when it crosses the narrow breakpoint, which
    /// scales with the pane font like the Activity Bar's.
    pub(in crate::workspace) fn set_question_strip_width(
        &mut self,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let breakpoint = crate::ui::theme::agent_chat_compact_options_w(cx);
        let marks = &mut self.input_dock.question;
        let was_narrow = marks.strip_width.is_some_and(|w| w <= breakpoint);
        marks.strip_width = Some(width);
        if (width <= breakpoint) != was_narrow {
            cx.notify();
        }
    }

    /// Esc while bound leaves answer mode; the strip then says the
    /// next Esc stops. `false` when not bound, so Esc goes on to Stop.
    pub(in crate::workspace) fn question_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((pane, question)) = self.focused_question(cx).filter(|(_, q)| q.bound) else {
            return false;
        };
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            view.update(cx, |v, cx| {
                v.release_question_composer(question.question_id, cx)
            });
        }
        self.input_dock.question.esc_released = Some(pane);
        cx.notify();
        true
    }

    /// The strip's "Use for prompt": text not yet committed stays as a
    /// prompt draft.
    pub(in crate::workspace) fn release_question_for_prompt(
        &mut self,
        pane: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        if let Some(question) = view.read(cx).question_composer() {
            view.update(cx, |v, cx| {
                v.release_question_composer(question.question_id, cx)
            });
        }
    }

    /// The strip's "✎ Answer": bind the question's current field, as its
    /// card's write row would.
    pub(in crate::workspace) fn answer_question_from_strip(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        let Some(question) = view.read(cx).question_composer() else {
            return;
        };
        let prefill = view
            .read(cx)
            .question_field_text(question.question_id, question.field);
        self.focus_composer_for_question(pane, question.question_id, prefill, window, cx);
    }

    /// The strip's "↑ Question".
    pub(in crate::workspace) fn reveal_pending_question(
        &mut self,
        pane: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        if let Some(question) = view.read(cx).question_composer() {
            view.update(cx, |v, cx| v.reveal_question(question.question_id, cx));
        }
    }

    /// `FocusComposer`: show and focus the composer, then bind it.
    /// Over a prompt draft the draft starts the answer and `prefill` is
    /// ignored; an empty composer takes `prefill`, the field's committed
    /// answer. Already bound, the field switch commits the text first.
    pub(in crate::workspace) fn focus_composer_for_question(
        &mut self,
        pane: PaneId,
        question_id: u64,
        prefill: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        // The card works with the dock shut, so writing opens it.
        if !self.docks.bottom.read(cx).is_open {
            self.docks.bottom.update(cx, |dock, cx| {
                dock.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        if self.active_runtime().focused_pane_id == pane {
            self.focus_pane(pane, window, cx);
        } else {
            self.focus_pane_on_click(pane, window, cx);
        }
        let was_bound = view
            .read(cx)
            .question_composer()
            .is_some_and(|q| q.question_id == question_id && q.bound);
        // Text typed as this question's answer and then released is still
        // its answer; only a prompt draft "became the answer".
        let answered = AnsweredQuestion {
            session: view.read(cx).permission_session(),
            question_id,
        };
        let own_answer = self.input_dock.question.answer_text.get(&pane) == Some(&answered);
        let draft_present =
            !was_bound && !own_answer && !self.input_dock.input.read(cx).value().trim().is_empty();
        view.update(cx, |v, cx| {
            v.bind_question_composer(question_id, None, draft_present, cx)
        });
        let bound_now = view
            .read(cx)
            .question_composer()
            .is_some_and(|q| q.question_id == question_id && q.bound);
        if bound_now
            && !was_bound
            && !draft_present
            && let Some(prefill) = prefill
        {
            self.set_pane_composer_text(pane, &prefill, window, cx);
        }
        self.sync_question_composer(window, cx);
    }

    /// `SubmitQuestion`: the card's send while the composer holds text typed
    /// for the bound field. The text is committed there first, as Enter
    /// would, so it goes with the answer; a field that takes no text keeps
    /// it in the composer and the strip says why.
    pub(in crate::workspace) fn submit_question_with_composer_text(
        &mut self,
        pane: PaneId,
        question_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        let Some(question) = view
            .read(cx)
            .question_composer()
            .filter(|q| q.question_id == question_id && q.bound)
        else {
            return;
        };
        let typed = self.pane_composer_text(pane, cx).trim().to_owned();
        if !typed.is_empty() {
            let taken = !self.question_swap_pending(pane, &question)
                && view.update(cx, |v, cx| {
                    v.commit_question_text(question_id, question.field, typed, cx)
                });
            if !taken {
                self.refuse_question_text(pane, &question, cx);
                return;
            }
            self.set_pane_composer_text(pane, "", window, cx);
            self.sync_question_composer(window, cx);
        }
        view.update(cx, |v, cx| v.submit_question(question_id, cx));
        self.sync_question_composer(window, cx);
    }

    /// "Skip and send this": decline the question, then send the held
    /// prompt, which the turn delivers once the agent goes on.
    pub(in crate::workspace) fn skip_question_and_send_prompt(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(blocked) = self.take_blocked_prompt(pane) else {
            return;
        };
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            view.update(cx, |v, cx| {
                v.skip_question_for_prompt(blocked.question_id, cx)
            });
        }
        self.send_composer_text(window, cx);
    }

    /// "Keep queued": send the held prompt; it waits behind the question.
    pub(in crate::workspace) fn keep_prompt_queued_behind_question(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.take_blocked_prompt(pane).is_some() {
            self.send_composer_text(window, cx);
        }
    }

    /// The held prompt, when it is `pane`'s and that pane holds the
    /// composer: the send that follows reads the composer itself.
    fn take_blocked_prompt(&mut self, pane: PaneId) -> Option<BlockedPrompt> {
        if self.active_runtime().focused_pane_id != pane {
            return None;
        }
        self.input_dock
            .question
            .blocked_prompt
            .take_if(|b| b.pane == pane)
    }
}
