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
mod tests {
    use super::*;

    fn init_frame(version: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","capabilities":{{}},"clientInfo":{{"name":"t","version":"1"}}}}}}"#
        )
    }

    fn id_one() -> serde_json::Value {
        serde_json::json!(1)
    }

    fn reply(frame: &str, session: &mut Session) -> serde_json::Value {
        let out = handle(frame, session, &ToolTable::empty()).expect("reply");
        serde_json::from_str(&out).expect("json")
    }

    #[test]
    fn a_supported_version_is_echoed_back() {
        let mut session = Session::default();
        let v = reply(&init_frame(PROTOCOL_VERSION), &mut session);
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(
            v["result"]["capabilities"]["tools"].is_object(),
            "tools advertised"
        );
        assert!(v["result"]["serverInfo"]["name"].is_string());
    }

    /// The spec lets a server answer with a version it does support rather
    /// than failing, which is what keeps an older client working.
    #[test]
    fn an_unsupported_version_answers_with_ours() {
        let mut session = Session::default();
        let v = reply(&init_frame("1999-01-01"), &mut session);
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn every_supported_version_is_echoed_rather_than_replaced() {
        for version in SUPPORTED_VERSIONS {
            let mut session = Session::default();
            let v = reply(&init_frame(version), &mut session);
            assert_eq!(v["result"]["protocolVersion"], version);
        }
    }

    #[test]
    fn initialized_is_a_notification_with_no_reply() {
        let mut session = Session::default();
        let _ = handle(
            &init_frame(PROTOCOL_VERSION),
            &mut session,
            &ToolTable::empty(),
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &mut session,
                &ToolTable::empty(),
            ),
            None
        );
        assert!(
            session
                .refuse_if_premature("tools/list", &id_one())
                .is_none()
        );
    }

    /// `initialized` without a preceding `initialize` must not open the door:
    /// a client that skipped the handshake has agreed to no version.
    #[test]
    fn initialized_alone_does_not_make_a_session_ready() {
        let mut session = Session::default();
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &mut session,
                &ToolTable::empty(),
            ),
            None
        );
        assert!(
            session
                .refuse_if_premature("tools/list", &id_one())
                .is_some()
        );
    }

    /// Valid JSON that is not a request object. Silence here would leave a
    /// client waiting out its own timeout.
    #[test]
    fn a_frame_that_is_not_a_request_object_is_an_invalid_request() {
        for frame in [
            "[]",
            "5",
            "\"hi\"",
            "null",
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
        ] {
            let mut session = Session::ready_for_test();
            let v = reply(frame, &mut session);
            assert_eq!(v["error"]["code"], INVALID_REQUEST, "{frame}");
            assert!(v["id"].is_null(), "{frame}");
        }
    }

    /// A serializer that does not skip `None` writes `"id": null`. Answering
    /// it would break two rules at once: a notification must get no reply,
    /// and the one notification that matters must still be acted on.
    #[test]
    fn an_explicit_null_id_is_still_a_notification() {
        let mut session = Session::default();
        let _ = handle(
            &init_frame(PROTOCOL_VERSION),
            &mut session,
            &ToolTable::empty(),
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":null,"method":"notifications/initialized"}"#,
                &mut session,
                &ToolTable::empty(),
            ),
            None,
            "no reply to a notification"
        );
        assert!(
            session
                .refuse_if_premature("tools/list", &id_one())
                .is_none(),
            "and the handshake still completes"
        );
    }

    #[test]
    fn ping_answers_before_initialize() {
        let mut session = Session::default();
        let v = reply(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#, &mut session);
        assert_eq!(v["id"], 9);
        assert!(v["result"].is_object());
    }

    #[test]
    fn a_request_before_initialize_is_refused() {
        let mut session = Session::default();
        let v = reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &mut session,
        );
        assert_eq!(v["error"]["code"], NOT_INITIALIZED);
    }

    /// The middle lifecycle state: `initialize` answered, `initialized` not
    /// yet sent. Still too early for a request.
    #[test]
    fn a_request_between_initialize_and_initialized_is_refused() {
        let mut session = Session::default();
        let _ = handle(
            &init_frame(PROTOCOL_VERSION),
            &mut session,
            &ToolTable::empty(),
        );
        let v = reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &mut session,
        );
        assert_eq!(v["error"]["code"], NOT_INITIALIZED);
    }

    #[test]
    fn an_unknown_method_is_method_not_found() {
        let mut session = Session::ready_for_test();
        let v = reply(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#, &mut session);
        assert_eq!(v["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_json_is_a_parse_error_with_null_id() {
        let mut session = Session::ready_for_test();
        let v = reply("{not json", &mut session);
        assert_eq!(v["error"]["code"], PARSE_ERROR);
        assert!(v["id"].is_null());
    }

    /// The host answers `tools/call` itself, so it must be able to ask the
    /// same admission question — one gate, not two.
    #[test]
    fn the_admission_gate_is_askable_for_a_method_handle_never_sees() {
        let id = serde_json::json!(1);
        let mut session = Session::default();
        assert!(
            session.refuse_if_premature("tools/call", &id).is_some(),
            "premature before the handshake"
        );
        assert!(session.refuse_if_premature("ping", &id).is_none());
        assert!(session.refuse_if_premature("initialize", &id).is_none());

        let _ = handle(
            &init_frame(PROTOCOL_VERSION),
            &mut session,
            &ToolTable::empty(),
        );
        let _ = handle(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut session,
            &ToolTable::empty(),
        );
        assert!(session.refuse_if_premature("tools/call", &id).is_none());
    }

    /// The host answers `tools/call`; reaching the protocol layer means the
    /// interception is broken, and saying so beats pretending the tool ran.
    #[test]
    fn a_tool_call_that_reaches_the_protocol_layer_is_an_invalid_request() {
        let mut session = Session::ready_for_test();
        let v = reply(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"x"}}"#,
            &mut session,
        );
        assert_eq!(v["error"]["code"], INTERNAL_ERROR);
    }

    /// A cancellation is a notification, so `handle` answers nothing and the
    /// host has to recognise it separately.
    #[test]
    fn a_cancellation_names_the_request_it_withdraws() {
        let frame = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"user stopped"}}"#;
        assert_eq!(cancellation_target(frame).as_deref(), Some("7"));

        let mut session = Session::ready_for_test();
        assert_eq!(
            handle(frame, &mut session, &ToolTable::empty()),
            None,
            "and it is still a notification"
        );
    }

    /// A number and a string are different ids in JSON-RPC, so the keys they
    /// produce must differ — otherwise a cancellation could withdraw a call it
    /// does not name.
    #[test]
    fn a_numeric_id_and_its_string_spelling_are_different_keys() {
        assert_ne!(
            request_key(&serde_json::json!(1)),
            request_key(&serde_json::json!("1"))
        );
    }

    #[test]
    fn frames_that_are_not_cancellations_name_nothing() {
        for frame in [
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            // Cancelling without saying what: nothing to withdraw.
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":null}}"#,
            "not json",
        ] {
            assert_eq!(cancellation_target(frame), None, "{frame}");
        }
    }

    /// A string id is legal JSON-RPC and has to come back unchanged, or the
    /// client cannot match the reply to its request.
    #[test]
    fn a_string_id_round_trips() {
        let mut session = Session::ready_for_test();
        let v = reply(
            r#"{"jsonrpc":"2.0","id":"abc","method":"tools/list"}"#,
            &mut session,
        );
        assert_eq!(v["id"], "abc");
    }
}
