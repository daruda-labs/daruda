//! JSON-RPC framing and method dispatch for the MCP server. GPUI-free.

use crate::control::mcp::tools::ToolTable;

/// The version we implement.
pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";

/// Versions we answer as-is. Anything else gets [`PROTOCOL_VERSION`], which
/// the spec allows: "Otherwise, the server MUST respond with another protocol
/// version it supports."
///
/// One entry on purpose. 2025-03-26's basic spec says a server MUST *receive*
/// JSON-RPC batches; this one does not implement them (batching was dropped
/// again in 2025-06-18), so agreeing to that version would be a promise we
/// break silently. The only client is daruda's own orchestrator, so there is
/// nothing to keep compatible with.
pub(crate) const SUPPORTED_VERSIONS: [&str; 1] = [PROTOCOL_VERSION];

/// JSON-RPC error codes we emit. The first four are standard; `-32002` is
/// MCP's "server not initialized".
///
/// No `-32602` (invalid params): a bad tool argument comes back as a *tool*
/// error instead, so the model sees it and can fix the call — see
/// `convert::convert_error_result`.
pub(crate) const PARSE_ERROR: i32 = -32700;
pub(crate) const INVALID_REQUEST: i32 = -32600;
pub(crate) const METHOD_NOT_FOUND: i32 = -32601;
pub(crate) const INTERNAL_ERROR: i32 = -32603;
pub(crate) const NOT_INITIALIZED: i32 = -32002;

/// One connection's handshake progress.
///
/// Two flags rather than one, because the lifecycle has three states and the
/// middle one is real: after `initialize` but before
/// `notifications/initialized` the client is still setting up, and a request
/// then is as premature as one sent first. `ping` is exempt at every stage.
#[derive(Debug, Default)]
pub(crate) struct Session {
    initialized: bool,
    ready: bool,
}

impl Session {
    /// Whether `method` may run yet, as a frame the caller can send straight
    /// back.
    ///
    /// The one admission decision. `tools/call` is answered by the host, not
    /// by [`handle`], so a gate living only inside `handle` would be a gate
    /// the one effectful method never met — the host asks this instead of
    /// re-deriving it and possibly forgetting.
    pub(crate) fn refuse_if_premature(
        &self,
        method: &str,
        id: &serde_json::Value,
    ) -> Option<String> {
        // `ping` is exempt at every stage per the lifecycle spec, and so is
        // the handshake itself.
        let exempt = matches!(method, "ping" | "initialize");
        (!exempt && !self.ready)
            .then(|| error_frame(id.clone(), NOT_INITIALIZED, "server not initialized"))
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test() -> Self {
        Self {
            initialized: true,
            ready: true,
        }
    }
}

/// The request a `notifications/cancelled` withdraws, as the JSON text of its
/// id. `None` for every other frame.
///
/// Text rather than a `Value` because it is only ever used as a key: an id may
/// be a number or a string, and `1` must not match `"1"`.
///
/// Parsed here — with the rest of the method dispatch — but acted on by the
/// host, which is the only side that knows what a call left pending.
pub(crate) fn cancellation_target(frame: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(frame).ok()?;
    if value.get("method").and_then(|m| m.as_str()) != Some("notifications/cancelled") {
        return None;
    }
    let id = value.pointer("/params/requestId")?;
    // A cancellation naming nothing cancels nothing.
    (!id.is_null()).then(|| request_key(id))
}

/// The map key for a JSON-RPC id.
pub(crate) fn request_key(id: &serde_json::Value) -> String {
    id.to_string()
}

/// The id a frame must be answered on, or `None` when the frame is a
/// notification.
///
/// One home for the rule, because two sides ask it: [`handle`] and the host's
/// `tools/call` interception. If they disagreed, one would answer a
/// notification while the other left a request hanging.
///
/// An explicit `"id": null` counts as absent — a serializer that does not skip
/// `None` emits it, and JSON-RPC has no null id.
pub(crate) fn request_id(
    members: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    members.get("id").filter(|v| !v.is_null()).cloned()
}

/// Handle one inbound frame. `None` means the frame was a notification and
/// gets no reply — writing one would corrupt the stream.
pub(crate) fn handle(frame: &str, session: &mut Session, tools: &ToolTable) -> Option<String> {
    let parsed: serde_json::Value = match serde_json::from_str(frame) {
        Ok(value) => value,
        Err(_) => {
            return Some(error_frame(
                serde_json::Value::Null,
                PARSE_ERROR,
                "invalid JSON",
            ));
        }
    };
    // A frame that is valid JSON but not a request object gets an Invalid
    // Request, not silence: `Value::get` on an array or a scalar returns
    // `None` for every member, which would otherwise read as a notification
    // and leave a client waiting for a reply that never comes.
    let Some(members) = parsed.as_object() else {
        return Some(error_frame(
            serde_json::Value::Null,
            INVALID_REQUEST,
            "not a JSON-RPC request object",
        ));
    };
    let method = members
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default();

    // A notification has no id — and treating one as a request would answer it
    // *and* leave the handshake stuck.
    let Some(id) = request_id(members) else {
        if method == "notifications/initialized" {
            session.ready = session.initialized;
        }
        return None;
    };

    if let Some(refusal) = session.refuse_if_premature(method, &id) {
        return Some(refusal);
    }

    match method {
        // Allowed at any stage, per the lifecycle spec.
        "ping" => Some(result_frame(id, serde_json::json!({}))),
        "initialize" => {
            session.initialized = true;
            let requested = parsed
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let version = if SUPPORTED_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(result_frame(
                id,
                serde_json::json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": crate::surface::constants::AGENT_FACING_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ))
        }
        "tools/list" => Some(result_frame(
            id,
            serde_json::json!({ "tools": tools.describe() }),
        )),
        // `tools/call` needs the app, so the host intercepts it before it
        // reaches here. Internal error, not invalid request: the client did
        // nothing wrong — the host failed to route a method it owns.
        "tools/call" => Some(error_frame(
            id,
            INTERNAL_ERROR,
            "tools/call is dispatched by the host",
        )),
        _ => Some(error_frame(id, METHOD_NOT_FOUND, "unknown method")),
    }
}

pub(crate) fn result_frame(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub(crate) fn error_frame(id: serde_json::Value, code: i32, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
    .to_string()
}

#[cfg(test)]
mod tests;
