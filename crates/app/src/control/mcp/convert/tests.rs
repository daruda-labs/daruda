use super::*;
use crate::control::result::{BriefSummary, ControlError, ControlResult};

fn pane_json() -> Value {
    serde_json::json!({ "workspace": uuid::Uuid::nil(), "pane": 7 })
}

fn workspace_json() -> Value {
    serde_json::json!(uuid::Uuid::nil())
}

fn nil_workspace() -> daruda_store::project::WorkspaceUuid {
    serde_json::from_value(workspace_json()).expect("uuid")
}

/// The payload the model actually reads: JSON parsed back out of the one
/// text block.
fn payload(result: &Value) -> Value {
    let text = result["content"][0]["text"].as_str().expect("text");
    serde_json::from_str(text).expect("json payload")
}

/// Both carriers say the same thing. A client reads one or the other, so
/// a result that disagreed with itself would be two different answers to
/// one call.
#[test]
fn the_text_block_and_the_structured_content_agree() {
    let cases = [
        to_tool_result(&Err(crate::control::result::ControlError::TargetGone)),
        convert_error_result(&ConvertError::MissingArgument { name: "text" }),
        unknown_tool_result("daruda_nope"),
    ];
    for result in cases {
        assert_eq!(
            result["structuredContent"],
            payload(&result),
            "the two carriers must not diverge: {result}"
        );
        assert!(
            result["structuredContent"].is_object(),
            "the spec's structured content is an object: {result}"
        );
    }
}

