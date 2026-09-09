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
mod tests;
