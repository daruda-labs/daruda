use super::*;
use gpui::{BorrowAppContext as _, TestAppContext};

/// A bridge that can actually deliver a card. Enabled *and* paired,
/// because an undeliverable one settles the request immediately — which
/// is the point of `undeliverable_settles_instead_of_waiting` below, not
/// the state the waiting/settling tests want.
fn setup(cx: &mut TestAppContext) -> Receiver<crate::telegram::bridge::Outbound> {
    setup_bridge(cx, true, Some(42))
}

fn setup_bridge(
    cx: &mut TestAppContext,
    enabled: bool,
    chat_id: Option<i64>,
) -> Receiver<crate::telegram::bridge::Outbound> {
    let (tx, rx) = smol::channel::unbounded();
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(daruda_config::Config {
                telegram: daruda_config::TelegramConfig {
                    enabled,
                    authorized_chat_id: chat_id,
                    ..Default::default()
                },
                ..daruda_config::Config::default()
            });
        });
        let mut outbound = crate::telegram::global::install_for_test(enabled, chat_id, cx);
        // Drain the bridge's `futures` channel into a `smol` one so the
        // tests can await it with the same primitive the store uses.
        cx.background_executor()
            .spawn(async move {
                use futures::StreamExt as _;
                while let Some(msg) = outbound.next().await {
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
    });
    rx
}

fn request_one(cx: &mut TestAppContext) -> (ApprovalId, Receiver<ApprovalOutcome>) {
    let (_id, rx) = cx.update(|cx| request("create worktree x".into(), Delivery::Explicit, cx));
    // One request, so it is the only id waiting.
    let id = cx.update(|cx| {
        *cx.global::<Approvals>()
            .waiting
            .keys()
            .next()
            .expect("one waiting")
    });
    (id, rx)
}

#[gpui::test]
async fn an_approved_request_resolves_with_approved(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, rx) = request_one(cx);
    assert!(cx.update(|cx| resolve(id, ApprovalChoice::Approved, Answerer::Telegram, cx)));
    assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Approved));
    cx.update(|cx| {
        assert_eq!(waiting_count_for_test(cx), 0);
        assert!(!withdraw(id, cx));
    });
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    assert!(rx.is_empty());
}

#[gpui::test]
async fn a_refused_request_resolves_with_refused(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, rx) = request_one(cx);
    assert!(cx.update(|cx| resolve(id, ApprovalChoice::Refused, Answerer::Telegram, cx)));
    assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Refused));
    assert!(!cx.update(|cx| resolve(id, ApprovalChoice::Approved, Answerer::Telegram, cx)));
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    assert!(rx.is_empty());
}

#[gpui::test]
async fn an_expired_request_resolves_as_timed_out(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (_id, rx) = request_one(cx);
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    assert_eq!(rx.recv().await, Ok(ApprovalOutcome::TimedOut));
    cx.update(|cx| assert_eq!(waiting_count_for_test(cx), 0));
}

/// The timeout took the entry, so a late tap must not deliver a second,
/// contradictory answer to a caller that already moved on.
#[gpui::test]
async fn a_late_tap_after_timeout_does_not_resurrect_the_request(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, rx) = request_one(cx);
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    assert_eq!(rx.recv().await, Ok(ApprovalOutcome::TimedOut));
    assert!(
        !cx.update(|cx| resolve(id, ApprovalChoice::Approved, Answerer::Telegram, cx)),
        "a late tap settles nothing, and the caller must be told so"
    );
    assert!(rx.is_empty(), "nothing more may arrive");
}

/// The card stays on screen, so a second tap is ordinary use — and must
/// not queue a second outcome behind the first.
#[gpui::test]
async fn a_second_tap_is_a_no_op(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, rx) = request_one(cx);
    assert!(cx.update(|cx| resolve(id, ApprovalChoice::Approved, Answerer::Telegram, cx)));
    assert!(
        !cx.update(|cx| resolve(id, ApprovalChoice::Refused, Answerer::Telegram, cx)),
        "the second tap changed nothing"
    );
    assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Approved));
    assert!(rx.is_empty(), "the first decision is the decision");
}

