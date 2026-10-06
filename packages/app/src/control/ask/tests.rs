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
    let (id, _deadline, rx) = cx.update(|cx| wait_for(target, cx));
    cx.update(|cx| {
        assert!(is_waiting(target, id, cx));
        assert!(resolve(
            target,
            PaneAnswer::Text {
                text: "done".into()
            },
            cx
        ));
        assert!(!is_waiting(target, id, cx), "answering ends the wait");
    });
    assert_eq!(
        rx.try_recv().expect("answered"),
        Some(PaneAnswer::Text {
            text: "done".into()
        })
    );
}

/// Most turns nobody asked about, so this is the common path and must be
/// cheap and silent rather than an error.
#[gpui::test]
fn settling_a_pane_nobody_asked_about_does_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(!resolve(pane(7), PaneAnswer::NoAnswer, cx));
        assert!(!has_waiter(pane(7), cx));
        assert_eq!(waiting_count_for_test(cx), 0);
    });
}

/// A withdrawal has to be *sent*, not just dropped. A dropped channel is how
/// the app going away looks, and the two owe the caller opposite things: one
/// owes silence (it freed its request id), the other owes the honest failure.
#[gpui::test]
fn withdrawing_sends_the_absence_of_an_answer(cx: &mut TestAppContext) {
    let target = pane(2);
    let (id, _deadline, rx) = cx.update(|cx| wait_for(target, cx));
    cx.update(|cx| {
        assert!(withdraw(target, id, cx));
        assert!(!is_waiting(target, id, cx));
        assert!(!resolve(target, PaneAnswer::NoAnswer, cx), "already gone");
    });
    assert_eq!(
        rx.try_recv()
            .expect("the withdrawal is delivered, not dropped"),
        None,
        "`None` is what tells the relay to send no reply at all"
    );
}

#[gpui::test]
fn withdrawing_twice_reports_that_nothing_was_waiting(cx: &mut TestAppContext) {
    let target = pane(3);
    cx.update(|cx| {
        let (id, _deadline, _rx) = wait_for(target, cx);
        assert!(withdraw(target, id, cx));
        assert!(!withdraw(target, id, cx));
    });
}

/// A cancellation for a call already answered must not reach into the *next*
/// call's wait. The `outstanding` record that names it can outlive it, so the
/// id is the only thing keeping the two apart.
#[gpui::test]
fn a_stale_withdrawal_does_not_take_back_a_later_wait(cx: &mut TestAppContext) {
    let target = pane(11);
    let (first, _d1, _rx1) = cx.update(|cx| wait_for(target, cx));
    cx.update(|cx| assert!(resolve(target, PaneAnswer::NoAnswer, cx)));

    let (second, _d2, rx2) = cx.update(|cx| wait_for(target, cx));
    cx.update(|cx| {
        assert!(
            !withdraw(target, first, cx),
            "the first call's cancellation names a wait that is gone"
        );
        assert!(is_waiting(target, second, cx), "the live wait is untouched");
    });
    assert!(rx2.try_recv().is_err(), "and it has not been answered");
}

/// Unreachable while a waiter is only registered for a delivered prompt, but
/// a displaced caller must not be left on a channel nothing will ever send to.
#[gpui::test]
fn a_second_wait_on_one_pane_does_not_strand_the_first(cx: &mut TestAppContext) {
    let target = pane(4);
    let (first, second) = cx.update(|cx| {
        let (_i1, _d1, first) = wait_for(target, cx);
        let (_i2, _d2, second) = wait_for(target, cx);
        (first, second)
    });
    assert_eq!(
        first.try_recv().expect("the displaced call was told"),
        Some(PaneAnswer::StillWorking)
    );
    cx.update(|cx| assert!(resolve(target, PaneAnswer::NoAnswer, cx)));
    assert_eq!(
        second.try_recv().expect("the live call"),
        Some(PaneAnswer::NoAnswer)
    );
}

/// Two panes are two waits — settling one must not answer the other.
#[gpui::test]
fn waits_are_kept_apart_by_pane(cx: &mut TestAppContext) {
    let (a, b) = (pane(5), pane(6));
    let (ra, rb, id_b) = cx.update(|cx| {
        let (_ia, _da, ra) = wait_for(a, cx);
        let (ib, _db, rb) = wait_for(b, cx);
        assert_eq!(waiting_count_for_test(cx), 2);
        (ra, rb, ib)
    });
    cx.update(|cx| {
        assert!(resolve(a, PaneAnswer::NoAnswer, cx));
        assert!(is_waiting(b, id_b, cx), "b's wait is its own");
    });
    assert_eq!(
        ra.try_recv().expect("a answered"),
        Some(PaneAnswer::NoAnswer)
    );
    assert!(rb.try_recv().is_err(), "b has not been answered");
}

/// A turn that never settles must not hold a tool call for good.
#[gpui::test]
async fn a_wait_that_outlives_its_window_answers_still_working(cx: &mut TestAppContext) {
    let target = pane(8);
    let (id, _deadline, rx) = cx.update(|cx| wait_for(target, cx));
    cx.executor()
        .advance_clock(ASK_TIMEOUT + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        rx.try_recv().expect("the timeout answered"),
        Some(PaneAnswer::StillWorking),
        "the turn is still running, which is not a failure"
    );
    cx.update(|cx| assert!(!is_waiting(target, id, cx), "and the wait is gone"));
}

/// The defect this id exists for. A wait that settled early leaves its timer
/// armed; without the identity check that timer answers the *next* ask on the
/// pane, cutting a 300 s window down to however long the first ask had left —
/// and repeat asks on one pane are the surface's primary use.
#[gpui::test]
async fn a_settled_waits_timer_does_not_answer_the_next_one(cx: &mut TestAppContext) {
    let target = pane(10);
    let (_first_id, _d1, first) = cx.update(|cx| wait_for(target, cx));
    cx.update(|cx| assert!(resolve(target, PaneAnswer::NoAnswer, cx)));
    assert_eq!(
        first.try_recv().expect("answered at once"),
        Some(PaneAnswer::NoAnswer)
    );

    // Walk the clock to just before the first wait's timer is due, then start
    // a second wait — which is entitled to the whole window of its own.
    cx.executor()
        .advance_clock(ASK_TIMEOUT - Duration::from_secs(1));
    cx.run_until_parked();
    let (second_id, _d2, second) = cx.update(|cx| wait_for(target, cx));

    // The first timer comes due here.
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(
        second.try_recv().is_err(),
        "the first wait's clock must not end the second wait"
    );
    cx.update(|cx| assert!(is_waiting(target, second_id, cx), "still waiting"));

    // Its own window still ends it.
    cx.executor().advance_clock(ASK_TIMEOUT);
    cx.run_until_parked();
    assert_eq!(
        second.try_recv().expect("its own timeout answered"),
        Some(PaneAnswer::StillWorking)
    );
}

/// The deadline is what a caller holding a record of the wait prunes by.
#[gpui::test]
fn the_deadline_is_the_window_from_now(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let (_id, deadline, _rx) = wait_for(pane(9), cx);
        assert!(
            deadline > Instant::now(),
            "a deadline already past prunes immediately"
        );
        assert!(deadline <= Instant::now() + ASK_TIMEOUT);
    });
}
