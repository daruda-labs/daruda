//! The questions the agent-chat question scenarios put in front of the
//! person, and the transcripts whose tool rows carry them. A question's card
//! is its tool row's body, so every request here names a call that exists in
//! the transcript it is seeded beside.

use daruda_acp::{
    ChatItem, MessagePhase, QuestionDelivery, QuestionField, QuestionFieldKind, QuestionOption,
    QuestionRequest, ToolCallItem, ToolKindView, ToolStatusView,
};

use super::{assistant, working_transcript};

/// The call [`shot_question`] belongs to.
pub const QUESTION_TOOL_ID: &str = "shot-question";
/// The call [`shot_question_form`] belongs to.
pub const QUESTION_FORM_TOOL_ID: &str = "shot-question-form";
/// The calls of [`settled_questions_transcript`] that end skipped and
/// cancelled; its answered one is [`QUESTION_FORM_TOOL_ID`].
pub const QUESTION_SKIPPED_TOOL_ID: &str = "shot-question-skipped";
pub const QUESTION_CANCELLED_TOOL_ID: &str = "shot-question-cancelled";
/// A prompt typed while [`shot_question_form`] waits released.
pub const QUESTION_BLOCKED_PROMPT: &str = "Skip the checks and open the PR as a draft";

/// The `AskUserQuestion` call a question rides on, still waiting.
pub fn shot_question_tool(id: &str) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_string(),
        title: "Asking for your input".to_string(),
        kind: ToolKindView::Other,
        tool_name: Some("AskUserQuestion".to_string()),
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: None,
        exit: None,
    })
}

fn settled_question_tool(id: &str, status: ToolStatusView) -> ChatItem {
    let mut item = shot_question_tool(id);
    if let ChatItem::ToolCall(call) = &mut item {
        call.status = status;
    }
    item
}

/// [`working_transcript`] with the agent's question call after its preamble:
/// the turn stays open while the question waits.
pub fn pending_question_transcript(tool_id: &str) -> Vec<ChatItem> {
    let mut items = working_transcript();
    items.push(shot_question_tool(tool_id));
    items
}

/// One turn holding three settled questions: the form answered, the single
/// select skipped, and a last one a Stop cancelled before any answer.
pub fn settled_questions_transcript() -> Vec<ChatItem> {
    vec![
        ChatItem::UserText("wire up the session cache and the merge checks".to_string()),
        assistant(
            "Two decisions are yours before I touch CI.",
            MessagePhase::Commentary,
        ),
        settled_question_tool(QUESTION_FORM_TOOL_ID, ToolStatusView::Completed),
        assistant(
            "Unit tests and Clippy it is. Now the cache backend.",
            MessagePhase::Commentary,
        ),
        settled_question_tool(QUESTION_SKIPPED_TOOL_ID, ToolStatusView::Completed),
        assistant(
            "Keeping the in-memory default. One more before the migration.",
            MessagePhase::Commentary,
        ),
        settled_question_tool(QUESTION_CANCELLED_TOOL_ID, ToolStatusView::Cancelled),
        ChatItem::Interrupted,
    ]
}

/// The question `AskUserQuestion` asks for one choice: options with their
/// descriptions, and the marked "Other" box beside them.
pub fn shot_question() -> QuestionRequest {
    let option = |value: &str, description: &str| QuestionOption {
        value: value.into(),
        label: value.into(),
        description: Some(description.into()),
    };
    QuestionRequest {
        message: "Which storage should the session cache use?".into(),
        tool_call_id: Some(QUESTION_TOOL_ID.into()),
        delivery: QuestionDelivery::Blocking,
        fields: vec![
            QuestionField {
                key: "question_0".into(),
                title: Some("Cache".into()),
                description: None,
                required: false,
                kind: QuestionFieldKind::SingleSelect(vec![
                    option("Redis", "Shared across instances"),
                    option("In-memory", "Fastest; lost on restart"),
                    option("SQLite", "Survives restarts on one host"),
                ]),
                companion_of: None,
                default: None,
            },
            QuestionField {
                key: "question_0_custom".into(),
                title: Some("Other".into()),
                description: None,
                required: false,
                kind: QuestionFieldKind::Text,
                companion_of: Some("question_0".into()),
                default: None,
            },
        ],
    }
}

/// A form of several fields, which waits for Send: a required multi-select
/// and a yes/no.
pub fn shot_question_form() -> QuestionRequest {
    let option = |value: &str| QuestionOption {
        value: value.into(),
        label: value.into(),
        description: None,
    };
    QuestionRequest {
        message: "Please answer the following questions.".into(),
        tool_call_id: Some(QUESTION_FORM_TOOL_ID.into()),
        delivery: QuestionDelivery::Blocking,
        fields: vec![
            QuestionField {
                key: "question_0".into(),
                title: Some("Which checks should run before merge?".into()),
                description: None,
                required: true,
                kind: QuestionFieldKind::MultiSelect(vec![
                    option("Unit tests"),
                    option("Clippy"),
                    option("Screenshot tests"),
                ]),
                companion_of: None,
                default: None,
            },
            QuestionField {
                key: "question_0_custom".into(),
                title: Some("Other".into()),
                description: None,
                required: false,
                kind: QuestionFieldKind::Text,
                companion_of: Some("question_0".into()),
                default: None,
            },
            QuestionField {
                key: "question_1".into(),
                title: Some("Open a draft pull request?".into()),
                description: None,
                required: false,
                kind: QuestionFieldKind::Boolean,
                companion_of: None,
                default: None,
            },
        ],
    }
}

/// The three questions [`settled_questions_transcript`] asks, in its order:
/// the form, the cache select, and the migration select a Stop cut off.
pub fn settled_questions() -> [QuestionRequest; 3] {
    let mut skipped = shot_question();
    skipped.tool_call_id = Some(QUESTION_SKIPPED_TOOL_ID.into());
    let mut cancelled = shot_question();
    cancelled.tool_call_id = Some(QUESTION_CANCELLED_TOOL_ID.into());
    cancelled.message = "Run the migration now?".into();
    if let Some(field) = cancelled.fields.first_mut() {
        field.title = Some("Migration".into());
        field.kind = QuestionFieldKind::SingleSelect(vec![
            QuestionOption {
                value: "Now".into(),
                label: "Now".into(),
                description: Some("Against staging, then production".into()),
            },
            QuestionOption {
                value: "Later".into(),
                label: "Later".into(),
                description: Some("Leave it for the release window".into()),
            },
        ]);
    }
    [shot_question_form(), skipped, cancelled]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_ids(items: &[ChatItem]) -> Vec<&str> {
        items
            .iter()
            .filter_map(|item| match item {
                ChatItem::ToolCall(call) => Some(call.id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// A question whose call is missing from its transcript falls back to the
    /// panel outside the list, which is not what these scenarios capture.
    #[test]
    fn every_seeded_question_names_a_call_in_its_transcript() {
        let pending = pending_question_transcript(QUESTION_TOOL_ID);
        assert!(tool_ids(&pending).contains(&QUESTION_TOOL_ID));
        let form = pending_question_transcript(QUESTION_FORM_TOOL_ID);
        assert_eq!(
            shot_question_form().tool_call_id.as_deref(),
            tool_ids(&form).last().copied()
        );
        let settled = settled_questions_transcript();
        let ids = tool_ids(&settled);
        for question in settled_questions() {
            assert!(ids.contains(&question.tool_call_id.as_deref().unwrap()));
        }
    }
}
