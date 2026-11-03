//! An agent's question on the phone: one card per field, sent by the same
//! level-triggered sweep that re-offers permissions, and the next field's
//! card the moment the phone answers the one before.

use gpui::Context;

use crate::remote_channel::bridge::{
    PingPrompt, QuestionButton, QuestionPrompt, QuestionPromptRef,
};
use crate::surface::strings as s;
use crate::telegram::bridge::TelegramTail;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::PhoneQuestion;

/// The card's body: who asks and where in the form, the agent's message,
/// the field's own question when it has one, how to answer in text, and
/// the time the answer timeout left when the card was sent (the card is not
/// edited as it runs down). Plain: the agent's words go verbatim, never
/// parsed as markdown.
fn question_wait_text(question: &PhoneQuestion) -> String {
    let lead = match question.position {
        (n, total) if total > 1 => {
            s::notification::telegram_question_asks_position(&question.asked_by, n, total)
        }
        _ => s::notification::telegram_question_asks(&question.asked_by),
    };
    let mut lines = vec![lead, question.message.clone()];
    if question.header != question.message {
        lines.push(question.header.clone());
    }
    if question.takes_text {
        lines.push(s::notification::telegram_question_reply_hint());
    }
    if let Some(left) = question.time_left {
        lines.push(s::notification::telegram_question_time_left(left.clock()));
    }
    lines.join("\n")
}

/// One choice per row, as the desktop card lists them (a yes/no is two);
/// a multi-select marks each choice and ends with Send. An optional field
/// can be left blank alone ("Skip this one"); "Skip all" declines the form.
fn question_rows(question: &PhoneQuestion) -> Vec<Vec<(String, QuestionButton)>> {
    let choices = question.options.iter().enumerate().map(|(ix, label)| {
        let label = match (question.multi, question.picked.contains(&ix)) {
            (false, _) => label.clone(),
            (true, true) => s::notification::telegram_question_picked(label),
            (true, false) => s::notification::telegram_question_unpicked(label),
        };
        (label, QuestionButton::Option(ix))
    });
    let mut rows: Vec<Vec<_>> = choices.map(|choice| vec![choice]).collect();
    let mut last = Vec::new();
    if question.multi {
        last.push((
            s::notification::telegram_question_send(),
            QuestionButton::Send,
        ));
    }
    if question.optional {
        last.push((
            s::notification::telegram_question_skip_one(),
            QuestionButton::SkipField,
        ));
    }
    last.push((
        s::notification::telegram_question_skip(),
        QuestionButton::Skip,
    ));
    rows.push(last);
    rows
}

impl Workspace {
    /// The header of a question card: the worktree it runs in, then the
    /// agent, so the phone can tell which of several chats is asking.
    fn telegram_question_header(&self, pane_id: PaneId, cx: &Context<Self>) -> String {
        let place = self.lane_ref_for_pane(pane_id).map(|lane| {
            s::control::lane_path(
                self.control_project_name(lane.project),
                self.control_lane_name(lane),
            )
        });
        self.telegram_header_at(pane_id, place, cx)
    }

    fn question_ping_parts(
        &self,
        pane_id: PaneId,
        question: &PhoneQuestion,
        cx: &Context<Self>,
    ) -> (String, TelegramTail, Option<PingPrompt>) {
        let session = self
            .agent_chat_view(pane_id)
            .map(|view| view.read(cx).permission_session())
            .unwrap_or_default();
        let prompt = QuestionPrompt {
            question: QuestionPromptRef {
                session,
                question_id: question.id,
                field: question.field,
            },
            step: question.step,
            rows: question_rows(question),
        };
        (
            self.telegram_question_header(pane_id, cx),
            TelegramTail::Plain(question_wait_text(question)),
            Some(PingPrompt::Question(prompt)),
        )
    }