/// Two requests in flight must not answer each other.
#[gpui::test]
async fn each_request_gets_its_own_answer(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (_first_id, first) = cx.update(|cx| request("first".into(), Delivery::Explicit, cx));
    let (_second_id, second) = cx.update(|cx| request("second".into(), Delivery::Explicit, cx));
    let ids: Vec<ApprovalId> = cx.update(|cx| {
        let mut ids: Vec<_> = cx.global::<Approvals>().waiting.keys().copied().collect();
        ids.sort_by_key(|i| i.0);
        ids
    });
    assert_eq!(ids.len(), 2);
    assert!(cx.update(|cx| resolve(ids[1], ApprovalChoice::Refused, Answerer::Telegram, cx)));
    assert_eq!(second.recv().await, Ok(ApprovalOutcome::Refused));
    assert!(first.is_empty(), "the other request is still waiting");
}

#[gpui::test]
async fn a_withdrawn_request_settles_once_and_forgets_its_tokens(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, answer) = request_one(cx);
    cx.update(|cx| {
        assert!(withdraw(id, cx));
        assert!(!withdraw(id, cx));
        assert!(!resolve(
            id,
            ApprovalChoice::Approved,
            Answerer::Telegram,
            cx
        ));
        assert_eq!(waiting_count_for_test(cx), 0);
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            0
        );
    });
    assert_eq!(answer.recv().await, Ok(ApprovalOutcome::Withdrawn));
    cx.executor()
        .advance_clock(APPROVAL_TIMEOUT + Duration::from_secs(1));
    assert!(answer.is_empty());
}

#[gpui::test]
async fn each_request_keeps_its_own_deadline(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (first_id, first) = cx.update(|cx| request("first".into(), Delivery::Explicit, cx));
    cx.executor().advance_clock(Duration::from_secs(100));
    let (second_id, second) = cx.update(|cx| request("second".into(), Delivery::Explicit, cx));
    cx.executor().advance_clock(Duration::from_secs(201));
    assert_eq!(first.recv().await, Ok(ApprovalOutcome::TimedOut));
    assert!(second.is_empty());
    cx.update(|cx| {
        assert!(!is_waiting(first_id, cx));
        assert!(is_waiting(second_id, cx));
    });
    cx.executor().advance_clock(Duration::from_secs(100));
    assert_eq!(second.recv().await, Ok(ApprovalOutcome::TimedOut));
}

#[gpui::test]
async fn an_extended_deadline_is_reread_before_timeout(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (extended_id, extended) =
        cx.update(|cx| request("extended".into(), Delivery::Explicit, cx));
    let (_other_id, other) = cx.update(|cx| request("other".into(), Delivery::Explicit, cx));
    cx.executor().advance_clock(Duration::from_secs(100));
    cx.update(|cx| {
        let deadline = cx.background_executor().now() + APPROVAL_TIMEOUT;
        cx.global_mut::<Approvals>()
            .waiting
            .get_mut(&extended_id)
            .unwrap()
            .deadline = deadline;
    });
    cx.executor().advance_clock(Duration::from_secs(201));
    assert_eq!(other.recv().await, Ok(ApprovalOutcome::TimedOut));
    assert!(extended.is_empty());
    cx.update(|cx| assert!(is_waiting(extended_id, cx)));
    cx.executor().advance_clock(Duration::from_secs(100));
    assert_eq!(extended.recv().await, Ok(ApprovalOutcome::TimedOut));
    assert!(extended.is_empty());
}

/// The card actually reaches the send queue, with one token per button.
/// Without this, a send path that silently dropped every card would look
/// identical to a working one from the store's side — which is exactly
/// how the enabled/paired gate came to be missing.
#[gpui::test]
async fn a_deliverable_request_queues_one_card_with_two_tokens(cx: &mut TestAppContext) {
    let outbound = setup(cx);
    let (_id, _rx) = cx.update(|cx| request("create worktree x".into(), Delivery::Explicit, cx));
    cx.run_until_parked();
    let sent = outbound.recv().await.expect("a card was queued");
    let crate::telegram::bridge::Outbound::Approval(prompt) = sent else {
        panic!("an approval must queue as a card, not as {sent:?}");
    };
    assert!(prompt.summary.contains('x'));
    assert_eq!(prompt.buttons.len(), 2, "allow and refuse");
    assert_ne!(
        prompt.buttons[0].1, prompt.buttons[1].1,
        "one token per button"
    );
    cx.update(|cx| assert_eq!(waiting_count_for_test(cx), 1));
}

