//! The user's approval for a tool that creates something.
//!
//! A separate table from the ACP permission prompts: those carry a pane id and
//! an agent-supplied `perm_id` and are *consumed* by the tap that answers
//! them, while these are daruda's own request with no ACP counterpart and a
//! card that stays answerable until settlement. The inline-keyboard and
//! callback-token machinery is shared; the bookkeeping is not.
//!
//! App-level, not per-window: the orchestrator asking to create a worktree in
//! some project is not a fact about any one window.
//!
//! Desktop host cards and eligible remote cards share one decision: the first
//! answer settles every surface. A desktop surface works without a phone.
//! Remote delivery follows captured solicitation and current presence policy.
//!
//! The initial deadline is five minutes; queuing or delivering a fresh phone
//! card extends it to at least two minutes from then. A 15-second presence
//! pump offers outstanding cards to each channel that has never queued one.
//! Timeout is silence, not consent, and permits neither retry nor bypass.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{App, Global, WeakEntity};
use smol::channel::{Receiver, Sender};

use crate::remote_channel::global::Delivery;
use crate::surface::strings as s;
use daruda_chat::view::{AgentChatView, HostCard, HostCardAction};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// Initial answer window; fresh phone cards can extend an individual deadline.
pub(crate) const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

const PHONE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

/// Cards that may be waiting at once.
///
/// The gate bounds *creation*, not *asking* — a refusal costs the agent
/// nothing, so a loop of gated calls would keep buzzing the one human in the
/// loop. Well above what a person would ever be shown deliberately, and far
/// below a notification flood.
pub(crate) const APPROVALS_IN_FLIGHT_MAX: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ApprovalId(pub u64);

const ACTION_APPROVE: &str = "approve";
const ACTION_REFUSE: &str = "refuse";

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
    /// until a desktop surface or remote channel is available.
    Undeliverable,
    /// Too many cards are already waiting, so this one was not added. The
    /// caller can retry once the user has worked through them.
    TooManyPending,
    /// The caller took the question back before anyone answered, or the
    /// orchestrator stopped. Distinct from a refusal: the user never said no,
    /// and nothing was done.
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
    waiting: HashMap<ApprovalId, WaitingApproval>,
}

struct WaitingApproval {
    tx: Sender<ApprovalOutcome>,
    summary: String,
    deadline: Instant,
    desktop: Option<WeakEntity<AgentChatView>>,
    offer: Offer,
}

/// How remote cards for a request are offered, fixed when it is made.
///
/// Solicitation is captured once: the phone that asked keeps its claim on a
/// card when the pump re-offers it later. Presence is not: it is read again
/// each time the card is offered, so it is not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offer {
    Solicited,
    ByPresence,
}

impl Offer {
    fn of(delivery: Delivery) -> Self {
        match delivery {
            Delivery::Explicit => Self::Solicited,
            Delivery::Presence { .. } => Self::ByPresence,
        }
    }

    fn delivery_now(self, cx: &mut App) -> Delivery {
        match self {
            Self::Solicited => Delivery::Explicit,
            Self::ByPresence => Delivery::Presence {
                away: crate::app_presence::is_away(cx),
            },
        }
    }
}

impl Global for Approvals {}

/// Ask the user, and answer on the returned channel exactly once.
///
/// Always offer a desktop host card; remote cards obey `delivery`. Only the
/// absence of both surfaces is undeliverable. Settlement updates remote cards
/// immediately and defers desktop refresh to avoid Workspace re-entry.
///
/// The channel is bounded at one and never awaited by the sender, so a caller
/// that has given up cannot block the resolution.
///
/// The id comes back so the caller can [`withdraw`] the question. It is minted
/// even when the answer is already in the channel — settling an id that names
/// no waiting request is a no-op by construction, so there is no state where
/// the caller has to ask whether it has one.
pub(crate) fn request(
    summary: String,
    delivery: Delivery,
    cx: &mut App,
) -> (ApprovalId, Receiver<ApprovalOutcome>) {
    let (tx, rx) = smol::channel::bounded(1);
    let deadline = cx.background_executor().now() + APPROVAL_TIMEOUT;
    let (id, full) = {
        let approvals = cx.default_global::<Approvals>();
        approvals.next_id += 1;
        let id = ApprovalId(approvals.next_id);
        let full = approvals.waiting.len() >= APPROVALS_IN_FLIGHT_MAX;
        approvals.waiting.insert(
            id,
            WaitingApproval {
                tx,
                summary,
                deadline,
                desktop: None,
                offer: Offer::of(delivery),
            },
        );
        (id, full)
    };
    if full {
        settle(id, Settlement::TooManyPending, cx);
    } else {
        send_card(id, delivery, cx);
        if is_waiting(id, cx) {
            spawn_timeout(id, cx);
        }
    }
    (id, rx)
}

