//! Timeout, cancel and permission — everything that ends a turn from
//! outside it. These share the parking and permission adapters, which
//! nothing else needs.

use super::*;

/// One turn against an adapter offering `options`, under `policy`.
/// Returns the answer the runner sent back, verbatim off the wire.
/// The answer the runner put on the wire, and how the node ended. The
/// two are separate questions: `Deny` answers *and* fails, so a helper
/// that asserted the outcome could only serve one policy.
fn permission_answer(
    policy: PermissionPolicy,
    options: &[&str],
) -> (Result<(), NodeFailure>, String) {
    let (_probe, answered) = probe("answer.json");
    let mut fixture = Fixture::with_script(&permission_adapter(&options.join(","), &answered));
    fixture.permission = policy;

    let result = fixture.run(&spec(AGENT));
    // Empty when nothing was written: a `Deny` node ends the moment it
    // answers, so the session — and the reply's flush — is gone before
    // the adapter can record it. Callers that care about the wire use a
    // policy that leaves the session alive.
    let answer = std::fs::read_to_string(&answered).unwrap_or_default();
    (result.outcome, answer)
}

/// §6's procedure: cancel the turn, wait out the grace, then drop. The
/// grace is what gives an adapter the chance to stop mid-write instead of
/// being killed with a half-written file on disk.
#[test]
fn a_turn_over_its_timeout_is_cancelled_then_dropped() {
    let (_probe, seen) = probe("cancel.seen");
    let mut fixture = Fixture::with_script(&parking_adapter(&seen, ""));
    fixture.timeout = Duration::from_millis(200);
    fixture.grace = Duration::from_millis(400);

    let started = Instant::now();
    let result = fixture.run(&spec(AGENT));
    let elapsed = started.elapsed();

    assert!(
        matches!(result.outcome, Err(NodeFailure::Timeout { .. })),
        "{:?}",
        result.outcome
    );
    assert!(
        seen.exists(),
        "the session was dropped without cancelling the turn first"
    );
    assert!(
        elapsed >= fixture.timeout + fixture.grace,
        "an adapter that never answered was dropped before its grace ran out: {elapsed:?}"
    );
}

/// An adapter that answers the cancel within the grace ends the turn
/// itself; the runner must not wait out the full grace after that.
#[test]
fn a_cancel_answered_within_the_grace_ends_the_turn_early() {
    let (_probe, seen) = probe("cancel.seen");
    let mut fixture = Fixture::with_script(&parking_adapter(&seen, ANSWERS_THE_CANCEL));
    fixture.timeout = Duration::from_millis(200);
    fixture.grace = Duration::from_secs(3);

    let started = Instant::now();
    let result = fixture.run(&spec(AGENT));
    let elapsed = started.elapsed();

    assert!(
        matches!(result.outcome, Err(NodeFailure::Timeout { .. })),
        "{:?}",
        result.outcome
    );
    assert!(
        elapsed < fixture.grace,
        "the turn had already ended and the runner waited anyway: {elapsed:?}"
    );
}

/// Cancel and timeout share one procedure and differ only in what they
/// report — a run stopped by the user must not read as a timeout.
#[test]
fn a_user_cancel_reports_a_cancel_not_a_timeout() {
    let (_probe, seen) = probe("cancel.seen");
    let fixture = Fixture::with_script(&parking_adapter(&seen, ANSWERS_THE_CANCEL));

    let cancel = fixture.cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        cancel.cancel();
    });

    let result = fixture.run(&spec(AGENT));
    let Err(NodeFailure::SessionError(message)) = &result.outcome else {
        panic!("expected a stopped session, got {:?}", result.outcome);
    };
    assert_eq!(message, crate::runner::CANCELED);
    assert!(seen.exists(), "the turn was never cancelled on the wire");
}

/// The rule that makes unattended running safe: an approval must not
/// outlive the session it was given in.
#[test]
fn an_allow_once_policy_never_selects_allow_always() {
    let (_, answered) = permission_answer(
        PermissionPolicy::AllowOnce,
        &[ALLOW_ONCE_OPTION, ALLOW_ALWAYS_OPTION, REJECT_ONCE_OPTION],
    );

    assert!(answered.contains(r#""optionId":"once""#), "{answered}");
    assert!(
        !answered.contains("always"),
        "an approval outlived its session: {answered}"
    );
}

/// A policy that may approve still must not settle for an option of
/// another kind — and the only always-allow on offer is the one option
/// that is never selectable.
#[test]
fn an_allow_once_policy_with_no_allow_once_option_cancels() {
    let (_, answered) = permission_answer(
        PermissionPolicy::AllowOnce,
        &[ALLOW_ALWAYS_OPTION, REJECT_ONCE_OPTION],
    );

    assert!(answered.contains(r#""outcome":"cancelled""#), "{answered}");
}

/// `Deny` fails the node it was asked in. `model.rs` defines it as "a
/// request arriving at all means the mode assumption was wrong, so
/// failing loud beats approving unattended" — refusing and letting the
/// turn run on is not loud: the agent works around it and the node
/// passes with nobody the wiser.
#[test]
fn a_deny_policy_fails_the_node_rather_than_only_refusing() {
    let (outcome, _) = permission_answer(
        PermissionPolicy::Deny,
        &[ALLOW_ONCE_OPTION, REJECT_ONCE_OPTION],
    );
    match outcome {
        Err(NodeFailure::PermissionDenied { tool }) => {
            assert_eq!(tool, "write a file", "the record names what was asked for")
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

/// Which option each policy picks, asked of the decision directly.
///
/// Not through the adapter: `Deny` ends the node the moment it answers,
/// so the session — and with it the reply's flush — is gone before the
/// wire can be observed. The end-to-end half is covered above and by the
/// two allow-once tests, which leave the session alive.
#[test]
fn a_policy_selects_only_an_option_of_its_own_kind() {
    use PermissionOptionKind::{AllowAlways, AllowOnce, RejectOnce};
    let all = [
        option("once", AllowOnce),
        option("always", AllowAlways),
        option("no", RejectOnce),
    ];

    assert!(matches!(
        decide(PermissionPolicy::Deny, &all),
        PermissionDecision::Reject { option_id } if option_id == "no"
    ));
    assert!(matches!(
        decide(PermissionPolicy::AllowOnce, &all),
        PermissionDecision::Allow { option_id } if option_id == "once"
    ));

    // An always-allow outlives the session, so it is never the answer —
    // not even when it is the only approval on offer.
    let no_once = [option("always", AllowAlways), option("no", RejectOnce)];
    assert!(matches!(
        decide(PermissionPolicy::AllowOnce, &no_once),
        PermissionDecision::Cancelled
    ));
    // And a policy that must refuse does not settle for an approval.
    let no_reject = [option("once", AllowOnce), option("always", AllowAlways)];
    assert!(matches!(
        decide(PermissionPolicy::Deny, &no_reject),
        PermissionDecision::Cancelled
    ));
}