    /// Offer the phone a question it has not been sent, under the same
    /// presence rule as a permission wait: a phone-started turn always hears
    /// it, otherwise only an absent user does. Marked told once it went out,
    /// so the sweep does not repeat it.
    pub(super) fn relay_question_wait_to_telegram(
        &mut self,
        pane_id: PaneId,
        question: &PhoneQuestion,
        cx: &mut Context<Self>,
    ) {
        let (header, tail, prompt) = self.question_ping_parts(pane_id, question, cx);
        if self.relay_when_presence_allows(pane_id, header, tail, prompt, cx)
            && let Some(view) = self.agent_chat_view(pane_id).cloned()
        {
            view.update(cx, |v, _| v.mark_question_told_to_phone(question.id));
        }
    }

    /// Send the card the phone's last answer asked for. Solicited, so it
    /// goes whatever presence says.
    pub(super) fn relay_next_question_field(
        &self,
        pane_id: PaneId,
        question: &PhoneQuestion,
        cx: &Context<Self>,
    ) {
        let (header, tail, prompt) = self.question_ping_parts(pane_id, question, cx);
        self.relay_to_telegram(pane_id, header, tail, prompt, cx);
    }

    /// Offer each open question of `pane_id` the phone has not been sent.
    pub(in crate::workspace) fn relay_untold_questions(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        if !view.read(cx).has_pending_question() {
            return;
        }
        let untold = view.update(cx, |v, _| v.take_questions_untold_to_phone());
        for question in untold {
            self.relay_question_wait_to_telegram(pane_id, &question, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(options: &[&str], multi: bool, takes_text: bool) -> PhoneQuestion {
        PhoneQuestion {
            id: 1,
            field: 0,
            step: 1,
            header: "Which?".into(),
            message: "Before I ship".into(),
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            picked: vec![1],
            multi,
            takes_text,
            optional: false,
            position: (1, 1),
            asked_by: "Claude".into(),
            time_left: None,
        }
    }

    #[test]
    fn a_single_select_lists_its_choices_then_skip() {
        let rows = question_rows(&card(&["a", "b"], false, false));
        let buttons: Vec<Vec<QuestionButton>> = rows
            .iter()
            .map(|row| row.iter().map(|(_, b)| *b).collect())
            .collect();
        assert_eq!(
            buttons,
            [
                vec![QuestionButton::Option(0)],
                vec![QuestionButton::Option(1)],
                vec![QuestionButton::Skip],
            ]
        );
        assert_eq!(rows[1][0].0, "b", "a single select is not marked");
    }

    #[test]
    fn a_multi_select_marks_its_picks_and_ends_with_send() {
        let rows = question_rows(&card(&["a", "b"], true, false));
        assert_eq!(
            rows[0][0].0,
            s::notification::telegram_question_unpicked("a")
        );
        assert_eq!(rows[1][0].0, s::notification::telegram_question_picked("b"));
        assert_eq!(
            rows[2].iter().map(|(_, b)| *b).collect::<Vec<_>>(),
            [QuestionButton::Send, QuestionButton::Skip]
        );
    }

    #[test]
    fn the_body_names_the_asker_and_says_how_to_answer_in_text() {
        let text = question_wait_text(&card(&[], false, true));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                s::notification::telegram_question_asks("Claude").as_str(),
                "Before I ship",
                "Which?",
                s::notification::telegram_question_reply_hint().as_str(),
            ]
        );
    }

    /// Q19: the countdown keeps running while the phone holds the card, so
    /// the card says how long was left when it was sent.
    #[test]
    fn the_body_ends_with_the_time_left_while_the_timeout_runs() {
        let now = std::time::Instant::now();
        let left = daruda_chat::view::QuestionCountdown::until(
            now + std::time::Duration::from_secs(252),
            now,
        );
        let text = question_wait_text(&PhoneQuestion {
            time_left: Some(left),
            ..card(&["a"], false, false)
        });
        assert_eq!(
            text.lines().last(),
            Some(s::notification::telegram_question_time_left("4:12").as_str())
        );
    }
}