/// Take back a question nobody has answered yet. `false` when it was already
/// settled — by an answer, the timeout, or an earlier withdrawal.
///
/// The work behind the card has not started (that is what "waiting" means), so
/// withdrawing is enough to make sure it never does.
pub(crate) fn withdraw(id: ApprovalId, cx: &mut App) -> bool {
    settle(id, Settlement::Withdrawn(Withdrawal::Caller), cx)
}

pub(crate) fn withdraw_all(cx: &mut App) {
    let ids: Vec<_> = cx
        .try_global::<Approvals>()
        .map(|approvals| approvals.waiting.keys().copied().collect())
        .unwrap_or_default();
    for id in ids {
        settle(id, Settlement::Withdrawn(Withdrawal::OrchestratorStop), cx);
    }
}

/// Whether `id` is still waiting for an answer.
pub(crate) fn is_waiting(id: ApprovalId, cx: &App) -> bool {
    cx.try_global::<Approvals>()
        .is_some_and(|a| a.waiting.contains_key(&id))
}

pub(crate) fn has_waiting(cx: &App) -> bool {
    cx.try_global::<Approvals>()
        .is_some_and(|approvals| !approvals.waiting.is_empty())
}

pub(crate) fn relay_outstanding(cx: &mut App) {
    let ids: Vec<_> = cx
        .try_global::<Approvals>()
        .map(|approvals| approvals.waiting.keys().copied().collect())
        .unwrap_or_default();
    for id in ids {
        let Some((summary, offer)) = cx
            .global::<Approvals>()
            .waiting
            .get(&id)
            .map(|waiting| (waiting.summary.clone(), waiting.offer))
        else {
            continue;
        };
        let delivery = offer.delivery_now(cx);
        if crate::remote_channel::send_approval_card(id, summary, delivery, cx) {
            extend_phone_deadline(id, cx);
        }
    }
}

pub(crate) fn remote_card_delivered(id: ApprovalId, cx: &mut App) {
    extend_phone_deadline(id, cx);
}

fn extend_phone_deadline(id: ApprovalId, cx: &mut App) {
    if !cx.has_global::<Approvals>() {
        return;
    }
    let deadline = cx.background_executor().now() + PHONE_RESPONSE_TIMEOUT;
    if let Some(waiting) = cx.global_mut::<Approvals>().waiting.get_mut(&id) {
        waiting.deadline = waiting.deadline.max(deadline);
    }
}

pub(crate) fn resolve_desktop(card_id: u64, action_id: &str, cx: &mut App) -> bool {
    let choice = match action_id {
        ACTION_APPROVE => ApprovalChoice::Approved,
        ACTION_REFUSE => ApprovalChoice::Refused,
        _ => return false,
    };
    settle(
        ApprovalId(card_id),
        Settlement::Answered {
            choice,
            by: Answerer::Desktop,
        },
        cx,
    )
}

/// Deliver a decision made on a remote channel. `false` when `id` was already
/// answered — the card keeps its buttons while it is live, so a second tap is
/// ordinary use, and the caller needs to know not to confirm a decision that
/// did not take effect.
pub(crate) fn resolve(id: ApprovalId, choice: ApprovalChoice, by: Answerer, cx: &mut App) -> bool {
    settle(id, Settlement::Answered { choice, by }, cx)
}

/// Answer whichever request `id` names, if it is still waiting. `false` when
/// nothing was.
///
/// Removing the entry *is* the decision: the first desktop/remote answer,
/// timeout or withdrawal wins, and later attempts find nothing.
fn settle(id: ApprovalId, settlement: Settlement, cx: &mut App) -> bool {
    if !cx.has_global::<Approvals>() {
        return false;
    }
    let Some(waiting) = cx.global_mut::<Approvals>().waiting.remove(&id) else {
        return false;
    };
    let outcome = settlement.outcome();
    let label = settlement.label();
    crate::remote_channel::settle_card(
        crate::remote_channel::cards::CardId::Approval(id),
        &label,
        cx,
    );
    crate::telegram::trace::delivery("approval.settled", || {
        format!(
            "id={id:?} outcome={outcome:?} by={} summary={}",
            settlement.trace_key(),
            crate::telegram::trace::digest(&waiting.summary)
        )
    });
    let _ = waiting.tx.try_send(outcome);
    cx.defer(move |cx| {
        if let Some(view) = waiting.desktop.and_then(|view| view.upgrade()) {
            view.update(cx, |view, cx| view.settle_host_card(id.0, label, cx));
        }
        crate::window_registry::WindowRegistry::for_each_workspace(cx, |_, _, cx| cx.notify());
    });
    true
}

/// Where a person answered a card from. Names the surface on the settled
/// card, so an answer given in a desktop Slack client does not read as "on
/// phone".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answerer {
    Desktop,
    Telegram,
    Slack,
    Discord,
}

/// Who took a question back before anyone answered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Withdrawal {
    Caller,
    OrchestratorStop,
}

/// How a request stopped waiting. Only an answer has an answerer, so a
/// decision without a surface, or a timeout with one, cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settlement {
    Answered {
        choice: ApprovalChoice,
        by: Answerer,
    },
    TimedOut,
    Withdrawn(Withdrawal),
    TooManyPending,
    Undeliverable,
}

