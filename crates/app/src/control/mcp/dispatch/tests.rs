use super::*;

fn call(id: i64, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
    .to_string()
}

use gpui::TestAppContext;

/// Drive one frame the way the socket task does, through a session that
/// has already handshaked.
fn answered(frame: &str, cx: &mut TestAppContext) -> serde_json::Value {
    let text = answer_on(frame, ConnectionSession::handshaked_for_test(1), 1, cx)
        .try_recv()
        .expect("answered now");
    serde_json::from_str(&text).expect("json")
}

/// Drive one frame the way the socket task does, returning the reply
/// channel so a deferred answer can be awaited.
fn answer_on(
    frame: &str,
    mut session: ConnectionSession,
    connection: u64,
    cx: &mut TestAppContext,
) -> smol::channel::Receiver<String> {
    let (tx, rx) = smol::channel::bounded(1);
    cx.update(|cx| {
        answer(
            Inbound {
                frame: frame.to_owned(),
                reply: tx,
                connection,
            },
            &mut session,
            crate::orchestrator::pane(cx),
            cx,
        );
    });
    rx
}

/// The payload the model reads, out of the one text block.
/// A configuration and a bridge that can actually deliver a card.
fn reachable_phone(cx: &mut App) {
    use gpui::BorrowAppContext as _;

    crate::settings_store::SettingsStore::init(cx);
    cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
        let mut cfg = (*store.user()).clone();
        cfg.telegram.enabled = true;
        cfg.telegram.authorized_chat_id = Some(42);
        store.set_user_for_testing(cfg);
    });
    let _outbound = crate::telegram::global::install_for_test(true, Some(42), cx);
}

fn cancellation(request_id: i64) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": { "requestId": request_id, "reason": "the turn was stopped" },
    })
    .to_string()
}

/// One frame into a session the caller keeps, for a test that spans two.
fn deliver(
    frame: &str,
    session: &mut ConnectionSession,
    cx: &mut TestAppContext,
) -> smol::channel::Receiver<String> {
    let (tx, rx) = smol::channel::bounded(1);
    cx.update(|cx| {
        answer(
            Inbound {
                frame: frame.to_owned(),
                reply: tx,
                connection: session.connection.max(1),
            },
            session,
            crate::orchestrator::pane(cx),
            cx,
        );
    });
    rx
}

fn payload(reply: &serde_json::Value) -> serde_json::Value {
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .expect("text block");
    serde_json::from_str(text).expect("json payload")
}

#[gpui::test]
async fn a_tool_call_reaches_the_control_core(cx: &mut TestAppContext) {
    let _fixture = crate::test_support::workspace_with_agent_chat(cx);
    let reply = answered(&call(1, "daruda_chat_list", serde_json::json!({})), cx);
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["isError"], false);
    let inner = payload(&reply);
    assert_eq!(inner["kind"], "listing");
    assert!(
        !inner["windows"].as_array().expect("windows").is_empty(),
        "the fixture's chat is listed: {inner}"
    );
}

/// A read of every worktree, including ones with no chat — the listing
/// `/list` answers with cannot name those.
#[gpui::test]
async fn lane_list_answers_from_the_same_frame_path(cx: &mut TestAppContext) {
    let _fixture = crate::test_support::workspace_with_agent_chat(cx);
    let reply = answered(&call(2, "daruda_worktree_list", serde_json::json!({})), cx);
    assert_eq!(reply["result"]["isError"], false);
    let inner = payload(&reply);
    assert_eq!(inner["kind"], "lane_listing");
    let lanes = inner["lanes"].as_array().expect("lanes");
    assert!(!lanes.is_empty());
    assert!(
        lanes[0]["target"]["workspace"].is_string(),
        "a handle carries its window: {inner}"
    );
}

/// The budget is not a decision the user gets to make, so it is answered
/// without a card ever reaching their phone.
#[gpui::test]
async fn a_gated_tool_refuses_when_the_budget_is_spent(cx: &mut TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(false, None, cx);
        for _ in 0..guards::AGENT_LANE_BUDGET {
            guards::note_agent_created_lane(cx);
        }
    });

    let reply = answered(
        &call(
            3,
            "daruda_worktree_create",
            serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
        ),
        cx,
    );
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(payload(&reply)["code"], "agent_limit_reached");
    cx.update(|cx| {
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            0,
            "refused before the card"
        );
    });
}

