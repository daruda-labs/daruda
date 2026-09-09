//! The user's approval for a tool that creates something.
//!
//! A separate table from the ACP permission prompts: those carry a pane id and
//! an agent-supplied `perm_id` and are *consumed* by the tap that answers
//! them, while these are daruda's own request with no ACP counterpart and a
//! card that stays answerable while it is on screen. The inline-keyboard and
//! callback-token machinery is shared; the bookkeeping is not.
//!
//! App-level, not per-window: the orchestrator asking to create a worktree in
//! some project is not a fact about any one window.
//!
//! Bounded on purpose. A tool call waits while the card sits on the phone, and
//! a phone in a pocket must not hold an agent's turn forever.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Global};
use smol::channel::{Receiver, Sender};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// How long a card stays answerable. Long enough to walk back to the desk,
/// short enough that a forgotten card is not still live an hour later.
pub(crate) const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// Cards that may be waiting at once.
///
/// The gate bounds *creation*, not *asking* — a refusal costs the agent
/// nothing, so a loop of gated calls would keep buzzing the one human in the
/// loop. Well above what a person would ever be shown deliberately, and far
/// below a notification flood.
pub(crate) const APPROVALS_IN_FLIGHT_MAX: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ApprovalId(pub u64);

/// What a button says. Only the two the card offers — "nobody answered" is not
/// a choice anyone makes, so it is not one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalChoice {
    Approved,
    Refused,
}

/// How a request ended. Includes the outcome no button produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalOutcome {
    Approved,
    Refused,
    /// The card went unanswered. Distinct from a refusal: nobody decided.
    TimedOut,
    /// The card could not be sent, so nobody was ever asked. Distinct from
    /// both of the above: there is nothing to wait for and nothing to retry
    /// until the bridge is configured.
    Undeliverable,
    /// Too many cards are already waiting, so this one was not added. The
    /// caller can retry once the user has worked through them.
    TooManyPending,
    /// The caller took the question back before anyone answered — an MCP
    /// `notifications/cancelled` for the call that asked it. Distinct from a
    /// refusal: the user never said no, and nothing was done.
    Withdrawn,
}

impl From<ApprovalChoice> for ApprovalOutcome {
    fn from(choice: ApprovalChoice) -> Self {
        match choice {
            ApprovalChoice::Approved => Self::Approved,
            ApprovalChoice::Refused => Self::Refused,
        }
    }
}

/// Requests still waiting for an answer.
#[derive(Default)]
struct Approvals {
    next_id: u64,
    /// The sender half of each waiting caller's channel. An entry's presence
    /// *is* the request being live: whichever of the tap and the timeout gets
    /// here first removes it, so the loser finds nothing and does nothing.
    waiting: HashMap<ApprovalId, Sender<ApprovalOutcome>>,
}

impl Global for Approvals {}

/// Ask the user, and answer on the returned channel exactly once.
///
/// The channel is bounded at one and never awaited by the sender, so a caller
/// that has given up cannot block the resolution.
///
/// The id comes back so the caller can [`withdraw`] the question. It is minted
/// even when the answer is already in the channel — settling an id that names
/// no waiting request is a no-op by construction, so there is no state where
/// the caller has to ask whether it has one.
pub(crate) fn request(summary: String, cx: &mut App) -> (ApprovalId, Receiver<ApprovalOutcome>) {
    let (tx, rx) = smol::channel::bounded(1);
    let id = {
        let approvals = cx.default_global::<Approvals>();
        approvals.next_id += 1;
        let id = ApprovalId(approvals.next_id);
        if approvals.waiting.len() >= APPROVALS_IN_FLIGHT_MAX {
            // Answered here rather than queued: a caller told "too many
            // pending" can back off, where one silently held behind three
            // others looks like a hang.
            let _ = tx.try_send(ApprovalOutcome::TooManyPending);
            return (id, rx);
        }
        approvals.waiting.insert(id, tx);
        id
    };
    send_card(id, summary, cx);
    spawn_timeout(id, cx);
    (id, rx)
}

/// Take back a question nobody has answered yet. `false` when it was already
/// settled — by a tap, the timeout, or an earlier withdrawal.
///
/// The work behind the card has not started (that is what "waiting" means), so
/// withdrawing is enough to make sure it never does.
pub(crate) fn withdraw(id: ApprovalId, cx: &mut App) -> bool {
    settle(id, ApprovalOutcome::Withdrawn, cx)
}

/// Whether `id` is still waiting for an answer.
pub(crate) fn is_waiting(id: ApprovalId, cx: &App) -> bool {
    cx.try_global::<Approvals>()
        .is_some_and(|a| a.waiting.contains_key(&id))
}

/// Deliver a decision. `false` when `id` was already answered — the card keeps
/// its buttons while it is live, so a second tap is ordinary use, and the
/// caller needs to know not to confirm a decision that did not take effect.
pub(crate) fn resolve(id: ApprovalId, choice: ApprovalChoice, cx: &mut App) -> bool {
    settle(id, choice.into(), cx)
}

/// Answer whichever request `id` names, if it is still waiting. `false` when
/// nothing was.
///
/// Removing the entry *is* the decision: whichever of the tap and the timeout
/// gets here first wins, and the loser finds nothing.
fn settle(id: ApprovalId, outcome: ApprovalOutcome, cx: &mut App) -> bool {
    if !cx.has_global::<Approvals>() {
        return false;
    }
    let Some(tx) = cx.global_mut::<Approvals>().waiting.remove(&id) else {
        return false;
    };
    // Dead tokens would otherwise fill a bounded table and evict a live
    // card's.
    crate::telegram::global::TelegramBridge::forget_approval(id, cx);
    // The receiver may be gone (the tool call's connection dropped). Nothing
    // to report: the decision simply has no reader.
    let _ = tx.try_send(outcome);
    true
}

