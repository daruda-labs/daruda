//! A node whose session starts billing extra usage stops the whole run
//! through its cancel token — when, and only when, the host enabled it.

use super::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A `usage_update` carrying the wire-captured `_meta` of a reply billed as
/// overage, sent before the turn's reply.
const BILLED_USAGE: &str = r#"printf '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"usage_update","used":10,"size":200000,"_meta":{"_claude/rateLimit":{"status":"rejected","resetsAt":1791340200,"rateLimitType":"five_hour","overageStatus":"allowed","isUsingOverage":true}}}}}\n'"#;

/// An adapter that reports the billed usage mid-turn, then holds the turn
/// until `session/cancel` arrives and answers it — the shape the interrupt
/// path is for.
fn billed_then_parked() -> String {
    format!(
        r#"while IFS= read -r line; do
  id=${{line#*\"id\":\"}}; id=${{id%%\"*}}
  case "$line" in
{INITIALIZE}
{NEW_SESSION}
*'"method":"session/prompt"'*)
  prompt_id="$id"
  {BILLED_USAGE} ;;
*'"method":"session/cancel"'*)
  {ANSWERS_THE_CANCEL} ;;
  esac
done
"#
    )
}

fn with_switch(script: &str, enabled: bool) -> Fixture {
    let mut fixture = Fixture::with_script(script);
    fixture.overage = OverageStop::new(Arc::new(AtomicBool::new(enabled)));
    fixture
}

#[test]
fn an_enabled_switch_cancels_the_run_on_a_billed_reply() {
    let fixture = with_switch(&billed_then_parked(), true);
    let result = fixture.run(&spec(AGENT));

    assert!(
        fixture.cancel.is_canceled(),
        "the run-wide token is tripped"
    );
    assert_eq!(
        result.outcome,
        Err(NodeFailure::SessionError(CANCELED.to_string()))
    );
    assert!(result.stopped_the_run, "this call's report stopped the run");
    let limit = fixture
        .overage
        .tripped()
        .expect("the billed report is kept");
    assert_eq!(limit.resets_at, Some(1_791_340_200));
    assert_eq!(limit.window, Some(daruda_acp::RateLimitWindow::FiveHour));

    let transcript = std::fs::read_to_string(&result.artifacts[0]).expect("transcript");
    assert!(
        transcript.contains("stopped: Claude would bill extra usage"),
        "{transcript}"
    );
}

/// The report usually lands just before the turn ends, so the turn can win
/// the race against the interrupt. Either way the token is tripped and the
/// result says this call did it — which is what lets the scheduler keep a
/// turn that won (`schedule/tests/cancel.rs`, the `own_overage` tests).
#[test]
fn a_turn_that_ends_with_the_billed_report_still_trips_the_token() {
    let script = adapter_script(BILLED_USAGE, &stops_with("end_turn"));
    let fixture = with_switch(&script, true);
    let result = fixture.run(&spec(AGENT));

    assert!(fixture.cancel.is_canceled());
    assert!(fixture.overage.tripped().is_some());
    assert!(result.stopped_the_run, "{:?}", result.outcome);
}

#[test]
fn a_disabled_switch_lets_the_node_finish() {
    let script = adapter_script(BILLED_USAGE, &stops_with("end_turn"));
    let fixture = with_switch(&script, false);
    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert!(!fixture.cancel.is_canceled());
    assert_eq!(fixture.overage.tripped(), None);
    assert!(!result.stopped_the_run);
    let usage = result.usage.expect("usage is still carried");
    assert!(usage.rate_limit.is_some_and(|limit| limit.bills_overage()));
}