/// The deferred path, end to end: a gated call whose answer arrives later
/// still comes back on the *same* JSON-RPC id, with the outcome the gate
/// produced. Nothing else exercises `Answer::Later`'s success branch —
/// `answered()` asserts the reply is already there, which is
/// `Answer::Now` by construction.
///
/// The card is undeliverable (no bridge configured), which is the one way
/// to make a *real* gated call settle inside a test without a phone.
#[gpui::test]
async fn a_deferred_answer_comes_back_on_the_same_id(cx: &mut TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(false, None, cx);
    });

    let mut session = ConnectionSession::handshaked_for_test(1);
    let reply = deliver(
        &call(
            4242,
            "daruda_worktree_create",
            serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
        ),
        &mut session,
        cx,
    );
    assert!(
        reply.try_recv().is_err(),
        "the gate answers later, not inline"
    );

    cx.run_until_parked();
    let text = reply.try_recv().expect("the deferred reply arrives");
    let answer: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        answer["id"], 4242,
        "a reply on the wrong id is a reply the client cannot match: {answer}"
    );
    assert_eq!(answer["result"]["isError"], true);
    assert_eq!(payload(&answer)["code"], "approval_unavailable");
}

/// A `notifications/cancelled` for a call still waiting on a card: the
/// question comes down, the work never starts, and the client that freed
/// the request id gets no reply for it.
#[gpui::test]
async fn cancelling_a_waiting_call_withdraws_it_unanswered(cx: &mut TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    // Deliverable, so the card is really waiting rather than settling as
    // undeliverable before the cancellation arrives. Both halves matter:
    // the *config* is what `send_approval_card` reads, and the bridge
    // global is what holds the card's tokens.
    cx.update(reachable_phone);

    let mut session = ConnectionSession::handshaked_for_test(1);
    let reply = deliver(
        &call(
            7,
            "daruda_worktree_create",
            serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
        ),
        &mut session,
        cx,
    );
    cx.update(|cx| {
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            1,
            "the call is waiting on the phone"
        );
    });
    assert!(reply.try_recv().is_err(), "and nothing is answered yet");

    let _ = deliver(&cancellation(7), &mut session, cx);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            0,
            "the withdrawn question is off the phone"
        );
    });
    assert!(
        reply.try_recv().is_err(),
        "a withdrawn call is not answered — the client has freed its id"
    );
}

/// A cancellation naming a call that was never pending must be a no-op,
/// not a panic and not a withdrawal of whatever else is waiting.
#[gpui::test]
async fn cancelling_something_unknown_leaves_the_waiting_call_alone(cx: &mut TestAppContext) {
    let fixture = crate::test_support::workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    cx.update(reachable_phone);

    let mut session = ConnectionSession::handshaked_for_test(1);
    let _reply = deliver(
        &call(
            7,
            "daruda_worktree_create",
            serde_json::json!({ "workspace": workspace, "project": project, "name": "x" }),
        ),
        &mut session,
        cx,
    );
    // A different id, and the string spelling of the same one: neither
    // names the call that is waiting.
    let _ = deliver(&cancellation(8), &mut session, cx);
    let _ = deliver(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": "7" },
        })
        .to_string(),
        &mut session,
        cx,
    );
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            1,
            "the waiting call is untouched"
        );
    });
}

/// MCP wants an unknown tool reported as a *tool* error so the model can
/// recover by listing again, not as a transport failure it may never see.
#[gpui::test]
async fn an_unknown_tool_name_is_an_error_result_not_a_protocol_error(cx: &mut TestAppContext) {
    let _fixture = crate::test_support::workspace_with_agent_chat(cx);
    let reply = answered(&call(4, "daruda_nope", serde_json::json!({})), cx);
    assert!(reply.get("error").is_none(), "not a protocol error");
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(payload(&reply)["code"], "unknown_tool");
    assert_eq!(payload(&reply)["tool"], "daruda_nope");
}

/// Bad arguments name the argument, so the model can fix them.
#[gpui::test]
async fn bad_arguments_are_named_in_the_tool_error(cx: &mut TestAppContext) {
    let _fixture = crate::test_support::workspace_with_agent_chat(cx);
    let reply = answered(&call(5, "daruda_chat_send", serde_json::json!({})), cx);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(payload(&reply)["code"], "missing_argument");
    assert_eq!(payload(&reply)["argument"], "target");
}

/// The orchestrator prompting itself is the one pure software loop left,
/// and it is refused at the frame boundary.
#[gpui::test]
async fn a_self_addressed_prompt_is_refused(cx: &mut TestAppContext) {
    let own = crate::test_support::register_test_orchestrator(cx);
    let reply = answered(
        &call(
            6,
            "daruda_chat_send",
            serde_json::json!({ "target": own, "text": "go" }),
        ),
        cx,
    );
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(payload(&reply)["code"], "self_target_refused");
}