/// Close the request as unanswered once the window passes.
///
/// The timeout removes the entry, so a tap that arrives afterwards resolves
/// nothing — the answer the caller already got stands.
fn spawn_timeout(id: ApprovalId, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(APPROVAL_TIMEOUT).await;
        // No `Result` to handle: `AsyncApp::update` returns the closure's
        // value, and a released app drops this task before it can run — so
        // there is nothing to swallow and nothing to report. The `bool` is
        // discarded inside the closure because a lost race is the normal
        // case: the user answered first.
        cx.update(|cx| {
            settle(id, ApprovalOutcome::TimedOut, cx);
        });
    })
    .detach();
}

/// Put the card on the phone, or settle the request as unaskable.
///
/// Failing fast rather than waiting out the timeout: a bridge that is off or
/// unpaired is a standing condition, not a transient hiccup, so making the
/// caller block five minutes would report "nobody answered" about a question
/// that was never asked. `ApprovalUnavailable` says the true thing.
fn send_card(id: ApprovalId, summary: String, cx: &mut App) {
    if crate::telegram::global::TelegramBridge::send_approval_card(id, summary, cx) {
        return;
    }
    LogWriter::log(
        ErrorReport::new("Approval card not sent: Telegram is off, unpaired, or absent")
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .dedup("approval.undeliverable")
            .build(),
    );
    settle(id, ApprovalOutcome::Undeliverable, cx);
}

#[cfg(test)]
pub(crate) fn waiting_count_for_test(cx: &App) -> usize {
    cx.try_global::<Approvals>().map_or(0, |a| a.waiting.len())
}

/// Answer whichever single request is outstanding. Lets a test stand in for
/// the phone tap without knowing the id the dispatcher never handed it.
#[cfg(test)]
pub(crate) fn resolve_only_pending_for_test(choice: ApprovalChoice, cx: &mut App) {
    let ids: Vec<ApprovalId> = cx
        .try_global::<Approvals>()
        .map(|a| a.waiting.keys().copied().collect())
        .unwrap_or_default();
    assert_eq!(ids.len(), 1, "exactly one request must be outstanding");
    resolve(ids[0], choice, cx);
}

#[cfg(test)]
mod tests {
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
        let (_id, rx) = cx.update(|cx| request("create worktree x".into(), cx));
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
        assert!(cx.update(|cx| resolve(id, ApprovalChoice::Approved, cx)));
        assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Approved));
        cx.update(|cx| assert_eq!(waiting_count_for_test(cx), 0));
    }

    #[gpui::test]
    async fn a_refused_request_resolves_with_refused(cx: &mut TestAppContext) {
        let _outbound = setup(cx);
        let (id, rx) = request_one(cx);
        assert!(cx.update(|cx| resolve(id, ApprovalChoice::Refused, cx)));
        assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Refused));
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
            !cx.update(|cx| resolve(id, ApprovalChoice::Approved, cx)),
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
        assert!(cx.update(|cx| resolve(id, ApprovalChoice::Approved, cx)));
        assert!(
            !cx.update(|cx| resolve(id, ApprovalChoice::Refused, cx)),
            "the second tap changed nothing"
        );
        assert_eq!(rx.recv().await, Ok(ApprovalOutcome::Approved));
        assert!(rx.is_empty(), "the first decision is the decision");
    }

    /// Two requests in flight must not answer each other.
    #[gpui::test]
    async fn each_request_gets_its_own_answer(cx: &mut TestAppContext) {
        let _outbound = setup(cx);
        let (_first_id, first) = cx.update(|cx| request("first".into(), cx));
        let (_second_id, second) = cx.update(|cx| request("second".into(), cx));
        let ids: Vec<ApprovalId> = cx.update(|cx| {
            let mut ids: Vec<_> = cx.global::<Approvals>().waiting.keys().copied().collect();
            ids.sort_by_key(|i| i.0);
            ids
        });
        assert_eq!(ids.len(), 2);
        assert!(cx.update(|cx| resolve(ids[1], ApprovalChoice::Refused, cx)));
        assert_eq!(second.recv().await, Ok(ApprovalOutcome::Refused));
        assert!(first.is_empty(), "the other request is still waiting");
    }

    /// The card actually reaches the send queue, with one token per button.
    /// Without this, a send path that silently dropped every card would look
    /// identical to a working one from the store's side — which is exactly
    /// how the enabled/paired gate came to be missing.
    #[gpui::test]
    async fn a_deliverable_request_queues_one_card_with_two_tokens(cx: &mut TestAppContext) {
        let outbound = setup(cx);
        let (_id, _rx) = cx.update(|cx| request("create worktree x".into(), cx));
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
            let (_id, rx) = app.update(|cx| request("create worktree x".into(), cx));
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
            assert!(resolve(id, ApprovalChoice::Approved, cx));
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
            held.push(cx.update(|cx| request("create worktree x".into(), cx)).1);
        }
        cx.update(|cx| assert_eq!(waiting_count_for_test(cx), APPROVALS_IN_FLIGHT_MAX));

        let (_over_id, over) = cx.update(|cx| request("one too many".into(), cx));
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
        assert!(!cx.update(|cx| resolve(ApprovalId(9_999), ApprovalChoice::Approved, cx)));
        cx.update(|cx| assert_eq!(waiting_count_for_test(cx), 0));
    }
}
