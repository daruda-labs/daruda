//! A question the agent asks in the middle of a turn: an ACP form
//! elicitation (`elicitation/create`), in this crate's own vocabulary.
//!
//! Claude's `AskUserQuestion` arrives this way once the client advertises
//! form elicitation, and the turn stays open until the answer goes back — so
//! a question is a wait inside a running turn, like a permission request,
//! rather than the end of one.

use std::collections::BTreeMap;

use agent_client_protocol::schema::v1::{
    CreateElicitationRequest, CreateElicitationResponse, ElicitationAcceptAction,
    ElicitationAction, ElicitationContentValue, ElicitationMode, ElicitationPropertySchema,
    ElicitationScope, EnumOption, Meta, MultiSelectItems,
};

/// One question as the host renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRequest {
    pub message: String,
    /// The tool call that asked, when the agent named one.
    pub tool_call_id: Option<String>,
    pub fields: Vec<QuestionField>,
    pub delivery: QuestionDelivery,
}

/// How a question holds the agent.
///
/// Only `Blocking` exists today: the turn waits on the answer. This is the
/// slot for a non-blocking question, which codex-acp's
/// `request_user_input_async` would send without parking the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionDelivery {
    Blocking,
}

/// One answer slot of a question, in the agent's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionField {
    /// The key the answer is returned under.
    pub key: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub required: bool,
    pub kind: QuestionFieldKind,
    /// The select field this free-text field supplements ("Other"), when the
    /// agent marked it as one. Rendered under that field rather than alone.
    pub companion_of: Option<String>,
    /// The agent's pre-filled answer, when the schema carried one.
    pub default: Option<QuestionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionFieldKind {
    SingleSelect(Vec<QuestionOption>),
    MultiSelect(Vec<QuestionOption>),
    Text,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    /// What the answer carries when this option is picked.
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// The host's reply to a question.
#[derive(Debug, Clone, PartialEq)]
pub enum QuestionAnswer {
    /// The filled form, keyed by [`QuestionField::key`]. An unanswered
    /// optional field is simply absent.
    Submit(BTreeMap<String, QuestionValue>),
    /// The person chose not to answer; the agent is told it was skipped.
    Decline,
    /// The question went away unanswered (Stop, a closed pane).
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionValue {
    Text(String),
    Choices(Vec<String>),
    Bool(bool),
}

/// Keys of the marker the Claude adapter puts on the free-text companion of a
/// select question: `_meta.jetbrains.air.customAnswer`, and the root
/// `_askUserQuestionCustomAnswer` spelling released AIR versions read.
const CUSTOM_ANSWER_KEY: &str = "customAnswer";
const LEGACY_CUSTOM_ANSWER_KEY: &str = "_askUserQuestionCustomAnswer";

impl QuestionRequest {
    /// The question a form elicitation asks, or `None` when it cannot be
    /// rendered faithfully: a URL elicitation, a mode this build does not
    /// know, or a field type with no control here. Refusing the whole form is
    /// safer than answering part of it.
    pub fn from_protocol(request: &CreateElicitationRequest) -> Option<Self> {
        let ElicitationMode::Form(form) = &request.mode else {
            return None;
        };
        let schema = &form.requested_schema;
        let required = schema.required.clone().unwrap_or_default();
        let fields = schema
            .properties
            .iter()
            .map(|(key, property)| field_of(key, property, required.contains(key)))
            .collect::<Option<Vec<_>>>()?;
        if fields.is_empty() {
            return None;
        }
        let tool_call_id = match request.scope() {
            ElicitationScope::Session(scope) => {
                scope.tool_call_id.as_ref().map(|id| id.0.to_string())
            }
            _ => None,
        };
        Some(Self {
            message: request.message.clone(),
            tool_call_id,
            fields: ordered(fields),
            delivery: QuestionDelivery::Blocking,
        })
    }
}

impl QuestionAnswer {
    pub(crate) fn into_response(self) -> CreateElicitationResponse {
        let action = match self {
            Self::Submit(values) => {
                let content = values
                    .into_iter()
                    .map(|(key, value)| (key, value.into_protocol()))
                    .collect::<BTreeMap<_, _>>();
                ElicitationAction::Accept(ElicitationAcceptAction::new().content(content))
            }
            Self::Decline => ElicitationAction::Decline,
            Self::Cancel => ElicitationAction::Cancel,
        };
        CreateElicitationResponse::new(action)
    }
}

impl QuestionValue {
    fn into_protocol(self) -> ElicitationContentValue {
        match self {
            Self::Text(text) => ElicitationContentValue::String(text),
            Self::Choices(values) => ElicitationContentValue::StringArray(values),
            Self::Bool(value) => ElicitationContentValue::Boolean(value),
        }
    }
}

fn field_of(
    key: &str,
    property: &ElicitationPropertySchema,
    required: bool,
) -> Option<QuestionField> {
    let (title, description, kind, meta, default) = match property {
        ElicitationPropertySchema::String(s) => {
            let kind = match (&s.one_of, &s.enum_values) {
                (Some(options), _) => QuestionFieldKind::SingleSelect(options_of(options)),
                (None, Some(values)) => QuestionFieldKind::SingleSelect(
                    values.iter().map(|v| plain_option(v)).collect(),
                ),
                (None, None) => QuestionFieldKind::Text,
            };
            let default = s
                .default
                .clone()
                .map(QuestionValue::Text)
                .filter(|d| in_options(&kind, d));
            (&s.title, &s.description, kind, s.meta.as_ref(), default)
        }
        ElicitationPropertySchema::Array(a) => {
            let options = match &a.items {
                MultiSelectItems::Titled(items) => options_of(&items.options),
                MultiSelectItems::String(items) => {
                    items.values.iter().map(|v| plain_option(v)).collect()
                }
                _ => return None,
            };
            let default = a.default.clone().map(QuestionValue::Choices);
            (
                &a.title,
                &a.description,
                QuestionFieldKind::MultiSelect(options),
                a.meta.as_ref(),
                default,
            )
        }
        ElicitationPropertySchema::Boolean(b) => (
            &b.title,
            &b.description,
            QuestionFieldKind::Boolean,
            b.meta.as_ref(),
            b.default.map(QuestionValue::Bool),
        ),
        _ => return None,
    };
    if matches!(&kind, QuestionFieldKind::SingleSelect(o) | QuestionFieldKind::MultiSelect(o) if o.is_empty())
    {
        return None;
    }
    Some(QuestionField {
        key: key.to_owned(),
        title: title.clone(),
        description: description.clone(),
        required,
        companion_of: matches!(kind, QuestionFieldKind::Text)
            .then(|| custom_answer_target(meta))
            .flatten(),
        kind,
        default,
    })
}

/// A single-select default must name an offered option; a free-text field
/// accepts any default.
fn in_options(kind: &QuestionFieldKind, default: &QuestionValue) -> bool {
    match (kind, default) {
        (QuestionFieldKind::SingleSelect(options), QuestionValue::Text(value)) => {
            options.iter().any(|o| o.value == *value)
        }
        _ => true,
    }
}

fn options_of(options: &[EnumOption]) -> Vec<QuestionOption> {
    options
        .iter()
        .map(|o| QuestionOption {
            value: o.value.clone(),
            label: o.title.clone(),
            description: o.description.clone(),
        })
        .collect()
}

fn plain_option(value: &str) -> QuestionOption {
    QuestionOption {
        value: value.to_owned(),
        label: value.to_owned(),
        description: None,
    }
}

/// The select field a free-text field is the "Other" box of, read from the
/// adapter's marker under either spelling.
fn custom_answer_target(meta: Option<&Meta>) -> Option<String> {
    let meta = meta?;
    let marker = meta
        .get("jetbrains")
        .and_then(|j| j.get("air"))
        .and_then(|a| a.get(CUSTOM_ANSWER_KEY))
        .or_else(|| meta.get(LEGACY_CUSTOM_ANSWER_KEY))?;
    marker
        .get("questionId")
        .and_then(|id| id.as_str())
        .map(str::to_owned)
}

/// The schema's properties arrive as a sorted map, which would put
/// `question_10` before `question_2`. Fields keep a natural numeric order of
/// their keys, so a multi-question form reads in the order it was asked.
fn ordered(mut fields: Vec<QuestionField>) -> Vec<QuestionField> {
    fields.sort_by_key(|f| natural_key(&f.key));
    fields
}

fn natural_key(key: &str) -> Vec<(String, u64)> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for c in key.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                parts.push((
                    std::mem::take(&mut text),
                    digits.parse().unwrap_or(u64::MAX),
                ));
                digits.clear();
            }
            text.push(c);
        }
    }
    let tail = if digits.is_empty() {
        0
    } else {
        digits.parse().unwrap_or(u64::MAX)
    };
    parts.push((text, tail));
    parts
}

#[cfg(test)]
mod tests;
