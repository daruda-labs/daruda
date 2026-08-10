//! One turn, end to end: which stop reasons pass, which fail, and how a
//! session that never opened is told apart from a turn that went wrong.

use super::*;

/// One turn against an adapter that answers `reply` to `session/prompt`.
fn turn_answered_with(reply: &str) -> RunResult {
    let fixture = Fixture::with_script(&adapter_script("", reply));
    fixture.run(&spec(AGENT))
}

/// The file contract is the scheduler's, but the turn's shape is the
/// runner's: a clean `EndTurn` is the only stop reason that passes.
#[test]
fn a_turn_that_ends_cleanly_passes() {
    let result = turn_answered_with(&stops_with("end_turn"));
    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
}

/// §6's core correction: `daruda_acp` calls these "completed normally",
/// and taking that at face value lets a node that wrote half its output
/// flow downstream as a pass.
#[test]
fn the_three_stop_reasons_that_are_not_success_each_map_to_their_own_failure() {
    for (wire, expected) in [
        ("max_tokens", NodeFailure::ContextExhausted),
        ("max_turn_requests", NodeFailure::TurnLimit),
        ("refusal", NodeFailure::Refused),
    ] {
        let result = turn_answered_with(&stops_with(wire));
        assert_eq!(result.outcome, Err(expected), "stop reason `{wire}`");
    }
}

/// An unfamiliar stop reason is a failure, not a pass. A new one appearing
/// in a future adapter must stop the run loudly rather than let a node
/// through on a reason nobody has read.
///
/// Pure rather than end-to-end: the schema this crate builds against
/// models `StopReason` as a closed enum, so an unknown value never
/// survives deserialization to reach `TurnEnded` today. The rule is here
/// for the schema version that adds an `Other(String)` variant.
#[test]
fn an_unknown_stop_reason_fails_rather_than_passing() {
    assert_eq!(failure_for("EndTurn"), None);
    for unknown in ["WarpDrive", "Cancelled", "", "end_turn"] {
        assert!(
            failure_for(unknown).is_some(),
            "`{unknown}` must not read as a pass"
        );
    }
}

/// The same rule from the wire side, at today's schema: a stop reason the
/// protocol cannot even parse must still not reach the scheduler as a
/// pass.
#[test]
fn a_stop_reason_the_protocol_cannot_parse_does_not_pass_either() {
    let result = turn_answered_with(&stops_with("warp_drive"));
    assert!(result.outcome.is_err(), "{:?}", result.outcome);
}

/// A turn-level error keeps the session alive in `daruda_acp`, but for a
/// node it is simply a failed attempt — the scheduler's retry decides what
/// happens next.
#[test]
fn a_turn_error_is_a_node_failure_not_a_session_failure() {
    let result =
        turn_answered_with(r#""error":{"code":-32603,"message":"the session limit was reached"}"#);
    let Err(NodeFailure::TurnFailed(message)) = &result.outcome else {
        panic!("expected a turn failure, got {:?}", result.outcome);
    };
    assert!(message.contains("session limit"), "{message}");
}

/// A connection that never comes up cannot be retried into existence by
/// re-prompting, so it has to reach the node as its own failure.
#[test]
fn a_connection_failure_reports_a_session_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-adapter");
    let fixture = Fixture::with_command(dir, missing.display().to_string());

    let result = fixture.run(&spec(AGENT));
    assert!(
        matches!(result.outcome, Err(NodeFailure::SessionError(_))),
        "{:?}",
        result.outcome
    );
}

/// The agent's own accounting is the only cumulative cost there is, so the
/// runner has to carry it or the run's cost budget is blind.
#[test]
fn reported_usage_reaches_the_run_result() {
    let usage = r#"printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":1234,"size":200000,"cost":{"amount":0.25,"currency":"USD"}}}}\n'"#;
    let fixture = Fixture::with_script(&adapter_script(usage, &stops_with("end_turn")));

    let result = fixture.run(&spec(AGENT));
    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(
        result.usage,
        Some(UsageView {
            used: 1234,
            size: 200_000,
            cost: Some(CostView {
                amount: 0.25,
                currency: "USD".to_string(),
            }),
        })
    );
}

/// `validate_request` rejects this at submission, so reaching it means the
/// host assembled a request by hand — a node failure, not a panic.
#[test]
fn an_agent_absent_from_the_catalog_fails_the_node() {
    let fixture = Fixture::with_script(&adapter_script("", &stops_with("end_turn")));

    let result = fixture.run(&spec("codex"));
    let Err(NodeFailure::SessionError(message)) = &result.outcome else {
        panic!("expected a session error, got {:?}", result.outcome);
    };
    assert!(message.contains("codex"), "{message}");
}

/// The debt this closes: an agent node left no artifact, so a repair's
/// `{{attempts}}` pointed at the failed node's output and nothing else,
/// and a cancelled turn left no trace of what it had said.
#[test]
fn a_turn_leaves_a_transcript_for_the_scheduler_to_archive() {
    let fixture = Fixture::with_script(&adapter_script("", &stops_with("end_turn")));
    let result = fixture.run(&spec(AGENT));

    let transcript = result
        .artifacts
        .first()
        .unwrap_or_else(|| panic!("no artifact: {:?}", result.artifacts));
    let text = std::fs::read_to_string(transcript).expect("the transcript is on disk");
    assert!(text.contains("# Prompt"), "{text}");
    assert!(text.contains("# Ended"), "{text}");
}
