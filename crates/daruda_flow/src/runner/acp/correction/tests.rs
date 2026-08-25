//! Which breaches are worth one more turn, and the arithmetic that decides
//! whether there is time for it. Pure gate questions; the wire-level half is
//! `runner::acp::tests::correction`.

use super::*;
use std::path::PathBuf;

fn breach(kind: BreachKind) -> ContractBreach {
    ContractBreach {
        kind,
        first: "the reason".to_string(),
        rest: Vec::new(),
    }
}

fn missing() -> ContractBreach {
    breach(BreachKind::Missing {
        expected: PathBuf::from("/run/design.md"),
    })
}

fn misshapen() -> ContractBreach {
    breach(BreachKind::Schema {
        expected: PathBuf::from("/run/verdict.json"),
    })
}

const TEN_MINUTES: Duration = Duration::from_secs(600);

/// A turn that ended cleanly with nothing on the path is the one breach
/// a second ask can answer.
#[test]
fn a_missing_output_with_time_left_is_worth_one_more_ask() {
    assert!(may_go_again(
        &missing(),
        false,
        Duration::from_secs(60),
        Duration::ZERO,
        TEN_MINUTES
    ));
}

/// The second fixable breach: the file is this node's and its contents
/// are the wrong shape, which is exactly what one more ask can put right —
/// and the session it would be asked in still holds the schema.
#[test]
fn a_misshapen_output_is_worth_one_more_ask_too() {
    assert!(may_go_again(
        &misshapen(),
        false,
        Duration::from_secs(60),
        Duration::ZERO,
        TEN_MINUTES
    ));
}

/// A link, or an output resolving outside the run, is a refusal rather
/// than a mistake to point out: the bytes it names are not this node's
/// work, so asking the agent that named them again checks nothing.
#[test]
fn a_link_or_an_escape_is_never_corrected() {
    let expected = PathBuf::from("/run/design.md");
    for kind in [
        BreachKind::NotAFile {
            expected: expected.clone(),
        },
        BreachKind::Escapes {
            expected,
            resolved: PathBuf::from("/elsewhere/design.md"),
        },
    ] {
        assert!(
            !may_go_again(
                &breach(kind.clone()),
                false,
                Duration::ZERO,
                Duration::ZERO,
                TEN_MINUTES
            ),
            "{kind:?} must not buy another turn"
        );
    }
}

/// A stopped run pays for nothing more, however fixable the breach —
/// asserted for both fixable kinds, since the gate order is what decides
/// it and a kind added past the cancel check would slip through.
#[test]
fn a_canceled_run_buys_no_correction() {
    for breach in [missing(), misshapen()] {
        assert!(!may_go_again(
            &breach,
            true,
            Duration::ZERO,
            Duration::ZERO,
            TEN_MINUTES
        ));
    }
}

/// A correction started on the last seconds of a budget dies as a
/// `Timeout`, which reports the clock and buries both the breach and the
/// attempt to fix it — and hands a repair's `{{failure}}` "timed out" as
/// the thing to put right.
#[test]
fn a_turn_that_has_nearly_spent_its_budget_does_not_start_one() {
    assert!(!may_go_again(
        &missing(),
        false,
        Duration::from_secs(590),
        Duration::ZERO,
        TEN_MINUTES
    ));
}

/// **The sign that matters.** The node's clock stops while a person is
/// waited on, so the time a correction has to fit into is work time. On
/// `elapsed + paused` this turn — nine minutes of waiting and a minute
/// of work — would be refused for the time somebody else took.
#[test]
fn a_long_wait_for_a_person_does_not_deny_a_correction() {
    assert!(may_go_again(
        &missing(),
        false,
        Duration::from_secs(600),
        Duration::from_secs(540),
        TEN_MINUTES
    ));
}
