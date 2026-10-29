use super::*;

#[test]
fn unknown_account_is_distinct_from_explicit_system_account() {
    let binding = SessionBinding {
        node: "hello".into(),
        attempt: 1,
        evidence: 1,
        agent: "claude".into(),
        account: None,
        cwd: PathBuf::from("."),
        session_id: "saved".into(),
    };
    let mut system = binding.clone();
    system.account = Some(SessionAccount { managed: None });
    assert_ne!(
        serde_json::to_value(binding).unwrap(),
        serde_json::to_value(system).unwrap()
    );
}

#[test]
fn binding_is_durable_and_cannot_be_reassigned_to_another_session() {
    let dir = tempfile::tempdir().unwrap();
    let node = NodeId::from("hello");
    let store = crate::run_log::testing::attempt_in_flight(dir.path(), &node);
    let port = SessionPort {
        store: &store,
        node: &node,
        attempt: 1,
        evidence: 1,
    };
    smol::block_on(port.record(
        "claude",
        Some(SessionAccount { managed: None }),
        dir.path(),
        "exact-session".into(),
    ))
    .unwrap();
    let inspected = crate::run_log::reader::inspect(dir.path())
        .unwrap()
        .unwrap();
    let binding = inspected.state.sessions().next().unwrap();
    assert_eq!(binding.session_id, "exact-session");
    assert_eq!(binding.evidence, 1);
    assert!(smol::block_on(port.record("claude", None, dir.path(), "replacement".into())).is_err());
    let inspected = crate::run_log::reader::inspect(dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(inspected.state.sessions().count(), 1);
    assert_eq!(
        inspected.state.sessions().next().unwrap().session_id,
        "exact-session"
    );
}

#[test]
fn an_unassigned_evidence_cannot_claim_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let node = NodeId::from("hello");
    let store = crate::run_log::testing::attempt_in_flight(dir.path(), &node);
    let port = SessionPort {
        store: &store,
        node: &node,
        attempt: 1,
        evidence: 2,
    };
    assert!(smol::block_on(port.record("claude", None, dir.path(), "wrong".into())).is_err());
    assert_eq!(
        crate::run_log::reader::inspect(dir.path())
            .unwrap()
            .unwrap()
            .state
            .sessions()
            .count(),
        0
    );
}
