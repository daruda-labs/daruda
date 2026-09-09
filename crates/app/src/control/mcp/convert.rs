//! Tool arguments in, tool results out. GPUI-free.
//!
//! Both directions live together because they are one contract: what a tool
//! accepts and what it answers with are the two halves of its schema, and
//! splitting them is how they drift.
//!
//! A result travels twice: as `structuredContent` (the object itself) and as
//! the same object serialized into a single text block. 2025-06-18 added the
//! former and says a tool returning it SHOULD also return the serialized JSON
//! as text — so a client that reads either one sees the same
//! [`ControlResult`] / [`ControlError`], tag and all. An error carries its
//! *code* (`target_gone`), never a sentence: the model branches on it, and it
//! must not depend on the user's locale.

use serde_json::Value;

use crate::control::mcp::tools::ToolId;
use crate::control::result::{ControlOutcome, LaneHandle};
use crate::control::spec::{FlowCommand, GatedCommand, ResolvedCommand};
use crate::telegram::bridge::PaneRef;

/// Why a call could not become a command. Reported as a tool error, not a
/// protocol one, so the model can fix its arguments and retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConvertError {
    MissingArgument { name: &'static str },
    BadArgument { name: &'static str },
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArgument { name } => write!(f, "missing argument: {name}"),
            Self::BadArgument { name } => write!(f, "bad argument: {name}"),
        }
    }
}

impl ConvertError {
    /// The tool-result payload for this failure.
    ///
    /// A code plus the offending argument, not a sentence — this is the error
    /// class an LLM caller hits most (it got an argument wrong), so it is the
    /// one it most needs to branch on. Same `{"code": …}` shape
    /// [`crate::control::result::ControlError`] serializes into.
    fn payload(&self) -> Value {
        let (code, name) = match self {
            Self::MissingArgument { name } => ("missing_argument", name),
            Self::BadArgument { name } => ("bad_argument", name),
        };
        serde_json::json!({ "code": code, "argument": name })
    }
}

/// What a converted call turns into. The two halves go to different executor
/// entry points, so the conversion is where they part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Immediate(ResolvedCommand),
    Gated(GatedCommand),
}

/// Turn a called tool plus its arguments into a command the executor runs.
///
/// Extra arguments are ignored rather than refused: a model that adds a field
/// the schema does not mention still asked for the thing the tool does, and
/// MCP has no notion of a strict schema.
pub(crate) fn to_command(id: ToolId, args: &Value) -> Result<Command, ConvertError> {
    let command = match id {
        ToolId::ChatList => Command::Immediate(ResolvedCommand::List),
        ToolId::Status => Command::Immediate(ResolvedCommand::Brief),
        ToolId::LaneList => Command::Immediate(ResolvedCommand::LaneList),
        ToolId::FlowList => Command::Immediate(ResolvedCommand::Flow(FlowCommand::List)),
        ToolId::ChatSend => Command::Immediate(ResolvedCommand::Say {
            target: pane_arg(args, "target")?,
            text: string_arg(args, "text")?,
        }),
        ToolId::ChatStop => Command::Immediate(ResolvedCommand::Stop {
            target: pane_arg(args, "target")?,
        }),
        ToolId::FlowRun => Command::Immediate(ResolvedCommand::Flow(FlowCommand::Run {
            name: string_arg(args, "name")?,
        })),
        ToolId::LaneCreate => Command::Gated(GatedCommand::LaneCreate {
            workspace: uuid_arg(args, "workspace")?,
            project: u64_arg(args, "project")?,
            name: string_arg(args, "name")?,
            base_ref: optional_string_arg(args, "base_ref")?,
            agent: optional_string_arg(args, "agent")?,
            prompt: optional_string_arg(args, "prompt")?,
        }),
        ToolId::ChatNew => Command::Gated(GatedCommand::ChatNew {
            lane: lane_arg(args, "worktree")?,
            agent: optional_string_arg(args, "agent")?,
        }),
    };
    Ok(command)
}

/// Render an outcome as an MCP tool result.
pub(crate) fn to_tool_result(outcome: &ControlOutcome) -> Value {
    let (is_error, payload) = match outcome {
        Ok(result) => (false, serde_json::to_value(result)),
        Err(error) => (true, serde_json::to_value(error)),
    };
    match payload {
        Ok(value) => tool_result(is_error, value),
        // A shape that will not serialize is a bug in the result type, not
        // something the caller did — but it still has to arrive as an error
        // rather than as a silently empty success.
        Err(_) => tool_result(true, serde_json::json!({ "code": "internal_error" })),
    }
}

/// A tool error for a call that never reached the executor.
///
/// Reported as a *tool* error rather than a JSON-RPC `-32602`, which the spec
/// lists these under: a protocol error is often swallowed by the client before
/// the model sees it, and the model's recovery here is to fix its arguments or
/// call `tools/list` again. A deliberate deviation, and a safe one — the only
/// client is daruda's own shim.
pub(crate) fn convert_error_result(error: &ConvertError) -> Value {
    tool_result(true, error.payload())
}

/// Same, for a name the table does not hold.
pub(crate) fn unknown_tool_result(name: &str) -> Value {
    tool_result(
        true,
        serde_json::json!({ "code": "unknown_tool", "tool": name }),
    )
}

/// One payload, both ways the protocol can carry it.
///
/// `structuredContent` is what a client parses; the text block is the same
/// object serialized, which is what the spec asks a structured result to also
/// return and what a client with no structured support renders.
fn tool_result(is_error: bool, payload: Value) -> Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "structuredContent": payload,
        "isError": is_error,
    })
}

/// An absent argument and a wrongly typed one are different mistakes, and the
/// model fixes them differently — so every accessor below keeps them apart.
fn field<'a>(args: &'a Value, name: &'static str) -> Result<&'a Value, ConvertError> {
    args.get(name)
        .filter(|v| !v.is_null())
        .ok_or(ConvertError::MissingArgument { name })
}

fn string_arg(args: &Value, name: &'static str) -> Result<String, ConvertError> {
    field(args, name)?
        .as_str()
        .map(str::to_owned)
        .ok_or(ConvertError::BadArgument { name })
}

/// `None` for absent or null; still an error for present-but-wrong-type.
fn optional_string_arg(args: &Value, name: &'static str) -> Result<Option<String>, ConvertError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or(ConvertError::BadArgument { name }),
    }
}

fn u64_arg(args: &Value, name: &'static str) -> Result<u64, ConvertError> {
    field(args, name)?
        .as_u64()
        .ok_or(ConvertError::BadArgument { name })
}

fn pane_arg(args: &Value, name: &'static str) -> Result<PaneRef, ConvertError> {
    serde_json::from_value(field(args, name)?.clone())
        .map_err(|_| ConvertError::BadArgument { name })
}

fn lane_arg(args: &Value, name: &'static str) -> Result<LaneHandle, ConvertError> {
    serde_json::from_value(field(args, name)?.clone())
        .map_err(|_| ConvertError::BadArgument { name })
}

fn uuid_arg(
    args: &Value,
    name: &'static str,
) -> Result<daruda_store::project::WorkspaceUuid, ConvertError> {
    serde_json::from_value(field(args, name)?.clone())
        .map_err(|_| ConvertError::BadArgument { name })
}

#[cfg(test)]
mod tests {
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
        let bare =
            serde_json::json!({ "workspace": workspace_json(), "project": 3, "name": "fix" });
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
}
