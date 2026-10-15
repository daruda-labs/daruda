use super::*;
use serde_json::json;

#[test]
fn compaction_capability_is_host_opt_in() {
    for compaction in [false, true] {
        let caps = serde_json::to_value(client_capabilities(ClientCapabilitiesView {
            compaction,
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(caps["session"]["compaction"].is_object(), compaction);
        assert!(caps["session"].get("notices").is_none());
        assert!(caps.get("subagents").is_none());
    }
}

#[test]
fn child_compaction_keeps_wire_patches_and_its_originating_session() {
    let mut router = NativeSubagentRouter::default();
    router.route(
        "root",
        &json!({"sessionUpdate":"subagent_spawned", "subagentSessionId":"child"}),
    );
    let (tx, mut rx) = unbounded();
    let modes = ModeTracker::default();
    for session in ["root", "child"] {
        for value in [
            json!({"sessionUpdate":"compaction_summary_chunk", "compactionId":"c", "content":{"type":"text", "text":"summary"}}),
            json!({"sessionUpdate":"compaction_update", "compactionId":"c", "status":"completed", "error":null}),
        ] {
            let updates = match router.route(session, &value) {
                Routed::Standard(update) => vec![*update],
                Routed::Normalized(updates) => updates,
                Routed::Unknown { .. } => panic!("known compaction"),
            };
            assert_eq!(updates.len(), 1);
            forward_session_update(
                &SessionId::from(session),
                updates.into_iter().next().unwrap(),
                None,
                &modes,
                &tx,
            );
            let AcpEvent::Compaction { session_id, update } = rx.try_recv().unwrap() else {
                panic!("compaction event")
            };
            assert_eq!(session_id, session);
            assert_eq!(serde_json::to_value(*update).unwrap(), value);
        }
    }
}
