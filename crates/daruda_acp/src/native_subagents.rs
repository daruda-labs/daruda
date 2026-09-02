//! Native ACP subagent sessions, normalized into the flat tool hierarchy.
//!
//! An adapter that supports native subagents stops flattening a spawned agent's
//! work into the parent session. It announces the child on the *parent's*
//! stream (`subagent_spawned`) and then routes the child's own tool calls under
//! the child's `sessionId`. daruda's render model has no session axis — a
//! subagent is a tool call whose children name it through `parent_tool_id` — so
//! this module converts one shape into the other.
//!
//! Input is one raw `session/update` payload plus the session it arrived on;
//! output is zero or more standard [`SessionUpdate`]s the mapper already
//! understands. Nothing here knows about GPUI or the render model, and the
//! public `AcpEvent` enum is unchanged: a native subagent reaches the screen as
//! ordinary tool calls, which is what every renderer already handles.

use std::collections::{HashMap, HashSet};

use agent_client_protocol::schema::v1::{
    ContentBlock, MessageId, Meta, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use serde_json::Value;

use crate::SessionUpdate;
use crate::adapter::{AcpAdapter, DefaultAdapter, MessagePhase};

/// `_meta` namespace for values daruda synthesized itself, as opposed to
/// anything an agent put on the wire.
pub const DARUDA_META_KEY: &str = "daruda";

/// Key under [`DARUDA_META_KEY`] naming the synthetic parent a normalized child
/// call belongs to. Read by [`crate::adapter::AcpAdapter::parent_tool_id`].
pub const PARENT_TOOL_ID_META_KEY: &str = "parentToolId";

/// The draft protocol's announcement of a spawned child session, delivered on
/// the *parent's* stream.
const SUBAGENT_SPAWNED: &str = "subagent_spawned";

/// The draft protocol's terminal report for a child session, delivered on the
/// *parent's* stream.
const SUBAGENT_STATE_UPDATE: &str = "subagent_state_update";

/// Title shown when an announcement carries no usable name.
const FALLBACK_AGENT_NAME: &str = "Agent";

/// Vendor-private `_meta` extension both installed adapters gate native
/// subagent sessions on, and its two nested keys.
///
/// Not in the ACP spec. The adapters also accept a `subagents` *object* on
/// `ClientCapabilities` itself, but this schema version's `ClientCapabilities`
/// is a typed struct with no such field, so `_meta` — which is free-form — is
/// the reachable form. Read from adapter source, not guessed:
/// `codex-acp@1.8.0` and `claude-agent-acp@0.73.0` both require an integer
/// `version` of at least [`AIR_EXTENSION_VERSION`] and an array `capabilities`
/// containing [`NATIVE_SUBAGENT_SESSIONS_CAPABILITY`].
pub const JETBRAINS_META_KEY: &str = "jetbrains";

/// Nested under [`JETBRAINS_META_KEY`]; see that constant.
pub const AIR_META_KEY: &str = "air";

/// The extension version both adapters compare against with `>=`.
pub const AIR_EXTENSION_VERSION: u64 = 1;

/// The one capability daruda claims from this extension.
pub const NATIVE_SUBAGENT_SESSIONS_CAPABILITY: &str = "nativeSubagentSessions";

/// The `_meta` value that switches a supporting adapter into native subagent
/// mode. Built here rather than at the `initialize` call site so the shape and
/// the router that consumes its traffic stay in one place.
pub fn air_capabilities_meta() -> Value {
    serde_json::json!({
        AIR_META_KEY: {
            "version": AIR_EXTENSION_VERSION,
            "capabilities": [NATIVE_SUBAGENT_SESSIONS_CAPABILITY],
        }
    })
}

/// The synthetic tool call standing in for a child session.
fn parent_tool_id_of(child_session: &str) -> String {
    format!("subagent:{child_session}")
}

/// A child's own tool call id, namespaced by its session. ACP only promises a
/// tool call id is unique *within* a session, so flattening several child
/// sessions into one transcript needs the session in the key.
fn child_tool_id_of(child_session: &str, tool_call_id: &str) -> String {
    format!("subagent:{child_session}:tool:{tool_call_id}")
}

/// What the router made of one `session/update`.
pub enum Routed {
    /// Ordinary conversation traffic. Already deserialized, so the caller takes
    /// its existing path without parsing the payload a second time. Boxed for
    /// the same reason `AcpEvent::Update` is: the widest `SessionUpdate`
    /// variant would otherwise set the size of every `Routed`.
    Standard(Box<SessionUpdate>),
    /// Normalized into updates the mapper already understands. Possibly empty:
    /// a child's reasoning, plan, config and usage belong to the child, not to
    /// the conversation the user is reading.
    Normalized(Vec<SessionUpdate>),
    /// An update kind this build does not understand, reported once per kind so
    /// a chatty unknown cannot flood the host. Never fatal — an unrecognized
    /// update must not take down a working session.
    Unknown { kind: String },
}

/// One live child session and the synthetic call standing in for it.
struct ChildSession {
    /// Id of the synthetic tool call this child renders as. A nested child's
    /// own parent link is not kept here: it is stamped onto the synthetic call
    /// at registration and read back off the update like any other.
    tool_id: String,
    /// The child's answer so far, folded into the synthetic call's output when
    /// the child reaches a terminal state.
    answer: Vec<ContentBlock>,
    /// Which message the trailing block of `answer` belongs to. A change of
    /// message id starts a new block instead of extending the last one.
    answer_message_id: Option<String>,
}

/// Turns native subagent session traffic into the flat parent/child tool
/// hierarchy the render model already speaks.
///
/// One instance per connection: it holds the session graph for that connection
/// and nothing outlives it.
pub struct NativeSubagentRouter {
    /// Reads the vendor `_meta` this router has to interpret — today only which
    /// role a child's streamed message plays.
    adapter: Box<dyn AcpAdapter>,
    /// Child session id → its synthetic call. Membership is the whole test for
    /// "is this a subagent's traffic": a session is a child only once it has
    /// been *announced* as one. Guessing from an unrecognized session id
    /// instead would claim a second conversation in the same capture as a
    /// subagent and swallow its messages.
    children: HashMap<String, ChildSession>,
    /// Update kinds already reported through [`Routed::Unknown`].
    noticed: HashSet<String>,
}

impl Default for NativeSubagentRouter {
    fn default() -> Self {
        Self::new(Box::new(DefaultAdapter))
    }
}

impl NativeSubagentRouter {
    pub fn new(adapter: Box<dyn AcpAdapter>) -> Self {
        Self {
            adapter,
            children: HashMap::new(),
            noticed: HashSet::new(),
        }
    }

    /// Swap in the strategy for the agent that actually answered `initialize`.
    /// Must happen before a `session/load` replays history, for the same reason
    /// `AcpEvent::AgentIdentified` is emitted there: those updates have to be
    /// read under the right dialect.
    pub fn set_adapter(&mut self, adapter: Box<dyn AcpAdapter>) {
        self.adapter = adapter;
    }

    /// Whether this session is a subagent's child rather than a conversation of
    /// its own. A caller counting the sessions in a capture needs the
    /// difference: a child belongs to the conversation that spawned it.
    pub fn is_child(&self, session_id: &str) -> bool {
        self.children.contains_key(session_id)
    }

    /// Route one `session/update` payload that arrived on `session_id`.
    pub fn route(&mut self, session_id: &str, update: &Value) -> Routed {
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match kind {
            SUBAGENT_SPAWNED => Routed::Normalized(self.spawn(session_id, update)),
            SUBAGENT_STATE_UPDATE => Routed::Normalized(self.finish(update)),
            _ => {
                let Ok(standard) = serde_json::from_value::<SessionUpdate>(update.clone()) else {
                    return self.notice(kind);
                };
                // Only an *announced* child is rewritten. An unrecognized
                // session id is not evidence of a subagent — a tap appends
                // every session of a process run to one file — and both
                // adapters announce a child before routing anything to it
                // (codex flushes its buffer only after `materialize`; claude
                // awaits `announceNativeSubagent` first), so there is no
                // unannounced-child case left to guess at.
                if !self.children.contains_key(session_id) {
                    return Routed::Standard(Box::new(standard));
                }
                Routed::Normalized(self.rewrite_child(session_id, standard))
            }
        }
    }

    /// Report `kind` the first time it is seen; stay silent afterwards.
    fn notice(&mut self, kind: &str) -> Routed {
        if self.noticed.insert(kind.to_owned()) {
            Routed::Unknown {
                kind: kind.to_owned(),
            }
        } else {
            Routed::Normalized(Vec::new())
        }
    }

    /// Record a child session and build the synthetic call that stands for it.
    fn register(
        &mut self,
        child_session: &str,
        parent_session: Option<&str>,
        name: &str,
        task: &str,
    ) -> SessionUpdate {
        // A child spawned by another child nests under that child's synthetic
        // call; one spawned by the root owns a top-level row.
        let parent_tool_id = parent_session
            .and_then(|p| self.children.get(p))
            .map(|c| c.tool_id.clone());
        let tool_id = parent_tool_id_of(child_session);
        self.children.insert(
            child_session.to_owned(),
            ChildSession {
                tool_id: tool_id.clone(),
                answer: Vec::new(),
                answer_message_id: None,
            },
        );

        // `raw_input` is the renderer's only signal that a call is a subagent
        // launch (`ToolCallItem::is_subagent_launch`), so the announcement's
        // name and task go in under the field names that predicate reads.
        let raw_input = serde_json::json!({ "subagent_type": name, "prompt": task });
        let mut call = ToolCall::new(tool_id, name.to_owned())
            .kind(ToolKind::Think)
            .status(ToolCallStatus::InProgress)
            .raw_input(raw_input);
        if let Some(parent) = parent_tool_id {
            call = call.meta(with_parent_tool_id(None, &parent));
        }
        SessionUpdate::ToolCall(call)
    }

    /// `subagent_spawned`, which always arrives on the *parent's* stream.
    fn spawn(&mut self, parent_session: &str, update: &Value) -> Vec<SessionUpdate> {
        let Some(child_session) = update
            .get("subagentSessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Vec::new();
        };
        // A re-spawn arrives under a fresh `<thread>:generation:N` session id,
        // so it registers as its own child and earns its own card — the wire
        // says two runs happened, and the transcript says the same.
        if self.children.contains_key(child_session) {
            return Vec::new();
        }
        let name = update
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(FALLBACK_AGENT_NAME)
            .to_owned();
        let task = update
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        vec![self.register(child_session, Some(parent_session), &name, &task)]
    }

    /// `subagent_state_update` — the child's terminal report.
    fn finish(&mut self, update: &Value) -> Vec<SessionUpdate> {
        let Some(child_session) = update.get("subagentSessionId").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(child) = self.children.get_mut(child_session) else {
            return Vec::new();
        };
        // `cancelled` and `disconnected` have no ACP v1 tool status of their
        // own; `failed` is the only terminal status that is not a success, and
        // saying a cancelled run completed would be worse than saying it failed.
        let status = match update.get("state").and_then(Value::as_str) {
            Some("completed") => ToolCallStatus::Completed,
            _ => ToolCallStatus::Failed,
        };
        let content: Vec<ToolCallContent> = std::mem::take(&mut child.answer)
            .into_iter()
            .map(ToolCallContent::from)
            .collect();
        let mut fields = ToolCallUpdateFields::new().status(status);
        // An empty `content` is a replacement, not a no-op, so only send it when
        // the child actually said something.
        if !content.is_empty() {
            fields = fields.content(content);
        }
        vec![SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            child.tool_id.clone(),
            fields,
        ))]
    }

    /// Re-key a child session's own update into the root transcript.
    fn rewrite_child(&mut self, child_session: &str, update: SessionUpdate) -> Vec<SessionUpdate> {
        let Some(child) = self.children.get(child_session) else {
            return Vec::new();
        };
        let parent = child.tool_id.clone();
        match update {
            SessionUpdate::ToolCall(mut call) => {
                call.tool_call_id = child_tool_id_of(child_session, &call.tool_call_id.0).into();
                call.meta = Some(with_parent_tool_id(call.meta, &parent));
                vec![SessionUpdate::ToolCall(call)]
            }
            SessionUpdate::ToolCallUpdate(mut tcu) => {
                tcu.tool_call_id = child_tool_id_of(child_session, &tcu.tool_call_id.0).into();
                tcu.meta = Some(with_parent_tool_id(tcu.meta, &parent));
                vec![SessionUpdate::ToolCallUpdate(tcu)]
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                if self.adapter.message_phase(&chunk.meta) == MessagePhase::Answer {
                    self.accumulate(child_session, chunk.content, chunk.message_id);
                }
                Vec::new()
            }
            // Everything else a child emits — its reasoning, its own plan, its
            // config and usage accounting — describes the child's run, not the
            // conversation, and mixing it into the root transcript would read as
            // the main agent's own work.
            _ => Vec::new(),
        }
    }

    /// Fold one answer chunk into the child's pending output.
    fn accumulate(
        &mut self,
        child_session: &str,
        block: ContentBlock,
        message_id: Option<MessageId>,
    ) {
        let Some(child) = self.children.get_mut(child_session) else {
            return;
        };
        let message_id = message_id.map(|id| id.0.to_string());
        // Chunks of one message concatenate; a new message id starts a block, so
        // two separate answers never run together into one paragraph.
        if child.answer_message_id == message_id
            && let (Some(ContentBlock::Text(last)), ContentBlock::Text(next)) =
                (child.answer.last_mut(), &block)
        {
            last.text.push_str(&next.text);
            return;
        }
        child.answer_message_id = message_id;
        child.answer.push(block);
    }

    /// The synthetic parent standing in for a child session, for tests and for
    /// callers correlating a child session back to its card.
    #[cfg(test)]
    fn tool_id_of(&self, child_session: &str) -> Option<&str> {
        self.children.get(child_session).map(|c| c.tool_id.as_str())
    }
}

/// Stamp daruda's own parent link onto an update's `_meta`, preserving whatever
/// the agent already put there.
fn with_parent_tool_id(meta: Option<Meta>, parent_tool_id: &str) -> Meta {
    let mut meta = meta.unwrap_or_default();
    meta.insert(
        DARUDA_META_KEY.to_owned(),
        serde_json::json!({ PARENT_TOOL_ID_META_KEY: parent_tool_id }),
    );
    meta
}

#[cfg(test)]
mod tests;