/// Telegram off, or on but unpaired: nobody can be asked, so the caller
/// hears that immediately instead of waiting out five minutes and being
/// told nobody answered.
#[gpui::test]
async fn an_undeliverable_request_settles_instead_of_waiting(cx: &mut TestAppContext) {
    for (enabled, chat_id) in [(false, Some(42)), (true, None), (false, None)] {
        let mut app = TestAppContext::single();
        let _outbound = setup_bridge(&mut app, enabled, chat_id);
        let (_id, rx) =
            app.update(|cx| request("create worktree x".into(), Delivery::Explicit, cx));
        assert_eq!(
            rx.recv().await,
            Ok(ApprovalOutcome::Undeliverable),
            "enabled={enabled} chat_id={chat_id:?}"
        );
        app.update(|cx| assert_eq!(waiting_count_for_test(cx), 0));
    }
    let _ = cx;
}

/// Settling drops the card's tokens, so the bounded table holds only
/// answerable cards and cannot evict a live one.
#[gpui::test]
async fn settling_forgets_the_cards_tokens(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let (id, _rx) = request_one(cx);
    cx.update(|cx| {
        assert!(
            crate::telegram::global::pending_approval_tokens_for_test(cx) > 0,
            "the card is answerable while it waits"
        );
        assert!(resolve(
            id,
            ApprovalChoice::Approved,
            Answerer::Telegram,
            cx
        ));
        assert_eq!(
            crate::telegram::global::pending_approval_tokens_for_test(cx),
            0,
            "an answered card leaves no live tokens"
        );
    });
}

/// The gate bounds creation, not asking — so asking has its own bound, or
/// a loop of gated calls becomes a notification flood.
#[gpui::test]
async fn too_many_waiting_cards_refuses_rather_than_queueing(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    let mut held = Vec::new();
    for _ in 0..APPROVALS_IN_FLIGHT_MAX {
        held.push(
            cx.update(|cx| request("create worktree x".into(), Delivery::Explicit, cx))
                .1,
        );
    }
    cx.update(|cx| assert_eq!(waiting_count_for_test(cx), APPROVALS_IN_FLIGHT_MAX));

    let (_over_id, over) = cx.update(|cx| request("one too many".into(), Delivery::Explicit, cx));
    assert_eq!(over.recv().await, Ok(ApprovalOutcome::TooManyPending));
    cx.update(|cx| {
        assert_eq!(
            waiting_count_for_test(cx),
            APPROVALS_IN_FLIGHT_MAX,
            "a refusal must not grow the table"
        );
    });
    assert!(held.iter().all(|rx| rx.is_empty()), "the held ones wait on");
}

/// An unknown id is not an error — it is the losing half of a race that
/// already resolved.
#[gpui::test]
async fn resolving_an_unknown_id_does_nothing(cx: &mut TestAppContext) {
    let _outbound = setup(cx);
    assert!(!cx.update(|cx| resolve(
        ApprovalId(9_999),
        ApprovalChoice::Approved,
        Answerer::Telegram,
        cx
    )));
    cx.update(|cx| assert_eq!(waiting_count_for_test(cx), 0));
}

#[gpui::test]
fn remote_delivery_keeps_a_later_deadline_and_never_resurrects_a_settled_request(
    cx: &mut TestAppContext,
) {
    let _outbound = setup(cx);
    let (id, answer) = request_one(cx);
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.update(|cx| {
        let deadline = cx.global::<Approvals>().waiting[&id].deadline;
        remote_card_delivered(id, cx);
        assert_eq!(cx.global::<Approvals>().waiting[&id].deadline, deadline);
        assert!(withdraw(id, cx));
        remote_card_delivered(id, cx);
        relay_outstanding(cx);
        assert!(!is_waiting(id, cx));
        assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
        assert!(answer.is_empty());
    });
}

