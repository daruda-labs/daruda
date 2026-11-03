//! Tests for the public `daruda_agent::hooks::turn_end` API.

use SessionStatus as S;
use daruda_agent::hooks::turn_end::*;
use daruda_agent::{AgentOutcome, SessionStatus};

fn opened(event: &str, status: SessionStatus) -> OpenTurns {
    let mut turns = OpenTurns::default();
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            status,
            event
        ),
        None
    );
    turns
}

#[test]
fn a_stop_ends_a_turn_the_channel_saw_open() {
    for (event, status) in [
        ("UserPromptSubmit", S::Working),
        ("PreToolUse", S::ExecutingTool),
        ("PermissionRequest", S::NeedsAttention),
    ] {
        let mut turns = opened(event, status);
        assert_eq!(
            turns.observe(
                &daruda_agent::StatusKey::LegacySession("s".into()),
                S::Idle,
                "Stop"
            ),
            Some(AgentOutcome::Completed),
            "{event}"
        );
    }
}

#[test]
fn a_stop_failure_errors_the_turn() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "StopFailure"
        ),
        Some(AgentOutcome::Errored)
    );
}

/// The watcher hands the same file over again; startup re-reads every
/// file. Neither is a turn ending now.
#[test]
fn a_stop_with_no_open_turn_ends_nothing() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    turns.observe(
        &daruda_agent::StatusKey::LegacySession("s".into()),
        S::Idle,
        "Stop",
    );
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "Stop"
        ),
        None,
        "re-delivered"
    );
    assert_eq!(
        OpenTurns::default().observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "Stop"
        ),
        None,
        "never seen open"
    );
}

#[test]
fn events_inside_a_turn_keep_it_open() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    for (event, status) in [
        ("PreToolUse", S::ExecutingTool),
        ("PostToolUse", S::Working),
        ("Notification", S::Working),
    ] {
        assert_eq!(
            turns.observe(
                &daruda_agent::StatusKey::LegacySession("s".into()),
                status,
                event
            ),
            None
        );
    }
    assert!(
        turns
            .observe(
                &daruda_agent::StatusKey::LegacySession("s".into()),
                S::Idle,
                "Stop"
            )
            .is_some()
    );
}

/// A fresh `SessionStart` means whatever turn was open is gone.
#[test]
fn a_resting_update_closes_the_turn_without_an_outcome() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "SessionStart"
        ),
        None
    );
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "Stop"
        ),
        None
    );
}

#[test]
fn forgetting_a_session_closes_its_turn() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    turns.forget(&daruda_agent::StatusKey::LegacySession("s".into()));
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("s".into()),
            S::Idle,
            "Stop"
        ),
        None
    );
}

#[test]
fn sessions_are_tracked_apart() {
    let mut turns = opened("UserPromptSubmit", S::Working);
    assert_eq!(
        turns.observe(
            &daruda_agent::StatusKey::LegacySession("other".into()),
            S::Idle,
            "Stop"
        ),
        None
    );
    assert!(
        turns
            .observe(
                &daruda_agent::StatusKey::LegacySession("s".into()),
                S::Idle,
                "Stop"
            )
            .is_some()
    );
}

#[test]
fn legacy_and_routed_turns_with_the_same_text_complete_independently() {
    let legacy = daruda_agent::StatusKey::LegacySession("terminal-a".into());
    let routed = daruda_agent::StatusKey::Terminal(
        daruda_agent::hooks::route::TerminalId::parse("terminal-a").unwrap(),
    );
    let mut turns = OpenTurns::default();
    assert_eq!(turns.observe(&legacy, S::Working, "UserPromptSubmit"), None);
    assert_eq!(turns.observe(&routed, S::Working, "UserPromptSubmit"), None);
    assert_eq!(
        turns.observe(&legacy, S::Idle, "Stop"),
        Some(AgentOutcome::Completed)
    );
    assert_eq!(
        turns.observe(&routed, S::Idle, "Stop"),
        Some(AgentOutcome::Completed)
    );
}
