use super::*;

/// The request claude-agent-acp sends for one `AskUserQuestion` question to
/// a client that advertises AIR: a select and its marked "Other" box.
fn ask_user_question() -> serde_json::Value {
    serde_json::json!({
        "mode": "form",
        "sessionId": "s-1",
        "toolCallId": "toolu_1",
        "message": "Which color should I use?",
        "requestedSchema": {
            "type": "object",
            "properties": {
                "question_0": {
                    "type": "string",
                    "title": "Color",
                    "oneOf": [
                        { "const": "red", "title": "red", "description": "warm" },
                        { "const": "blue", "title": "blue" }
                    ]
                },
                "question_0_custom": {
                    "type": "string",
                    "title": "Other",
                    "description": "Type your own answer",
                    "_meta": {
                        "_askUserQuestionCustomAnswer": { "questionId": "question_0", "isCustomAnswer": true },
                        "jetbrains": { "air": { "customAnswer": { "questionId": "question_0", "isCustomAnswer": true } } }
                    }
                }
            }
        }
    })
}

fn parse(value: serde_json::Value) -> Option<QuestionRequest> {
    let request: CreateElicitationRequest = serde_json::from_value(value).expect("valid request");
    QuestionRequest::from_protocol(&request)
}

#[test]
fn a_select_question_keeps_its_options_and_its_other_box() {
    let question = parse(ask_user_question()).expect("renderable");
    assert_eq!(question.message, "Which color should I use?");
    assert_eq!(question.tool_call_id.as_deref(), Some("toolu_1"));
    assert_eq!(question.fields.len(), 2);
    let select = &question.fields[0];
    assert_eq!(select.key, "question_0");
    assert_eq!(select.title.as_deref(), Some("Color"));
    assert_eq!(
        select.kind,
        QuestionFieldKind::SingleSelect(vec![
            QuestionOption {
                value: "red".into(),
                label: "red".into(),
                description: Some("warm".into()),
            },
            QuestionOption {
                value: "blue".into(),
                label: "blue".into(),
                description: None,
            },
        ])
    );
    let other = &question.fields[1];
    assert_eq!(other.kind, QuestionFieldKind::Text);
    assert_eq!(other.companion_of.as_deref(), Some("question_0"));
}

/// Without the marker (a client the adapter does not treat as AIR) the box is
/// an ordinary text field rather than lost.
#[test]
fn an_unmarked_text_field_stands_alone() {
    let mut value = ask_user_question();
    value["requestedSchema"]["properties"]["question_0_custom"]
        .as_object_mut()
        .unwrap()
        .remove("_meta");
    let question = parse(value).expect("renderable");
    assert_eq!(question.fields[1].companion_of, None);
}

#[test]
fn a_multi_select_and_a_boolean_are_rendered() {
    let question = parse(serde_json::json!({
        "mode": "form",
        "sessionId": "s-1",
        "message": "Pick",
        "requestedSchema": {
            "type": "object",
            "required": ["features"],
            "properties": {
                "features": {
                    "type": "array",
                    "items": { "anyOf": [ { "const": "a", "title": "A" }, { "const": "b", "title": "B" } ] }
                },
                "confirm": { "type": "boolean", "title": "Sure?" }
            }
        }
    }))
    .expect("renderable");
    let features = question
        .fields
        .iter()
        .find(|f| f.key == "features")
        .unwrap();
    assert!(features.required);
    assert!(matches!(&features.kind, QuestionFieldKind::MultiSelect(o) if o.len() == 2));
    let confirm = question.fields.iter().find(|f| f.key == "confirm").unwrap();
    assert!(!confirm.required);
    assert_eq!(confirm.kind, QuestionFieldKind::Boolean);
}

/// A form with a field there is no control for is refused whole: answering
/// part of it would hand the agent a reply to a question it did not ask.
#[test]
fn a_form_with_an_unrenderable_field_is_refused() {
    assert_eq!(
        parse(serde_json::json!({
            "mode": "form",
            "sessionId": "s-1",
            "message": "How many?",
            "requestedSchema": { "type": "object", "properties": { "n": { "type": "number" } } }
        })),
        None
    );
    assert_eq!(
        parse(serde_json::json!({
            "mode": "url",
            "sessionId": "s-1",
            "message": "Sign in",
            "url": "https://example.com",
            "elicitationId": "e-1"
        })),
        None
    );
}

