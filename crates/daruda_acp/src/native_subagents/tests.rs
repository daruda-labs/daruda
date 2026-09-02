use serde_json::{Value, json};

use super::*;
use crate::adapter::adapter_for;

const ROOT: &str = "root-session";
const CHILD: &str = "child-session";

fn spawned(child: &str, name: &str, task: &str) -> Value {
    json!({
        "sessionUpdate": "subagent_spawned",
        "subagentSessionId": child,
        "name": name,
        "task": task,
        "capabilities": {},
    })
}

fn state(child: &str, state: &str) -> Value {
    json!({
        "sessionUpdate": "subagent_state_update",
        "subagentSessionId": child,
        "state": state,
    })
}

fn tool_call(id: &str, title: &str) -> Value {
    json!({ "sessionUpdate": "tool_call", "toolCallId": id, "title": title, "kind": "read" })
}

fn tool_call_update(id: &str, status: &str) -> Value {
    json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": status })
}

fn answer(text: &str, message_id: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": text },
        "messageId": message_id,
    })
}

/// The updates a route produced, or an empty list for anything else.
fn normalized(routed: Routed) -> Vec<SessionUpdate> {
    match routed {
        Routed::Normalized(updates) => updates,
        _ => Vec::new(),
    }
}

/// The one call a route was expected to produce.
fn one_call(routed: Routed) -> ToolCall {
    match normalized(routed).into_iter().next() {
        Some(SessionUpdate::ToolCall(call)) => call,
        other => panic!("expected exactly one tool call, got {other:?}"),
    }
}

fn one_update(routed: Routed) -> ToolCallUpdate {
    match normalized(routed).into_iter().next() {
        Some(SessionUpdate::ToolCallUpdate(u)) => u,
        other => panic!("expected exactly one tool call update, got {other:?}"),
    }
}

/// The parent link the adapter reads back off a normalized update.
fn parent_of(meta: &Option<Meta>) -> Option<String> {
    DefaultAdapter.parent_tool_id(meta)
}

fn router() -> NativeSubagentRouter {
    NativeSubagentRouter::default()
}

#[test]
fn a_root_update_passes_through_untouched() {
    let mut r = router();
    let call = match r.route(ROOT, &tool_call("toolu_1", "Read")) {
        Routed::Standard(u) => match *u {
            SessionUpdate::ToolCall(call) => call,
            other => panic!("expected a tool call, got {other:?}"),
        },
        _ => panic!("a root tool call is standard traffic"),
    };
    assert_eq!(&*call.tool_call_id.0, "toolu_1");
    assert_eq!(call.meta, None, "nothing is stamped on root traffic");
}

#[test]
fn a_spawn_makes_one_synthetic_launch_card() {
    let mut r = router();
    let call = one_call(r.route(ROOT, &spawned(CHILD, "Lorentz", "Probe the UI")));

    assert_eq!(&*call.tool_call_id.0, "subagent:child-session");
    assert_eq!(call.status, ToolCallStatus::InProgress);
    assert_eq!(call.kind, ToolKind::Think);
    // These two fields are the whole contract with the renderer's
    // `is_subagent_launch` / `subagent_type` / `subagent_prompt`.
    let raw = call.raw_input.expect("a launch carries its input");
    assert_eq!(raw["subagent_type"], json!("Lorentz"));
    assert_eq!(raw["prompt"], json!("Probe the UI"));
    assert_eq!(
        parent_of(&call.meta),
        None,
        "a root child owns a top-level row"
    );
}

#[test]
fn a_spawn_without_a_name_still_reads_as_a_launch() {
    let mut r = router();
    let call = one_call(r.route(
        ROOT,
        &json!({
            "sessionUpdate": "subagent_spawned",
            "subagentSessionId": CHILD,
            "task": "Do it",
        }),
    ));
    assert_eq!(
        call.raw_input.expect("input")["subagent_type"],
        json!("Agent")
    );
}

#[test]
fn every_child_tool_points_at_the_same_launch() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "Probe"));

    let parents: Vec<Option<String>> = (0..7)
        .map(|i| {
            let call = one_call(r.route(CHILD, &tool_call(&format!("call_{i}"), "Read")));
            parent_of(&call.meta)
        })
        .collect();

    assert_eq!(parents.len(), 7);
    assert!(
        parents
            .iter()
            .all(|p| p.as_deref() == Some("subagent:child-session")),
        "all seven children name the one launch: {parents:?}"
    );
}