/// A stop is how a runaway is *ended*, so it must not be refused for
/// naming the orchestrator.
#[gpui::test]
async fn stopping_the_orchestrator_is_allowed(cx: &mut TestAppContext) {
    let own = crate::test_support::register_test_orchestrator(cx);
    let reply = answered(
        &call(7, "daruda_chat_stop", serde_json::json!({ "target": own })),
        cx,
    );
    assert_eq!(
        reply["result"]["isError"], false,
        "a stop is the way out of a loop: {reply}"
    );
}

/// A frame that is not a call still gets its protocol answer through this
/// same entry point — the host must not swallow `tools/list`.
#[gpui::test]
async fn a_non_call_frame_falls_through_to_the_protocol_layer(cx: &mut TestAppContext) {
    let reply = answered(r#"{"jsonrpc":"2.0","id":8,"method":"tools/list"}"#, cx);
    assert_eq!(reply["id"], 8);
    // Against the table, not a literal: this test is about the frame reaching
    // the protocol layer at all. How many tools there are is
    // `tools::tests`'s to assert, and one owner means one place to update.
    assert_eq!(
        reply["result"]["tools"].as_array().expect("tools").len(),
        ToolTable::all().describe().len(),
    );
}

/// A premature call is refused, and by the same predicate `handle` uses.
#[gpui::test]
async fn a_call_before_the_handshake_is_refused(cx: &mut TestAppContext) {
    let text = answer_on(
        &call(9, "daruda_status", serde_json::json!({})),
        ConnectionSession::default(),
        1,
        cx,
    )
    .try_recv()
    .expect("answered now");
    let reply: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(reply["error"]["code"], protocol::NOT_INITIALIZED);
}

/// The socket serves connections one at a time but *sequentially*, so a
/// reconnect must start over — otherwise shim two inherits shim one's
/// handshake and can call a tool having never sent `initialize`.
#[gpui::test]
async fn a_second_connection_does_not_inherit_the_first_handshake(cx: &mut TestAppContext) {
    let mut session = ConnectionSession::handshaked_for_test(1);
    let text = answer_on(
        &call(1, "daruda_status", serde_json::json!({})),
        std::mem::take(&mut session),
        2,
        cx,
    )
    .try_recv()
    .expect("answered now");
    let reply: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        reply["error"]["code"],
        protocol::NOT_INITIALIZED,
        "connection two starts over: {reply}"
    );
}

#[test]
fn a_tool_call_is_recognised_and_its_parts_extracted() {
    let parsed = ToolCall::parse(&call(1, "daruda_chat_list", serde_json::json!({"a": 1})))
        .expect("a tools/call");
    assert_eq!(parsed.id, 1);
    assert_eq!(parsed.name.as_deref(), Ok("daruda_chat_list"));
    assert_eq!(parsed.arguments["a"], 1);
}

/// Missing `params` is not a parse failure — it is a call with no
/// arguments, which four of the nine tools legitimately are. The *name*
/// is a different matter: there is no tool without one.
#[test]
fn a_call_without_params_reads_as_no_arguments_and_no_name() {
    let frame = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
    let parsed = ToolCall::parse(frame).expect("a tools/call");
    assert_eq!(
        parsed.name,
        Err(convert::ConvertError::MissingArgument { name: "name" })
    );
    assert_eq!(parsed.arguments, serde_json::json!({}));
}

/// A call that named no tool, or named one with something that is not a
/// string, must not be reported as a tool called "" that does not exist —
/// the model would go looking for a name it never sent.
#[gpui::test]
async fn a_call_with_no_usable_name_says_so(cx: &mut TestAppContext) {
    let _fixture = crate::test_support::workspace_with_agent_chat(cx);
    for (frame, code, expected) in [
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
            "missing_argument",
            "name",
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":7}}"#,
            "bad_argument",
            "name",
        ),
    ] {
        let reply = answered(frame, cx);
        assert_eq!(reply["result"]["isError"], true, "{frame}");
        assert_eq!(payload(&reply)["code"], code, "{frame}");
        assert_eq!(payload(&reply)["argument"], expected, "{frame}");
    }
}

#[test]
fn another_method_is_left_to_the_protocol_layer() {
    for frame in [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        "{not json",
        "[]",
    ] {
        assert!(ToolCall::parse(frame).is_none(), "{frame}");
    }
}

/// A call with no id cannot be answered, so it is not run: a tool whose
/// result nobody hears is all effect and no report.
#[test]
fn a_call_with_no_id_is_not_treated_as_a_call() {
    for frame in [
        r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"daruda_status"}}"#,
        r#"{"jsonrpc":"2.0","id":null,"method":"tools/call","params":{"name":"daruda_status"}}"#,
    ] {
        assert!(ToolCall::parse(frame).is_none(), "{frame}");
    }
}