#[test]
fn fields_read_in_the_order_they_were_asked() {
    let mut properties = serde_json::Map::new();
    for key in [
        "question_10",
        "question_2",
        "question_2_custom",
        "question_1",
    ] {
        properties.insert(key.into(), serde_json::json!({ "type": "string" }));
    }
    let question = parse(serde_json::json!({
        "mode": "form",
        "sessionId": "s-1",
        "message": "Several",
        "requestedSchema": { "type": "object", "properties": properties }
    }))
    .expect("renderable");
    let keys: Vec<_> = question.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "question_1",
            "question_2",
            "question_2_custom",
            "question_10"
        ]
    );
}

#[test]
fn each_answer_becomes_the_response_the_adapter_reads() {
    let mut values = BTreeMap::new();
    values.insert("question_0".into(), QuestionValue::Text("blue".into()));
    values.insert(
        "features".into(),
        QuestionValue::Choices(vec!["a".into(), "b".into()]),
    );
    values.insert("confirm".into(), QuestionValue::Bool(true));
    let submitted = serde_json::to_value(QuestionAnswer::Submit(values).into_response()).unwrap();
    assert_eq!(
        submitted,
        serde_json::json!({
            "action": "accept",
            "content": { "confirm": true, "features": ["a", "b"], "question_0": "blue" }
        })
    );
    assert_eq!(
        serde_json::to_value(QuestionAnswer::Decline.into_response()).unwrap(),
        serde_json::json!({ "action": "decline" })
    );
    assert_eq!(
        serde_json::to_value(QuestionAnswer::Cancel.into_response()).unwrap(),
        serde_json::json!({ "action": "cancel" })
    );
}

/// A trailing number too long for `u64` sorts last, wherever it sits in the
/// key, rather than first.
#[test]
fn an_overlong_trailing_number_sorts_last() {
    let mut properties = serde_json::Map::new();
    for key in ["question_99999999999999999999999", "question_1"] {
        properties.insert(key.into(), serde_json::json!({ "type": "string" }));
    }
    let question = parse(serde_json::json!({
        "mode": "form",
        "sessionId": "s-1",
        "message": "Several",
        "requestedSchema": { "type": "object", "properties": properties }
    }))
    .expect("renderable");
    assert_eq!(question.fields[0].key, "question_1");
}

#[test]
fn defaults_are_read_per_kind() {
    let question = parse(serde_json::json!({
        "mode": "form",
        "sessionId": "s-1",
        "message": "Setup",
        "requestedSchema": {
            "type": "object",
            "properties": {
                "a_text": { "type": "string", "default": "x" },
                "b_one": {
                    "type": "string",
                    "default": "blue",
                    "oneOf": [{ "const": "red", "title": "red" }, { "const": "blue", "title": "blue" }]
                },
                "c_stale": {
                    "type": "string",
                    "default": "green",
                    "oneOf": [{ "const": "red", "title": "red" }]
                },
                "d_many": {
                    "type": "array",
                    "default": ["p", "q"],
                    "items": { "type": "string", "enum": ["p", "q", "r"] }
                },
                "e_flag": { "type": "boolean", "default": true },
                "f_none": { "type": "boolean" }
            }
        }
    }))
    .expect("renderable");
    let default = |key: &str| {
        question
            .fields
            .iter()
            .find(|f| f.key == key)
            .unwrap()
            .default
            .clone()
    };
    assert_eq!(default("a_text"), Some(QuestionValue::Text("x".into())));
    assert_eq!(default("b_one"), Some(QuestionValue::Text("blue".into())));
    assert_eq!(
        default("c_stale"),
        None,
        "a default outside the options is dropped"
    );
    assert_eq!(
        default("d_many"),
        Some(QuestionValue::Choices(vec!["p".into(), "q".into()]))
    );
    assert_eq!(default("e_flag"), Some(QuestionValue::Bool(true)));
    assert_eq!(default("f_none"), None);
    assert_eq!(question.delivery, QuestionDelivery::Blocking);
}