#[test]
fn a_child_tool_id_is_namespaced_by_its_session() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "Probe"));
    let call = one_call(r.route(CHILD, &tool_call("call_1", "Read")));
    assert_eq!(
        &*call.tool_call_id.0, "subagent:child-session:tool:call_1",
        "the raw id alone would collide across sessions"
    );
}

#[test]
fn a_child_update_reaches_the_same_namespaced_call() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "Probe"));
    r.route(CHILD, &tool_call("call_1", "Read"));

    let update = one_update(r.route(CHILD, &tool_call_update("call_1", "completed")));
    assert_eq!(
        &*update.tool_call_id.0,
        "subagent:child-session:tool:call_1"
    );
    assert_eq!(update.fields.status, Some(ToolCallStatus::Completed));
    assert_eq!(
        parent_of(&update.meta).as_deref(),
        Some("subagent:child-session")
    );
}

#[test]
fn the_same_raw_tool_id_in_two_children_does_not_collide() {
    let mut r = router();
    r.route(ROOT, &spawned("child-a", "A", "t"));
    r.route(ROOT, &spawned("child-b", "B", "t"));

    let a = one_call(r.route("child-a", &tool_call("call_1", "Read")));
    let b = one_call(r.route("child-b", &tool_call("call_1", "Read")));

    assert_ne!(a.tool_call_id.0, b.tool_call_id.0);
    assert_eq!(parent_of(&a.meta).as_deref(), Some("subagent:child-a"));
    assert_eq!(parent_of(&b.meta).as_deref(), Some("subagent:child-b"));
}

#[test]
fn a_nested_spawn_names_its_parent_child_not_the_root() {
    let mut r = router();
    r.route(ROOT, &spawned("outer", "Outer", "t"));
    // The inner spawn is announced on the *outer child's* stream.
    let inner = one_call(r.route("outer", &spawned("inner", "Inner", "t")));

    assert_eq!(&*inner.tool_call_id.0, "subagent:inner");
    assert_eq!(
        parent_of(&inner.meta).as_deref(),
        Some("subagent:outer"),
        "a nested launch nests inside the card that spawned it"
    );
}

#[test]
fn a_regeneration_earns_its_own_card() {
    let mut r = router();
    r.route(ROOT, &spawned("thread-1", "Lorentz", "t"));
    let second = one_call(r.route(ROOT, &spawned("thread-1:generation:2", "Lorentz", "t")));

    assert_eq!(&*second.tool_call_id.0, "subagent:thread-1:generation:2");
    assert_ne!(
        r.tool_id_of("thread-1"),
        r.tool_id_of("thread-1:generation:2")
    );
}

#[test]
fn a_repeated_spawn_of_one_session_is_ignored() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    assert!(
        normalized(r.route(ROOT, &spawned(CHILD, "Lorentz", "t"))).is_empty(),
        "re-announcing a live child must not make a second card"
    );
}

#[test]
fn an_unannounced_session_is_ordinary_traffic_not_a_guessed_subagent() {
    // A tap appends every session of a process run to one file, so a session id
    // this router has not been told about is far more likely to be a second
    // conversation than an unannounced child. Claiming it as a subagent would
    // nest that conversation inside an invented card and swallow its messages.
    let mut r = router();
    assert!(matches!(
        r.route("some-other-session", &tool_call("toolu_1", "Read")),
        Routed::Standard(_)
    ));
}

#[test]
fn an_answer_is_folded_into_the_launch_on_completion() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    assert!(normalized(r.route(CHILD, &answer("Found ", "m1"))).is_empty());
    assert!(normalized(r.route(CHILD, &answer("7 calls.", "m1"))).is_empty());

    let update = one_update(r.route(ROOT, &state(CHILD, "completed")));
    assert_eq!(update.fields.status, Some(ToolCallStatus::Completed));
    let content = update.fields.content.expect("the answer rides along");
    assert_eq!(content.len(), 1, "one message is one block");
    match &content[0] {
        ToolCallContent::Content(c) => match &c.content {
            ContentBlock::Text(t) => assert_eq!(t.text, "Found 7 calls."),
            other => panic!("expected text, got {other:?}"),
        },
        other => panic!("expected content, got {other:?}"),
    }
}

#[test]
fn a_new_message_id_starts_a_new_block() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    r.route(CHILD, &answer("first", "m1"));
    r.route(CHILD, &answer("second", "m2"));

    let update = one_update(r.route(ROOT, &state(CHILD, "completed")));
    assert_eq!(update.fields.content.expect("content").len(), 2);
}

