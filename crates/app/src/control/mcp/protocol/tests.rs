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
