//! Questions the agent asked mid-turn, waiting on the person's answer.
//!
//! The turn stays open while a question waits, so these live beside the
//! permission index rather than in the transcript: the tool row that asked
//! records the exchange once it is answered.

use std::collections::BTreeMap;

use daruda_acp::{QuestionAnswer, QuestionFieldKind, QuestionRequest, QuestionValue};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};

use super::AgentChatView;

/// Open questions, oldest first. Only the first is shown; the rest wait their
/// turn, so answering one never shifts the form under the pointer.
#[derive(Default)]
pub(crate) struct Questions {
    pending: Vec<PendingQuestion>,
}

/// One open question and what the person has filled in so far.
pub(crate) struct PendingQuestion {
    pub(crate) id: u64,
    pub(crate) request: QuestionRequest,
    /// Picked option values per select field, in pick order.
    picks: BTreeMap<String, Vec<String>>,
    flags: BTreeMap<String, bool>,
    /// Text boxes the person opened, by field key. Created on demand, since
    /// an input needs the window a click handler carries.
    texts: BTreeMap<String, Entity<daruda_ui::InputState>>,
    /// Keeps the panel in step with what is typed, and sends on Enter.
    _typing: Vec<Subscription>,
}

impl Questions {
    pub(crate) fn first(&self) -> Option<&PendingQuestion> {
        self.pending.first()
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn ask(&mut self, id: u64, request: QuestionRequest) {
        self.pending.push(PendingQuestion {
            id,
            request,
            picks: BTreeMap::new(),
            flags: BTreeMap::new(),
            texts: BTreeMap::new(),
            _typing: Vec::new(),
        });
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut PendingQuestion> {
        self.pending.iter_mut().find(|q| q.id == id)
    }

    fn take(&mut self, id: u64) -> Option<PendingQuestion> {
        let at = self.pending.iter().position(|q| q.id == id)?;
        Some(self.pending.remove(at))
    }
}

impl PendingQuestion {
    pub(crate) fn is_picked(&self, key: &str, value: &str) -> bool {
        self.picks
            .get(key)
            .is_some_and(|picks| picks.iter().any(|p| p == value))
    }

    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        self.flags.get(key).copied()
    }

    pub(crate) fn text(&self, key: &str) -> Option<&Entity<daruda_ui::InputState>> {
        self.texts.get(key)
    }

    pub(crate) fn has_open_text(&self) -> bool {
        !self.texts.is_empty()
    }

    /// One select and nothing but its own "Other" box: picking an option is
    /// the whole answer, so a pick sends it.
    pub(crate) fn picks_answer(&self) -> bool {
        let mut selects = self
            .request
            .fields
            .iter()
            .filter(|f| matches!(f.kind, QuestionFieldKind::SingleSelect(_)));
        let Some(select) = selects.next() else {
            return false;
        };
        selects.next().is_none()
            && self.request.fields.iter().all(|f| {
                f.key == select.key || f.companion_of.as_deref() == Some(select.key.as_str())
            })
    }

    /// The filled form, or `None` while a required field is unanswered. A
    /// select counts as answered by its own "Other" box.
    pub(crate) fn answer(&self, cx: &App) -> Option<BTreeMap<String, QuestionValue>> {
        let mut values = BTreeMap::new();
        for field in &self.request.fields {
            let value = match &field.kind {
                QuestionFieldKind::SingleSelect(_) => self
                    .picks
                    .get(&field.key)
                    .and_then(|p| p.first())
                    .map(|v| QuestionValue::Text(v.clone())),
                QuestionFieldKind::MultiSelect(_) => self
                    .picks
                    .get(&field.key)
                    .filter(|p| !p.is_empty())
                    .map(|p| QuestionValue::Choices(p.clone())),
                QuestionFieldKind::Boolean => {
                    self.flags.get(&field.key).map(|b| QuestionValue::Bool(*b))
                }
                QuestionFieldKind::Text => self.typed(&field.key, cx).map(QuestionValue::Text),
            };
            match value {
                Some(value) => {
                    values.insert(field.key.clone(), value);
                }
                None if field.required && !self.companion_typed(&field.key, cx) => return None,
                None => {}
            }
        }
        (!values.is_empty()).then_some(values)
    }

    fn typed(&self, key: &str, cx: &App) -> Option<String> {
        let text = self.texts.get(key)?.read(cx).value().trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn companion_typed(&self, key: &str, cx: &App) -> bool {
        self.request
            .fields
            .iter()
            .filter(|f| f.companion_of.as_deref() == Some(key))
            .any(|f| self.typed(&f.key, cx).is_some())
    }
}

impl AgentChatView {
    pub(crate) fn questions(&self) -> &Questions {
        &self.questions
    }

    pub fn has_pending_question(&self) -> bool {
        !self.questions.is_empty()
    }

    pub(super) fn ask_question(&mut self, id: u64, request: QuestionRequest) {
        self.questions.ask(id, request);
    }

    /// Pick `value` in the select `key`. A single select keeps one pick; a
    /// multi-select toggles. A form that is one select sends on the pick.
    pub fn pick_question_option(
        &mut self,
        id: u64,
        key: String,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.get_mut(id) else {
            return;
        };
        let multi = question
            .request
            .fields
            .iter()
            .any(|f| f.key == key && matches!(f.kind, QuestionFieldKind::MultiSelect(_)));
        let picks = question.picks.entry(key).or_default();
        if multi {
            match picks.iter().position(|p| *p == value) {
                Some(at) => {
                    picks.remove(at);
                }
                None => picks.push(value),
            }
        } else {
            *picks = vec![value];
        }
        // In a form that is one select, a pick is the answer: whatever was
        // being typed in its "Other" box is set aside rather than sent beside
        // it as a contradicting second answer.
        if !multi && question.picks_answer() {
            question.texts.clear();
            self.submit_question(id, cx);
            return;
        }
        cx.notify();
    }

    pub fn set_question_flag(&mut self, id: u64, key: String, value: bool, cx: &mut Context<Self>) {
        if let Some(question) = self.questions.get_mut(id) {
            question.flags.insert(key, value);
            cx.notify();
        }
    }

    /// Open the text box for `key` and focus it. What is typed re-renders
    /// the panel, so Send enables as soon as the answer is there, and Enter
    /// sends it.
    pub fn open_question_text(
        &mut self,
        id: u64,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.questions.get_mut(id) else {
            return;
        };
        // Already open: its chip is gone, so only a repeated call gets here,
        // and a second box would be one nothing renders.
        if question.texts.contains_key(&key) {
            return;
        }
        let input = cx.new(|cx| {
            daruda_ui::InputState::new(window, cx)
                .placeholder(crate::strings::agent_chat::question_text_placeholder())
        });
        let typing = cx.subscribe(&input, move |view, _, event, cx| match event {
            daruda_ui::InputEvent::Change => cx.notify(),
            daruda_ui::InputEvent::PressEnter { .. } => view.submit_question(id, cx),
            daruda_ui::InputEvent::Focus | daruda_ui::InputEvent::Blur => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        if let Some(question) = self.questions.get_mut(id) {
            question.texts.insert(key, input);
            question._typing.push(typing);
        }
        cx.notify();
    }

    /// Send the filled form. Does nothing while a required field is empty.
    pub fn submit_question(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(values) = self
            .questions
            .pending
            .iter()
            .find(|q| q.id == id)
            .and_then(|q| q.answer(cx))
        else {
            return;
        };
        self.resolve_question(id, QuestionAnswer::Submit(values), cx);
    }

    /// Answer "skipped": the agent is told the person chose not to answer.
    pub fn skip_question(&mut self, id: u64, cx: &mut Context<Self>) {
        self.resolve_question(id, QuestionAnswer::Decline, cx);
    }

    fn resolve_question(&mut self, id: u64, answer: QuestionAnswer, cx: &mut Context<Self>) {
        if self.questions.take(id).is_none() {
            return;
        }
        if let Some(handle) = self.live_handle() {
            handle.respond_question(id, answer);
        }
        cx.notify();
    }

    /// Cancel every open question: the turn they belonged to is over.
    pub(super) fn cancel_pending_questions(&mut self, cx: &mut Context<Self>) {
        if self.questions.is_empty() {
            return;
        }
        for question in std::mem::take(&mut self.questions.pending) {
            if let Some(handle) = self.any_handle() {
                handle.respond_question(question.id, QuestionAnswer::Cancel);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