#[test]
fn commentary_is_left_out_of_the_answer() {
    let mut r = NativeSubagentRouter::new(adapter_for(Some("codex-acp"), ""));
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    r.route(
        CHILD,
        &json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "I'll start by reading." },
            "_meta": { "codex": { "phase": "commentary" } },
        }),
    );
    r.route(CHILD, &answer("The result.", "m1"));

    let update = one_update(r.route(ROOT, &state(CHILD, "completed")));
    let content = update.fields.content.expect("content");
    assert_eq!(content.len(), 1, "only the answer survives: {content:?}");
}

#[test]
fn a_child_thought_never_reaches_the_transcript() {
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    let out = normalized(r.route(
        CHILD,
        &json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "hmm" },
        }),
    ));
    assert!(out.is_empty(), "the child's reasoning is not the answer");
}

#[test]
fn every_terminal_state_maps_to_a_settled_status() {
    for (state_name, expected) in [
        ("completed", ToolCallStatus::Completed),
        ("failed", ToolCallStatus::Failed),
        ("cancelled", ToolCallStatus::Failed),
        ("disconnected", ToolCallStatus::Failed),
    ] {
        let mut r = router();
        r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
        let update = one_update(r.route(ROOT, &state(CHILD, state_name)));
        assert_eq!(
            update.fields.status,
            Some(expected),
            "state {state_name} must settle the card"
        );
    }
}

#[test]
fn a_terminal_report_for_an_unknown_child_is_dropped() {
    let mut r = router();
    assert!(normalized(r.route(ROOT, &state("never-seen", "completed"))).is_empty());
}

#[test]
fn a_malformed_native_payload_yields_nothing_rather_than_panicking() {
    let mut r = router();
    assert!(
        normalized(r.route(
            ROOT,
            &json!({ "sessionUpdate": "subagent_spawned", "name": 7 })
        ))
        .is_empty(),
        "a spawn with no child session id has nothing to register"
    );
    assert!(
        normalized(r.route(ROOT, &json!({ "sessionUpdate": "subagent_state_update" }))).is_empty()
    );
}

#[test]
fn an_unknown_update_kind_is_reported_once() {
    let mut r = router();
    let first = r.route(ROOT, &json!({ "sessionUpdate": "quantum_update" }));
    assert!(
        matches!(&first, Routed::Unknown { kind } if kind == "quantum_update"),
        "the first sighting names the kind"
    );
    assert!(
        matches!(
            r.route(ROOT, &json!({ "sessionUpdate": "quantum_update" })),
            Routed::Normalized(u) if u.is_empty()
        ),
        "a chatty unknown must not flood the host"
    );
}

#[test]
fn a_child_that_has_reported_is_still_a_child() {
    // `is_child` is what tells a capture's session count that a subagent's
    // session belongs to the conversation that spawned it, and a finished
    // subagent must not stop counting.
    let mut r = router();
    r.route(ROOT, &spawned(CHILD, "Lorentz", "t"));
    r.route(ROOT, &state(CHILD, "completed"));
    assert!(r.is_child(CHILD));
    assert!(!r.is_child(ROOT));
}

/// Codex marks its legacy delegation on the update itself, so the router
/// can say the subagent's calls will never arrive without knowing anything
/// about how the agent is configured. Once per session: one run emits
/// `spawnAgent`, `wait` and `closeAgent`, and three copies of the same
/// notice would be noise.
#[test]
fn legacy_delegation_is_reported_once_and_only_for_the_spawn() {
    let mut router = router();
    let spawn = serde_json::json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t1",
        "title": "spawnAgent",
        "_meta": {"codex": {"collaboration": {"tool": "spawnAgent"}}},
    });
    let wait = serde_json::json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t2",
        "title": "wait",
        "_meta": {"codex": {"collaboration": {"tool": "wait"}}},
    });
    let plain = serde_json::json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t3",
        "title": "Read main.rs",
    });

    assert!(router.first_legacy_delegation(&spawn));
    assert!(!router.first_legacy_delegation(&spawn), "reported once");
    assert!(
        !router.first_legacy_delegation(&wait),
        "only the spawn marks it"
    );
    assert!(
        !router.first_legacy_delegation(&plain),
        "native traffic is silent"
    );
}

/// A native run must not trip the legacy notice — that is the whole point
/// of keying off the collaboration marker rather than off "a subagent
/// appeared".
#[test]
fn a_native_spawn_is_not_reported_as_legacy() {
    let mut router = router();
    assert!(!router.first_legacy_delegation(&spawned("kid", "Lorentz", "Probe the UI")));
}
