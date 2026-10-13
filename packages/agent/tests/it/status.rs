//! Tests for the public `daruda_agent::status` API.

use daruda_agent::status::*;

/// A prompt a live agent is blocked on outranks a dead session, which in
/// turn outranks a working one — both want the user, the prompt sooner.
#[test]
fn failed_sits_between_attention_and_work() {
    assert!(SessionStatus::NeedsAttention.priority() > SessionStatus::Failed.priority());
    assert!(SessionStatus::Failed.priority() > SessionStatus::Working.priority());
    assert_eq!(
        SessionStatus::aggregate([SessionStatus::Working, SessionStatus::Failed]),
        Some(SessionStatus::Failed)
    );
}

/// Only a still badge lets the status-pulse clock stop.
#[test]
fn idle_and_failed_are_the_still_statuses() {
    assert!(!SessionStatus::Idle.animates());
    assert!(!SessionStatus::Failed.animates());
    for moving in [
        SessionStatus::Working,
        SessionStatus::ExecutingTool,
        SessionStatus::NeedsAttention,
        SessionStatus::Connecting,
    ] {
        assert!(moving.animates(), "{moving:?}");
    }
}

#[test]
fn priority_ordering() {
    assert!(SessionStatus::NeedsAttention.priority() > SessionStatus::Working.priority());
    assert!(SessionStatus::NeedsAttention.priority() > SessionStatus::ExecutingTool.priority());
    assert_eq!(
        SessionStatus::Working.priority(),
        SessionStatus::ExecutingTool.priority()
    );
    assert!(SessionStatus::Working.priority() > SessionStatus::Idle.priority());
    assert!(SessionStatus::Idle.priority() > SessionStatus::Connecting.priority());
}

#[test]
fn aggregate_picks_highest_priority() {
    assert_eq!(
        SessionStatus::aggregate([
            SessionStatus::Idle,
            SessionStatus::Working,
            SessionStatus::Connecting,
        ]),
        Some(SessionStatus::Working)
    );
    assert_eq!(
        SessionStatus::aggregate([SessionStatus::ExecutingTool, SessionStatus::Idle]),
        Some(SessionStatus::ExecutingTool)
    );
}

#[test]
fn aggregate_of_nothing_is_none() {
    assert_eq!(SessionStatus::aggregate([]), None);
}

#[test]
fn json_roundtrip() {
    for s in [
        SessionStatus::Working,
        SessionStatus::ExecutingTool,
        SessionStatus::NeedsAttention,
        SessionStatus::Idle,
        SessionStatus::Connecting,
    ] {
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
