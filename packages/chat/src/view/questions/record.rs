//! A question that has been settled, kept for the summary bar.

use std::collections::BTreeMap;

use daruda_acp::{QuestionFieldKind, QuestionRequest, QuestionValue};

use super::draft::{companion_key, primary_fields};
use crate::view::QuestionEnd;

/// A settled question and how it ended.
#[derive(Debug, Clone)]
pub(crate) struct QuestionRecord {
    pub(crate) request: QuestionRequest,
    pub(crate) outcome: QuestionOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum QuestionOutcome {
    Answered(BTreeMap<String, QuestionValue>),
    Skipped,
    /// The person sent a new prompt instead of answering.
    SkippedForPrompt,
    Cancelled,
    /// The agent took the question back.
    Withdrawn,
    /// Nobody touched it before the opt-in timeout ran out.
    Expired {
        after_secs: u32,
    },
}

impl QuestionOutcome {
    /// The host's word for how the question ended.
    pub(crate) fn end(&self) -> QuestionEnd {
        match self {
            Self::Answered(_) => QuestionEnd::Answered,
            Self::Skipped => QuestionEnd::Skipped,
            Self::SkippedForPrompt => QuestionEnd::SkippedForPrompt,
            Self::Cancelled => QuestionEnd::Cancelled,
            Self::Withdrawn => QuestionEnd::Withdrawn,
            Self::Expired { .. } => QuestionEnd::Expired,
        }
    }
}

impl QuestionRecord {
    /// How many questions the form asked: its primary fields.
    pub(crate) fn asked(&self) -> usize {
        primary_fields(&self.request).count()
    }

    /// How many questions the answer covers, by a value or its "Other" text.
    pub(crate) fn answered(&self) -> usize {
        let QuestionOutcome::Answered(values) = &self.outcome else {
            return 0;
        };
        primary_fields(&self.request)
            .filter(|field| {
                values.contains_key(&field.key)
                    || companion_key(&self.request, &field.key)
                        .is_some_and(|key| values.contains_key(key))
            })
            .count()
    }

    /// Labels the summary bar shows after the count: picked labels and
    /// typed answers in field order, at most `limit`, plus how many more.
    /// Booleans are left out: a bare yes/no reads as noise without its
    /// question beside it.
    pub(crate) fn answer_preview(&self, limit: usize) -> (Vec<String>, usize) {
        let QuestionOutcome::Answered(values) = &self.outcome else {
            return (Vec::new(), 0);
        };
        let mut entries = Vec::new();
        for field in primary_fields(&self.request) {
            match (&field.kind, values.get(&field.key)) {
                (
                    QuestionFieldKind::SingleSelect(options)
                    | QuestionFieldKind::MultiSelect(options),
                    Some(value),
                ) => {
                    let picked: &[String] = match value {
                        QuestionValue::Choices(values) => values,
                        QuestionValue::Text(value) => std::slice::from_ref(value),
                        QuestionValue::Bool(_) => &[],
                    };
                    for pick in picked {
                        let label = options.iter().find(|o| o.value == *pick).map(|o| &o.label);
                        entries.push(label.unwrap_or(pick).clone());
                    }
                }
                (QuestionFieldKind::Text, Some(QuestionValue::Text(text))) => {
                    entries.push(text.clone());
                }
                _ => {}
            }
            if let Some(QuestionValue::Text(text)) =
                companion_key(&self.request, &field.key).and_then(|key| values.get(key))
            {
                entries.push(text.clone());
            }
        }
        let rest = entries.len().saturating_sub(limit);
        entries.truncate(limit);
        (entries, rest)
    }
}

#[cfg(test)]
mod tests {
    use daruda_acp::QuestionOption;

    use super::*;
    use crate::view::questions::draft::tests::{field, request};

    fn labelled(value: &str, label: &str) -> QuestionOption {
        QuestionOption {
            value: value.into(),
            label: label.into(),
            description: None,
        }
    }

    #[test]
    fn answer_preview_lists_picks_then_text_and_counts_the_rest() {
        let request = request(vec![
            field(
                "question_0",
                QuestionFieldKind::SingleSelect(vec![labelled("c", "Clippy")]),
                None,
            ),
            field(
                "question_1",
                QuestionFieldKind::MultiSelect(vec![labelled("l", "링크")]),
                None,
            ),
            field(
                "question_1_custom",
                QuestionFieldKind::Text,
                Some("question_1"),
            ),
        ]);
        let answers = BTreeMap::from([
            ("question_0".to_owned(), QuestionValue::Text("c".into())),
            (
                "question_1".to_owned(),
                QuestionValue::Choices(vec!["l".into()]),
            ),
            (
                "question_1_custom".to_owned(),
                QuestionValue::Text("README 앵커".into()),
            ),
        ]);
        let record = QuestionRecord {
            request,
            outcome: QuestionOutcome::Answered(answers),
        };
        assert_eq!(
            record.answer_preview(2),
            (vec!["Clippy".to_owned(), "링크".to_owned()], 1)
        );
        assert_eq!(record.answer_preview(9).1, 0);
    }

    #[test]
    fn unanswered_outcomes_preview_nothing() {
        let record = QuestionRecord {
            request: request(vec![]),
            outcome: QuestionOutcome::Skipped,
        };
        assert_eq!(record.answer_preview(3), (Vec::new(), 0));
    }
}