#[test]
fn chat_send_converts() {
    let args = serde_json::json!({ "target": pane_json(), "text": "go" });
    match to_command(ToolId::ChatSend, &args).expect("converted") {
        Command::Immediate(ResolvedCommand::Say { text, .. }) => assert_eq!(text, "go"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn a_missing_required_argument_is_a_convert_error() {
    let args = serde_json::json!({ "target": pane_json() });
    assert_eq!(
        to_command(ToolId::ChatSend, &args),
        Err(ConvertError::MissingArgument { name: "text" })
    );
}

#[test]
fn a_wrongly_typed_argument_is_a_convert_error() {
    let args = serde_json::json!({ "target": pane_json(), "text": 42 });
    assert_eq!(
        to_command(ToolId::ChatSend, &args),
        Err(ConvertError::BadArgument { name: "text" })
    );
}

#[test]
fn an_unparseable_target_is_a_convert_error() {
    let args = serde_json::json!({ "target": "not-an-object", "text": "go" });
    assert_eq!(
        to_command(ToolId::ChatSend, &args),
        Err(ConvertError::BadArgument { name: "target" })
    );
}

#[test]
fn argument_free_tools_ignore_extra_arguments() {
    let args = serde_json::json!({ "unexpected": true });
    assert_eq!(
        to_command(ToolId::ChatList, &args).expect("converted"),
        Command::Immediate(ResolvedCommand::List)
    );
}

/// The listing emits `worktree`; `lane` is the accepted alias. A model
/// copying either back has to work.
#[test]
fn a_lane_handle_is_accepted_under_both_key_names() {
    for key in ["worktree", "lane"] {
        let args = serde_json::json!({
            "worktree": { "workspace": workspace_json(), "project": 1, key: 2 },
        });
        assert_eq!(
            to_command(ToolId::ChatNew, &args).expect("converted"),
            Command::Gated(GatedCommand::ChatNew {
                lane: LaneHandle {
                    workspace: nil_workspace(),
                    project: 1,
                    lane: 2,
                },
                agent: None,
            })
        );
    }
}

#[test]
fn lane_create_takes_its_optional_arguments_or_leaves_them_unset() {
    let bare = serde_json::json!({ "workspace": workspace_json(), "project": 3, "name": "fix" });
    assert_eq!(
        to_command(ToolId::LaneCreate, &bare).expect("converted"),
        Command::Gated(GatedCommand::LaneCreate {
            workspace: nil_workspace(),
            project: 3,
            name: "fix".into(),
            base_ref: None,
            agent: None,
            prompt: None,
        })
    );

    let full = serde_json::json!({
        "workspace": workspace_json(), "project": 3, "name": "fix",
        "base_ref": "main", "agent": "codex-acp", "prompt": "start",
    });
    assert_eq!(
        to_command(ToolId::LaneCreate, &full).expect("converted"),
        Command::Gated(GatedCommand::LaneCreate {
            workspace: nil_workspace(),
            project: 3,
            name: "fix".into(),
            base_ref: Some("main".into()),
            agent: Some("codex-acp".into()),
            prompt: Some("start".into()),
        })
    );
}

/// An explicit `null` reads as "not supplied" for an optional argument and
/// as missing for a required one — models emit both.
#[test]
fn an_explicit_null_reads_as_absent() {
    let args = serde_json::json!({
        "workspace": workspace_json(), "project": 3, "name": "fix", "agent": null,
    });
    assert!(matches!(
        to_command(ToolId::LaneCreate, &args),
        Ok(Command::Gated(GatedCommand::LaneCreate { agent: None, .. }))
    ));
    let args = serde_json::json!({ "target": pane_json(), "text": null });
    assert_eq!(
        to_command(ToolId::ChatSend, &args),
        Err(ConvertError::MissingArgument { name: "text" })
    );
}

/// A wrongly typed *optional* argument is still a mistake — silently
/// dropping it would run a different command than the one asked for.
#[test]
fn a_wrongly_typed_optional_argument_is_still_refused() {
    let args = serde_json::json!({
        "workspace": workspace_json(), "project": 3, "name": "fix", "agent": 7,
    });
    assert_eq!(
        to_command(ToolId::LaneCreate, &args),
        Err(ConvertError::BadArgument { name: "agent" })
    );
}

/// A negative or fractional project id is not a handle daruda ever
/// emitted, so it is a bad argument rather than a missing one.
#[test]
fn a_project_id_that_is_not_a_whole_number_is_refused() {
    for bad in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("3"),
    ] {
        let args = serde_json::json!({
            "workspace": workspace_json(), "project": bad, "name": "fix",
        });
        assert_eq!(
            to_command(ToolId::LaneCreate, &args),
            Err(ConvertError::BadArgument { name: "project" })
        );
    }
}

#[test]
fn a_successful_outcome_renders_as_structured_content() {
    let out: ControlOutcome = Ok(ControlResult::Brief(BriefSummary {
        working: 1,
        awaiting_permission: 0,
        error: 0,
        total: 3,
    }));
    let v = to_tool_result(&out);
    assert_eq!(v["isError"], false);
    assert_eq!(v["content"][0]["type"], "text");
    let inner = payload(&v);
    assert_eq!(inner["kind"], "brief");
    assert_eq!(inner["working"], 1);
}

#[test]
fn a_failed_outcome_is_marked_as_an_error() {
    let out: ControlOutcome = Err(ControlError::TargetGone);
    let v = to_tool_result(&out);
    assert_eq!(v["isError"], true);
    let text = v["content"][0]["text"].as_str().expect("text");
    assert!(
        text.contains("target_gone"),
        "the code travels, not a sentence: {text}"
    );
}

/// Every refusal the guards and the gate produce has to reach the model as
/// a code it can branch on.
#[test]
fn every_new_refusal_travels_as_its_code() {
    for (error, code) in [
        (ControlError::ApprovalRefused, "approval_refused"),
        (ControlError::ApprovalTimedOut, "approval_timed_out"),
        (ControlError::AgentLimitReached, "agent_limit_reached"),
        (ControlError::QueueFull, "queue_full"),
        (ControlError::SelfTargetRefused, "self_target_refused"),
        (ControlError::LaneCreateBusy, "lane_create_busy"),
        (
            ControlError::LaneCreateFailed {
                detail: "fatal: a branch named 'main' already exists".into(),
            },
            "lane_create_failed",
        ),
        (ControlError::LaneNameInvalid, "lane_name_invalid"),
        (ControlError::ApprovalUnavailable, "approval_unavailable"),
        (ControlError::ApprovalsPending, "approvals_pending"),
    ] {
        let v = to_tool_result(&Err(error));
        assert_eq!(v["isError"], true);
        assert_eq!(payload(&v)["code"], code);
    }
}

/// The model's most common failure has to be the most branchable one:
/// a code plus the argument it got wrong.
#[test]
fn a_convert_failure_carries_a_code_and_the_argument() {
    let v = convert_error_result(&ConvertError::MissingArgument { name: "text" });
    assert_eq!(v["isError"], true);
    assert_eq!(payload(&v)["code"], "missing_argument");
    assert_eq!(payload(&v)["argument"], "text");

    let v = convert_error_result(&ConvertError::BadArgument { name: "project" });
    assert_eq!(payload(&v)["code"], "bad_argument");
    assert_eq!(payload(&v)["argument"], "project");

    let v = unknown_tool_result("daruda_nope");
    assert_eq!(v["isError"], true);
    assert_eq!(payload(&v)["code"], "unknown_tool");
    assert_eq!(payload(&v)["tool"], "daruda_nope");
}

/// The schema and the converter are two statements of one contract. A key
/// spelled differently in each, or a `required` the converter treats as
/// optional, would compile and ship — the model would then pass an
/// argument nothing reads, or omit one the converter demands.
#[test]
fn every_tools_schema_matches_what_the_converter_reads() {
    for tool in crate::control::mcp::tools::table_for_test() {
        let properties = (tool.properties)();
        let map = properties.as_object().expect("object schema");
        let full: Value = map
            .iter()
            .map(|(key, schema)| (key.clone(), dummy_for(schema)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        assert!(
            to_command(tool.id, &full).is_ok(),
            "{}: the converter rejects its own schema: {:?}",
            tool.name,
            to_command(tool.id, &full)
        );

        // Both directions. A `required` key the converter shrugs off, and
        // an optional key it secretly demands, are the two ways the
        // schema and the converter can disagree — and the model only ever
        // sees the schema.
        for key in map.keys() {
            let mut without = full.clone();
            without
                .as_object_mut()
                .expect("object")
                .remove(key)
                .expect("declared property");
            let outcome = to_command(tool.id, &without);
            if tool.required.contains(&key.as_str()) {
                assert_eq!(
                    outcome,
                    Err(ConvertError::MissingArgument {
                        name: leaked(key.clone())
                    }),
                    "{}: `{key}` is required, so dropping it must name it",
                    tool.name
                );
            } else {
                assert!(
                    outcome.is_ok(),
                    "{}: `{key}` is optional in the schema but the \
                     converter demands it: {outcome:?}",
                    tool.name
                );
            }
        }
    }
}

/// The gate and the command shape are the same fact stated twice: the
/// table says a tool needs approval, and the converter decides that by
/// returning `Command::Gated`. Nothing in the type system binds them, and
/// a release build has no `debug_assert` — so the tool that disagreed
/// would either skip the user's approval or wait for one on a read.
///
/// Exhaustive over the table, which is what makes it a binding rather than
/// a spot check.
#[test]
fn the_declared_gate_and_the_converted_command_agree() {
    use crate::control::mcp::tools::{Gate, ToolTable};

    let table = ToolTable::all();
    for tool in crate::control::mcp::tools::table_for_test() {
        let properties = (tool.properties)();
        let full: Value = properties
            .as_object()
            .expect("object schema")
            .iter()
            .map(|(key, schema)| (key.clone(), dummy_for(schema)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        let command = to_command(tool.id, &full).expect("its own schema converts");
        assert_eq!(
            matches!(command, Command::Gated(_)),
            table.gate(tool.id) == Gate::NeedsApproval,
            "{}: gated {:?} but converts to {command:?}",
            tool.name,
            table.gate(tool.id)
        );
    }
}

/// `ConvertError` names arguments with `&'static str`, and a schema key is
/// owned — so a test comparing the two has to lengthen one of them.
fn leaked(key: String) -> &'static str {
    Box::leak(key.into_boxed_str())
}

/// A value of the type a property declares. Objects are filled from their
/// own nested schema, so a handle argument gets every key it needs.
fn dummy_for(schema: &Value) -> Value {
    match schema["type"].as_str() {
        Some("integer") => serde_json::json!(1),
        Some("object") => schema["properties"]
            .as_object()
            .map(|nested| {
                nested
                    .iter()
                    .map(|(key, inner)| (key.clone(), dummy_for(inner)))
                    .collect::<serde_json::Map<_, _>>()
                    .into()
            })
            .unwrap_or_else(|| serde_json::json!({})),
        // A uuid-shaped string where the property is named for one, so a
        // `WorkspaceUuid` parses; any other string is free-form.
        Some("string") => serde_json::json!(uuid::Uuid::nil()),
        other => panic!("unhandled schema type: {other:?}"),
    }
}
