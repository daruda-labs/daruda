//! Keeping the composer, each pane's draft hint, and the strip's marks in
//! line with the question a pane's composer answers.

use gpui::{App, Context, Window};

use super::{AnsweredQuestion, BoundField};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Bring the composer in line with the question: the owner pane's
    /// empty-draft hint, the bound field's text, stale strip marks, and the
    /// placeholder. Runs on focus change, `QuestionComposerChanged`, and
    /// every composer `Change`; never from inside an `InputState` hook.
    pub(in crate::workspace) fn sync_question_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.active_runtime().focused_pane_id;
        let has_text = !self.input_dock.input.read(cx).value().is_empty();
        let gained_text = has_text && !self.input_dock.question.composer_has_text;
        self.input_dock.question.composer_has_text = has_text;
        // Spec 7.1: typing ends keyboard choosing. Before the field follows,
        // so text typed on the send button stays with the last field.
        if gained_text
            && let Some((pane, question)) = self.focused_question(cx).filter(|(_, q)| q.bound)
            && let Some(view) = self.agent_chat_view(pane).cloned()
        {
            view.update(cx, |v, cx| {
                v.clear_question_highlight(question.question_id, cx)
            });
        }
        self.follow_bound_field(focused, window, cx);
        self.sync_composer_hints(cx);
        self.note_answer_text(cx);
        if self.drop_stale_question_marks(cx) {
            cx.notify();
        }
        let placeholder = self.focused_question_placeholder(cx);
        if placeholder != self.input_dock.question.placeholder {
            self.input_dock.question.placeholder = placeholder;
            self.apply_input_placeholder(window, cx);
        }
    }

    /// `QuestionComposerChanged` from `pane`, focused or not.
    pub(in crate::workspace) fn question_composer_changed(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.follow_bound_field(pane, window, cx);
        self.sync_question_composer(window, cx);
    }

    /// Refresh the empty-draft hint of the owner's visible text and of
    /// every saved draft, which a field swap may rewrite.
    fn sync_composer_hints(&mut self, cx: &mut Context<Self>) {
        let panes: Vec<PaneId> = self
            .input_dock
            .owner
            .into_iter()
            .chain(self.input_dock.drafts.keys().copied())
            .collect();
        for pane in panes {
            self.sync_composer_hint(pane, cx);
        }
    }

    /// The one update site of `pane`'s empty-draft hint.
    fn sync_composer_hint(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            let empty = self.pane_composer_text(pane, cx).trim().is_empty();
            view.update(cx, |v, cx| v.set_composer_hint_empty(empty, cx));
        }
    }

    /// Remember that the owner's text was typed while bound: it stays that
    /// question's answer after a release. An emptied composer forgets.
    fn note_answer_text(&mut self, cx: &App) {
        let Some(owner) = self.input_dock.owner else {
            return;
        };
        if self.input_dock.input.read(cx).value().trim().is_empty() {
            self.input_dock.question.answer_text.remove(&owner);
        } else if let Some((pane, q)) = self.focused_question(cx)
            && pane == owner
            && q.bound
            && let Some(view) = self.agent_chat_view(owner)
        {
            let answered = AnsweredQuestion {
                session: view.read(cx).permission_session(),
                question_id: q.question_id,
            };
            self.input_dock.question.answer_text.insert(owner, answered);
        }
    }

    /// T4: when `pane`'s bound field moved since the last sync, the text it
    /// holds was typed for the old field. Commit it there, then load the new
    /// field's committed answer, so text never follows focus.
    fn follow_bound_field(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            self.input_dock.question.bound.remove(&pane);
            return;
        };
        let now = view
            .read(cx)
            .question_composer()
            .filter(|q| q.bound)
            .map(|q| BoundField::of(&q));
        let before = self.input_dock.question.bound.get(&pane).copied();
        let (Some(before), Some(now)) = (before, now) else {
            match now {
                Some(now) => self.input_dock.question.bound.insert(pane, now),
                None => self.input_dock.question.bound.remove(&pane),
            };
            return;
        };
        // Leaving the send button changes no field: Shift+Tab loads the field's
        // text itself, and text typed there is already the field's.
        if before.question_id != now.question_id || before == now || before.send {
            self.input_dock.question.bound.insert(pane, now);
            return;
        }
        let typed = self.pane_composer_text(pane, cx).trim().to_owned();
        // A field that takes no text refuses it. The text then has no home,
        // so the record stays on the old field: Enter cannot make it the new
        // field's answer, and the swap completes once the composer empties.
        if !typed.is_empty()
            && !view.update(cx, |v, cx| {
                v.commit_question_text(before.question_id, before.field, typed, cx)
            })
        {
            return;
        }
        self.input_dock.question.bound.insert(pane, now);
        if now.send {
            self.set_pane_composer_text(pane, "", window, cx);
        } else {
            self.load_question_field_text(pane, now.question_id, now.field, window, cx);
        }
    }

    /// Put field `field`'s committed answer into `pane`'s composer, so an
    /// earlier answer can be edited.
    pub(in crate::workspace::main_area::bottom_dock) fn load_question_field_text(
        &mut self,
        pane: PaneId,
        question_id: u64,
        field: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        let text = view
            .read(cx)
            .question_field_text(question_id, field)
            .unwrap_or_default();
        self.set_pane_composer_text(pane, &text, window, cx);
    }

    /// The composer text `pane` holds: the visible one when it owns the
    /// composer, its saved draft otherwise.
    fn pane_composer_text(&self, pane: PaneId, cx: &App) -> String {
        if self.input_dock.owner == Some(pane) {
            self.input_dock.input.read(cx).value().to_string()
        } else {
            self.input_dock
                .drafts
                .get(&pane)
                .cloned()
                .unwrap_or_default()
        }
    }

    pub(super) fn set_pane_composer_text(
        &mut self,
        pane: PaneId,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_dock.owner == Some(pane) {
            // `set_value` emits `Change` even for the same text.
            if self.input_dock.input.read(cx).value() != text {
                self.input_dock.input.update(cx, |state, cx| {
                    state.set_value(text, window, cx);
                    state.move_cursor_to_end(cx);
                });
            }
        } else {
            if text.is_empty() {
                self.input_dock.drafts.remove(&pane);
            } else {
                self.input_dock.drafts.insert(pane, text.to_owned());
            }
            // A removed draft drops out of `sync_composer_hints`, so the
            // hint follows here or it would keep saying "typed".
            self.sync_composer_hint(pane, cx);
        }
    }

    /// Drop the held-back prompt once the composer no longer shows it for a
    /// released question, the Esc mark once its pane is not released, and
    /// the refusal once its field is not bound or the composer empties.
    fn drop_stale_question_marks(&mut self, cx: &App) -> bool {
        let question = self.focused_question(cx);
        let focused = question
            .as_ref()
            .map(|(pane, q)| (*pane, q.question_id, q.bound));
        let bound_field = question
            .as_ref()
            .filter(|(_, q)| q.bound)
            .map(|(pane, q)| (*pane, BoundField::of(q)));
        let composer = self.input_dock.input.read(cx).value().trim().to_owned();
        let marks = &mut self.input_dock.question;
        let mut changed = false;
        if marks.text_refused.is_some()
            && (marks.text_refused != bound_field || composer.is_empty())
        {
            marks.text_refused = None;
            changed = true;
        }
        if let Some(blocked) = &marks.blocked_prompt
            && (focused != Some((blocked.pane, blocked.question_id, false))
                || composer != blocked.text)
        {
            marks.blocked_prompt = None;
            changed = true;
        }
        if let Some(pane) = marks.esc_released
            && !matches!(focused, Some((p, _, false)) if p == pane)
        {
            marks.esc_released = None;
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {}
