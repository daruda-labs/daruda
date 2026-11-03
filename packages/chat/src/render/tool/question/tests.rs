use std::collections::BTreeMap;

use daruda_acp::{
    QuestionDelivery, QuestionField, QuestionFieldKind, QuestionOption, QuestionRequest,
    QuestionValue,
};

use super::record::record_summary;
use crate::strings as s;
use crate::view::questions::{OpenQuestion, QuestionOutcome, QuestionRecord};

fn option(value: &str, label: &str) -> QuestionOption {
    QuestionOption {
        value: value.into(),
        label: label.into(),
        description: None,
    }
}

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

/// Q0 a single select, Q1 a multi-select with its "Other" box.
fn two_questions() -> QuestionRequest {
    QuestionRequest {
        message: "?".into(),
        tool_call_id: Some("tool".into()),
        delivery: QuestionDelivery::Blocking,
        fields: vec![
            field(
                "question_0",
                QuestionFieldKind::SingleSelect(vec![option("c", "Clippy"), option("t", "Tests")]),
                None,
            ),
            field(
                "question_1",
                QuestionFieldKind::MultiSelect(vec![option("l", "링크"), option("s", "맞춤법")]),
                None,
            ),
            field(
                "question_1_custom",
                QuestionFieldKind::Text,
                Some("question_1"),
            ),
        ],
    }
}

fn record(outcome: QuestionOutcome) -> QuestionRecord {
    QuestionRecord {
        request: two_questions(),
        outcome,
    }
}

/// Composed from the generated strings, so the test holds under either
/// locale; under Korean the answered line reads
/// "질문 2개 답함 · Clippy · 링크 외 1".
#[test]
fn record_summary_reads_the_outcome() {
    let sep = s::agent_chat::question_separator();
    let answered = record(QuestionOutcome::Answered(BTreeMap::from([
        ("question_0".to_owned(), QuestionValue::Text("c".into())),
        (
            "question_1".to_owned(),
            QuestionValue::Choices(vec!["l".into()]),
        ),
        (
            "question_1_custom".to_owned(),
            QuestionValue::Text("README 앵커".into()),
        ),
    ])));
    assert_eq!(
        record_summary(&answered),
        s::agent_chat::question_record_more(
            format!(
                "{}{sep}Clippy{sep}링크",
                s::agent_chat::question_record_answered(2)
            ),
            1,
        )
    );
    assert_eq!(
        record_summary(&record(QuestionOutcome::Skipped)),
        s::agent_chat::question_record_skipped(2)
    );
    assert_eq!(
        record_summary(&record(QuestionOutcome::Expired { after_secs: 300 })),
        s::agent_chat::question_record_ended(
            2,
            s::agent_chat::question_record_expired(s::agent_chat::question_timeout_minutes(5)),
        )
    );
    assert_eq!(
        record_summary(&record(QuestionOutcome::Withdrawn)),
        s::agent_chat::question_record_ended(2, s::agent_chat::question_record_withdrawn())
    );
}

/// An answer that names no more labels than fit leaves the count off.
#[test]
fn record_summary_counts_only_answered_questions() {
    let one = record(QuestionOutcome::Answered(BTreeMap::from([(
        "question_0".to_owned(),
        QuestionValue::Text("t".into()),
    )])));
    assert_eq!(
        record_summary(&one),
        format!(
            "{}{}Tests",
            s::agent_chat::question_record_answered(1),
            s::agent_chat::question_separator(),
        )
    );
}

fn open(request: QuestionRequest) -> OpenQuestion {
    OpenQuestion {
        id: 0,
        draft: crate::view::questions::QuestionDraft::seeded(&request),
        request,
        focus: Default::default(),
        message_expanded: false,
        composer: crate::view::questions::ComposerBinding::Bound {
            draft_became_answer: false,
        },
        requested_field: None,
        told_to_phone: false,
        asked_at: std::time::Instant::now(),
        deadline: None,
    }
}

/// A lone select sends on its pick, so it shows no send button until text
/// committed beside it means a pick no longer sends by itself.
#[test]
fn a_lone_select_offers_send_once_other_text_is_committed() {
    let mut request = two_questions();
    request.fields.remove(0);
    request.fields[0].kind = QuestionFieldKind::SingleSelect(vec![option("l", "링크")]);
    let mut lone = open(request);
    assert!(!lone.offers_send());
    lone.draft.commit_text("question_1_custom", "README");
    assert!(lone.offers_send());
    assert!(open(two_questions()).offers_send());
}
