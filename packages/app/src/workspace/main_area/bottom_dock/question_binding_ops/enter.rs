//! The composer's Enter while a question waits: answer it, or hold a prompt
//! back behind it.

use daruda_chat::view::QuestionComposer;
use gpui::{App, Context, Window};

use super::{BlockedPrompt, BoundField};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// The composer's Enter, before it becomes a prompt. While bound the
    /// text answers the question (whatever it starts with, `/` included).
    /// Released, Enter holds a prompt back with a warning, since it
    /// would wait behind the question; only the strip's buttons send it on.
    /// `true` when the Enter was consumed here. `text` is the composer's
    /// value without trailing line breaks.
    pub(in crate::workspace) fn route_composer_submit_to_question(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((pane, question)) = self.focused_question(cx) else {
            return false;
        };
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return false;
        };
        let typed = text.trim();
        if typed.is_empty() && question.bound {
            // An empty Enter chooses the card's highlighted row; the parked
            // queue's resume gesture waits until the question is done.
            view.update(cx, |v, cx| v.choose_question_row(question.question_id, cx));
            self.sync_question_composer(window, cx);
            return true;
        }
        // Released, an empty Enter is a prompt only when it carries
        // attachments; otherwise it is the empty-Enter gesture.
        if typed.is_empty() && view.read(cx).draft_attachments().is_empty() {
            return false;
        }
        if question.bound {
            let accepted = !self.question_swap_pending(pane, &question)
                && view.update(cx, |v, cx| {
                    v.answer_question_text(question.question_id, typed.to_owned(), cx)
                });
            if accepted {
                self.input_dock
                    .input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.sync_question_composer(window, cx);
            } else {
                // The text stays; the strip says why Enter took nothing.
                self.drop_submit_line_break(text, window, cx);
                self.refuse_question_text(pane, &question, cx);
            }
            return true;
        }
        // Armed once; later Enters, key repeat included, change nothing.
        let marks = &mut self.input_dock.question;
        if marks.blocked_prompt.is_none() {
            marks.blocked_prompt = Some(BlockedPrompt {
                pane,
                question_id: question.question_id,
                text: typed.to_owned(),
            });
            cx.notify();
        }
        self.drop_submit_line_break(text, window, cx);
        true
    }

    /// A field swap whose text found no home is still pending: the composer
    /// holds text typed for the field `pane` was last bound to, which must
    /// not answer or be committed to the field focus moved to.
    pub(in crate::workspace::main_area::bottom_dock) fn question_swap_pending(
        &self,
        pane: PaneId,
        question: &QuestionComposer,
    ) -> bool {
        self.input_dock
            .question
            .bound
            .get(&pane)
            .is_some_and(|b| *b != BoundField::of(question))
    }

    /// The bound field did not take the composer's text; the text stays and
    /// the strip says why.
    pub(in crate::workspace::main_area::bottom_dock) fn refuse_question_text(
        &mut self,
        pane: PaneId,
        question: &QuestionComposer,
        cx: &mut Context<Self>,
    ) {
        self.input_dock.question.text_refused = Some((pane, BoundField::of(question)));
        cx.notify();
    }

    /// Whether the arrow keys belong to the question: bound, over a composer
    /// empty at the last sync. Read from the composer's own hook.
    pub(in crate::workspace) fn question_arrows_take(&self, cx: &App) -> bool {
        !self.input_dock.question.composer_has_text && self.focused_question_bound(cx)
    }

    /// An Enter that sent nothing leaves the composer as it was: a submit
    /// that inserted a line break first must not pile them up.
    fn drop_submit_line_break(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_dock.input.read(cx).value() != text {
            self.input_dock.input.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.move_cursor_to_end(cx);
            });
        }
    }
}
