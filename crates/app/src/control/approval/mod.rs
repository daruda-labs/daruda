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
    crate::remote_channel::forget_approval(id, cx);
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
    if crate::remote_channel::send_approval_card(id, summary, cx) {
        return;
    }
    LogWriter::log(
        ErrorReport::new("Approval card not sent: no remote channel can deliver it")
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
mod tests;