impl Settlement {
    fn outcome(self) -> ApprovalOutcome {
        match self {
            Self::Answered { choice, .. } => choice.into(),
            Self::TimedOut => ApprovalOutcome::TimedOut,
            Self::Withdrawn(_) => ApprovalOutcome::Withdrawn,
            Self::TooManyPending => ApprovalOutcome::TooManyPending,
            Self::Undeliverable => ApprovalOutcome::Undeliverable,
        }
    }

    /// The `by=` value of the `approval.settled` trace. Stable across
    /// releases so delivery logs stay comparable.
    fn trace_key(self) -> &'static str {
        match self {
            Self::Answered { by, .. } => match by {
                Answerer::Desktop => "desktop",
                Answerer::Telegram => "telegram",
                Answerer::Slack => "slack",
                Answerer::Discord => "discord",
            },
            Self::TimedOut => "timeout",
            Self::Withdrawn(Withdrawal::Caller) => "caller",
            Self::Withdrawn(Withdrawal::OrchestratorStop) => "orchestrator_stop",
            Self::TooManyPending => "limit",
            Self::Undeliverable => "delivery",
        }
    }

    fn label(self) -> String {
        use ApprovalChoice::{Approved, Refused};
        match self {
            Self::Answered { choice, by } => match (choice, by) {
                (Approved, Answerer::Desktop) => s::control::approval_approved_desktop(),
                (Approved, Answerer::Telegram) => s::control::approval_approved_telegram(),
                (Approved, Answerer::Slack) => s::control::approval_approved_slack(),
                (Approved, Answerer::Discord) => s::control::approval_approved_discord(),
                (Refused, Answerer::Desktop) => s::control::approval_refused_desktop(),
                (Refused, Answerer::Telegram) => s::control::approval_refused_telegram(),
                (Refused, Answerer::Slack) => s::control::approval_refused_slack(),
                (Refused, Answerer::Discord) => s::control::approval_refused_discord(),
            },
            Self::Withdrawn(_) => s::control::approval_cancelled(),
            Self::TimedOut => s::control::approval_timed_out(),
            Self::Undeliverable => s::control::error_approval_unavailable(),
            Self::TooManyPending => s::control::error_approvals_pending(),
        }
    }
}

/// Close the request as unanswered once the window passes.
///
/// The timeout removes the entry, so a tap that arrives afterwards resolves
/// nothing — the answer the caller already got stands.
fn spawn_timeout(id: ApprovalId, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let remaining = cx.update(|cx| {
                let deadline = cx.try_global::<Approvals>()?.waiting.get(&id)?.deadline;
                let now = cx.background_executor().now();
                if deadline <= now {
                    settle(id, Settlement::TimedOut, cx);
                    None
                } else {
                    Some(deadline.duration_since(now))
                }
            });
            let Some(remaining) = remaining else {
                break;
            };
            cx.background_executor().timer(remaining).await;
        }
    })
    .detach();
}

fn send_card(id: ApprovalId, delivery: Delivery, cx: &mut App) {
    let Some(summary) = cx
        .try_global::<Approvals>()
        .and_then(|approvals| approvals.waiting.get(&id))
        .map(|waiting| waiting.summary.clone())
    else {
        return;
    };
    let remote = crate::remote_channel::send_approval_card(id, summary.clone(), delivery, cx);
    if remote {
        extend_phone_deadline(id, cx);
    }
    cx.defer(move |cx| {
        if !is_waiting(id, cx) {
            return;
        }
        let card = HostCard {
            id: id.0,
            summary,
            actions: vec![
                HostCardAction {
                    id: ACTION_APPROVE.into(),
                    label: s::control::approval_allow(),
                },
                HostCardAction {
                    id: ACTION_REFUSE.into(),
                    label: s::control::approval_refuse(),
                },
            ],
        };
        let desktop = crate::window_registry::WindowRegistry::orchestrator(cx).and_then(
            |(_, host)| match host.update(cx, |ws, cx| ws.show_approval_card(card, cx)) {
                Ok(view) => view,
                Err(error) => {
                    crate::remote_channel::log_error(
                        "Desktop approval surface unavailable",
                        &error,
                        "approval.desktop.unavailable",
                    );
                    None
                }
            },
        );
        if desktop.is_none() && !remote {
            LogWriter::log(
                ErrorReport::new(
                    "Approval card not sent: no desktop or remote surface can deliver it",
                )
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("approval.undeliverable")
                .build(),
            );
            settle(id, Settlement::Undeliverable, cx);
            return;
        }
        if let Some(waiting) = cx.global_mut::<Approvals>().waiting.get_mut(&id) {
            waiting.desktop = desktop;
        }
        crate::window_registry::WindowRegistry::for_each_workspace(cx, |_, _, cx| cx.notify());
    });
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
    resolve(ids[0], choice, Answerer::Telegram, cx);
}

#[cfg(test)]
mod tests;