#[gpui::test]
fn a_rejected_retry_queue_does_not_extend_the_deadline(cx: &mut TestAppContext) {
    let mut outgoing = cx.update(crate::remote_channel::global::tests::setup);
    let (id, answer) = cx.update(|cx| {
        request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        )
    });
    assert_eq!(outgoing.try_recv().unwrap().id, "personal");
    outgoing.close();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(299));
    cx.update(|cx| {
        crate::app_presence::seed_for_test(
            crate::platform::presence::AwaySignal::HERE.observe(
                false,
                Some(Duration::from_secs(300)),
                Instant::now() - Duration::from_secs(120),
            ),
            false,
            Some(Duration::from_secs(300)),
            cx,
        );
        let deadline = cx.global::<Approvals>().waiting[&id].deadline;
        relay_outstanding(cx);
        assert_eq!(cx.global::<Approvals>().waiting[&id].deadline, deadline);
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}

/// A solicited request stays solicited when the pump re-offers it: the phone
/// that asked must get its card even if its channel only connected later and
/// the desk is still occupied.
#[gpui::test]
fn the_pump_reoffers_a_solicited_approval_regardless_of_presence(cx: &mut TestAppContext) {
    let mut outgoing = cx.update(|cx| {
        let outgoing = crate::remote_channel::global::tests::setup(cx);
        cx.global_mut::<crate::remote_channel::global::RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .bot = None;
        outgoing
    });
    let (id, _answer) = cx.update(|cx| request("Create worktree".into(), Delivery::Explicit, cx));
    assert_eq!(outgoing.try_recv().unwrap().id, "personal");
    assert!(outgoing.try_recv().is_err(), "work was still connecting");
    cx.update(|cx| {
        assert!(!crate::app_presence::is_away(cx));
        cx.global_mut::<crate::remote_channel::global::RemoteChannels>()
            .connections
            .get_mut("work")
            .unwrap()
            .publish_bot(crate::remote_channel::global::tests::ready_bot());
        relay_outstanding(cx);
    });
    assert_eq!(
        outgoing.try_recv().unwrap().id,
        "work",
        "the only_when_away channel still owes the phone that asked its card"
    );
    cx.update(|cx| {
        relay_outstanding(cx);
        assert!(withdraw(id, cx));
    });
    assert!(outgoing.try_recv().is_err(), "offered once per channel");
}

/// The settled card names the surface that answered: a Slack answer may come
/// from a desktop Slack client, so it must not read as "on phone".
#[test]
fn a_settled_card_names_the_surface_that_answered() {
    let answered = |choice, by| Settlement::Answered { choice, by };
    for (by, approved, refused, key) in [
        (
            Answerer::Desktop,
            s::control::approval_approved_desktop(),
            s::control::approval_refused_desktop(),
            "desktop",
        ),
        (
            Answerer::Telegram,
            s::control::approval_approved_telegram(),
            s::control::approval_refused_telegram(),
            "telegram",
        ),
        (
            Answerer::Slack,
            s::control::approval_approved_slack(),
            s::control::approval_refused_slack(),
            "slack",
        ),
        (
            Answerer::Discord,
            s::control::approval_approved_discord(),
            s::control::approval_refused_discord(),
            "discord",
        ),
    ] {
        assert_eq!(answered(ApprovalChoice::Approved, by).label(), approved);
        assert_eq!(answered(ApprovalChoice::Refused, by).label(), refused);
        assert_eq!(answered(ApprovalChoice::Approved, by).trace_key(), key);
        assert_eq!(
            answered(ApprovalChoice::Refused, by).outcome(),
            ApprovalOutcome::Refused
        );
    }
    assert_eq!(Settlement::TimedOut.trace_key(), "timeout");
    assert_eq!(
        Settlement::Withdrawn(Withdrawal::OrchestratorStop).outcome(),
        ApprovalOutcome::Withdrawn
    );
}
