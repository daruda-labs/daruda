use super::*;
use gpui::TestAppContext;

fn pane(n: u64) -> PaneRef {
    PaneRef {
        workspace: daruda_store::project::WorkspaceUuid::new(),
        pane: n,
    }
}

#[gpui::test]
fn a_settled_turn_answers_the_call_waiting_on_it(cx: &mut TestAppContext) {
    let target = pane(1);
    let rx = cx.update(|cx| {
        let (_deadline, rx) = wait_for(target, cx);
        assert!(is_waiting(target, cx));
        rx
    });
    cx.update(|cx| {
        assert!(resolve(
            target,
            PaneAnswer::Text {
                text: "done".into()
            },
            cx
        ));
        assert!(!is_waiting(target, cx), "answering ends the wait");
    });
    assert_eq!(
        rx.try_recv().expect("answered"),
        PaneAnswer::Text {
            text: "done".into()
        }
    );
}

/// Most turns nobody asked about, so this is the common path and must be
/// cheap and silent rather than an error.
#[gpui::test]
fn settling_a_pane_nobody_asked_about_does_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(!resolve(pane(7), PaneAnswer::NoAnswer, cx));
        assert_eq!(waiting_count_for_test(cx), 0);
    });
}

/// The whole point of the withdrawal: the turn is the pane's, not the
/// caller's, so giving up on the answer must not be able to stop it.
#[gpui::test]
fn withdrawing_only_ends_the_wait(cx: &mut TestAppContext) {
    let target = pane(2);
    let rx = cx.update(|cx| {
        let (_deadline, rx) = wait_for(target, cx);
        rx
    });
    cx.update(|cx| {
        assert!(withdraw(target, cx));
        assert!(!is_waiting(target, cx));
        // Nothing was sent — a withdrawn call is owed no answer.
        assert!(!resolve(target, PaneAnswer::NoAnswer, cx), "already gone");
    });
    assert!(rx.try_recv().is_err(), "a withdrawn call gets no reply");
}

#[gpui::test]
fn withdrawing_twice_reports_that_nothing_was_waiting(cx: &mut TestAppContext) {
    let target = pane(3);
    cx.update(|cx| {
        let _ = wait_for(target, cx);
        assert!(withdraw(target, cx));
        assert!(!withdraw(target, cx));
    });
}

/// Unreachable while a waiter is only registered for a delivered prompt, but
/// a displaced caller must not be left on a channel nothing will ever send to.
#[gpui::test]
fn a_second_wait_on_one_pane_does_not_strand_the_first(cx: &mut TestAppContext) {
    let target = pane(4);
    let (first, second) = cx.update(|cx| {
        let (_d1, first) = wait_for(target, cx);
        let (_d2, second) = wait_for(target, cx);
        (first, second)
    });
    assert_eq!(
        first.try_recv().expect("the displaced call was told"),
        PaneAnswer::StillWorking
    );
    cx.update(|cx| {
        assert!(resolve(target, PaneAnswer::NoAnswer, cx));
    });
    assert_eq!(
        second.try_recv().expect("the live call"),
        PaneAnswer::NoAnswer
    );
}

/// Two panes are two waits — settling one must not answer the other.
#[gpui::test]
fn waits_are_kept_apart_by_pane(cx: &mut TestAppContext) {
    let (a, b) = (pane(5), pane(6));
    let (ra, rb) = cx.update(|cx| {
        let (_da, ra) = wait_for(a, cx);
        let (_db, rb) = wait_for(b, cx);
        assert_eq!(waiting_count_for_test(cx), 2);
        (ra, rb)
    });
    cx.update(|cx| {
        assert!(resolve(a, PaneAnswer::NoAnswer, cx));
        assert!(is_waiting(b, cx), "b's wait is its own");
    });
    assert_eq!(ra.try_recv().expect("a answered"), PaneAnswer::NoAnswer);
    assert!(rb.try_recv().is_err(), "b has not been answered");
}

/// A turn that never settles must not hold a tool call for good.
#[gpui::test]
async fn a_wait_that_outlives_its_window_answers_still_working(cx: &mut TestAppContext) {
    let target = pane(8);
    let rx = cx.update(|cx| {
        let (_deadline, rx) = wait_for(target, cx);
        rx
    });
    cx.executor()
        .advance_clock(ASK_TIMEOUT + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        rx.try_recv().expect("the timeout answered"),
        PaneAnswer::StillWorking,
        "the turn is still running, which is not a failure"
    );
    cx.update(|cx| assert!(!is_waiting(target, cx), "and the wait is gone"));
}

/// The deadline is what a caller holding a record of the wait prunes by.
#[gpui::test]
fn the_deadline_is_the_window_from_now(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let (deadline, _rx) = wait_for(pane(9), cx);
        assert!(
            deadline > Instant::now(),
            "a deadline already past prunes immediately"
        );
        assert!(deadline <= Instant::now() + ASK_TIMEOUT);
    });
}
