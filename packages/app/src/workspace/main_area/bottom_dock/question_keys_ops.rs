//! The composer's keys while it answers the focused pane's question: digits
//! and Space over an empty composer, Tab and Shift+Tab, and the arrow keys
//! the history hook hands over. The empty Enter goes through
//! `route_composer_submit_to_question` with the rest of the composer's Enter.

use gpui::{Context, Window};

use crate::ui::HistoryDir;
use crate::workspace::Workspace;

/// A key bound in the composer's answer context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum QuestionKey {
    /// Choose the card's row `n`, 1-based.
    Pick(usize),
    Toggle,
    NextField,
    PrevField,
}

impl Workspace {
    /// A question key from the composer's answer context. That context was
    /// set at the last paint, so the state is checked again here; a key the
    /// question does not take propagates, to be typed or to indent.
    pub(in crate::workspace) fn question_key(
        &mut self,
        key: QuestionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.take_question_key(key, window, cx) {
            cx.propagate();
        }
    }

    fn take_question_key(
        &mut self,
        key: QuestionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((pane, question)) = self.focused_question(cx).filter(|(_, q)| q.bound) else {
            return false;
        };
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return false;
        };
        let id = question.question_id;
        // An empty value also means no IME composition: marked text is part
        // of the value until it is committed or cancelled.
        let empty = self.input_dock.input.read(cx).value().is_empty();
        let forward = match key {
            QuestionKey::Pick(n) => {
                return empty && view.update(cx, |v, cx| v.choose_question_number(id, n, cx));
            }
            QuestionKey::Toggle => {
                return empty && view.update(cx, |v, cx| v.toggle_question_row(id, cx));
            }
            QuestionKey::NextField => true,
            QuestionKey::PrevField => false,
        };
        let text = self.input_dock.input.read(cx).value().to_string();
        let typed = !text.trim().is_empty();
        if typed && self.question_swap_pending(pane, &question) {
            self.refuse_question_text(pane, &question, cx);
            return true;
        }
        let moved = view.update(cx, |v, cx| v.move_question_field(id, forward, text, cx));
        if !moved && typed && !at_field_edge(question.position, forward) {
            self.refuse_question_text(pane, &question, cx);
        }
        // Back from the send button to the last field, whose answer the
        // composer gave up on the way there.
        if moved && question.send_highlighted {
            self.load_question_field_text(pane, id, question.field, window, cx);
        }
        // The field swap loads the new field's committed text into the
        // composer, as a card pick that moves focus does.
        self.sync_question_composer(window, cx);
        true
    }

    /// Up or down the history hook gave the question. The hook read the
    /// last sync; a composer that has text by now gets its history step
    /// instead, when there is one to take.
    pub(in crate::workspace) fn move_question_highlight(
        &mut self,
        dir: HistoryDir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let empty = self.input_dock.input.read(cx).value().is_empty();
        let bound = self.focused_question(cx).filter(|(_, q)| q.bound);
        match bound.and_then(|(pane, q)| Some((self.agent_chat_view(pane)?.clone(), q))) {
            Some((view, question)) if empty => view.update(cx, |v, cx| {
                v.move_question_row(question.question_id, dir == HistoryDir::Down, cx)
            }),
            _ if self.history_navigate_possible(dir, cx) => {
                self.do_history_navigate(dir, window, cx)
            }
            // WORKAROUND: the hook already consumed the key on a stale empty
            // mark (IME marked text emits no `Change`), and replaying cursor
            // movement would re-enter the same hook - the key is dropped.
            _ => {}
        }
    }
}

/// Whether focus at `(number, count)` has no field to move to that way, so
/// a move that did not happen refused nothing.
fn at_field_edge((number, count): (usize, usize), forward: bool) -> bool {
    if forward {
        number >= count
    } else {
        number <= 1
    }
}

#[cfg(test)]
mod tests {
    use super::at_field_edge;

    #[test]
    fn only_the_first_and_last_fields_are_edges() {
        assert!(at_field_edge((1, 3), false));
        assert!(!at_field_edge((1, 3), true));
        assert!(!at_field_edge((2, 3), false));
        assert!(at_field_edge((3, 3), true));
        assert!(at_field_edge((1, 1), true) && at_field_edge((1, 1), false));
    }
}
