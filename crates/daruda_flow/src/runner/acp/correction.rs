//! Going round again: whether a turn that ended without meeting its contract
//! is worth another, and what to say when it is.
//!
//! Two reasons to go round. The output is broken — that is the correction turn
//! this file started as. Or the output is fine and says the work is not over,
//! which is a node that would otherwise stop after one turn and wait for a
//! person to say "continue".
//!
//! Here rather than in `runner/mod.rs` because only this runner can correct
//! anything — a command node owes no file — and because the arithmetic below
//! is `interrupted`'s rule about waiting, read from the other side.

use super::{Answering, Recording, drain};
use crate::runner::{BreachKind, ContractBreach, NodeFailure, RunContext};
use daruda_acp::{AcpEvent, AcpSessionHandle};
use smol::stream::Stream;
use std::time::{Duration, Instant};

/// How much of a node's budget a correction turn has to have left to be
/// worth paying for. A correction is a whole agent turn — the same order as
/// the settings budget the handshake gives one reply — and one started with
/// less than this dies as a `Timeout`, which reports the clock and buries
/// both the contract breach and the attempt to fix it.
const CORRECTION_FLOOR: Duration = Duration::from_secs(30);

/// Whether one more turn on the session that just ended could plausibly
/// put `breach` right, given what the turn has already spent.
///
/// `elapsed` and `paused` arrive separately because the node's clock stops
/// while a person is waited on: `interrupted` extends the deadline by
/// `paused`, so the time a correction has to fit into is work time. Adding
/// the wait instead would deny a correction to any ordinary turn that
/// happened to sit through a permission grant.
fn may_go_again(
    breach: &ContractBreach,
    canceled: bool,
    elapsed: Duration,
    paused: Duration,
    timeout: Duration,
) -> bool {
    // A stop does not lose the cancel — `interrupted` polls for it — but it
    // does have to stop this future paying for one more prompt on the way
    // out.
    if canceled {
        return false;
    }
    // A file that was never written, one whose contents are the wrong shape,
    // and one that says the work is unfinished are all things being asked
    // again can produce. A link, or an output resolving outside the run, is a
    // refusal: the bytes it points at are not this node's work, and re-asking
    // the agent that pointed there is not a check on it.
    if !matches!(
        breach.kind,
        BreachKind::Missing { .. } | BreachKind::Schema { .. } | BreachKind::Unfinished { .. }
    ) {
        return false;
    }
    let worked = elapsed.saturating_sub(paused);
    worked.saturating_add(CORRECTION_FLOOR) <= timeout
}

/// One more turn on the open session, for a breach a second ask could
/// answer. The session is still up and still holds the contract, which
/// is what a fresh session next attempt would not.
///
/// **Almost none of its own failure is the node's.** The first turn ended
/// cleanly, so the attempt is judged on what is on disk — which is exactly
/// what would have been reported without the correction. Only exhaustion
/// propagates, and [`ended_the_node`] says why.
pub(super) async fn keep_going(
    events: &mut (impl Stream<Item = AcpEvent> + Unpin),
    session: &AcpSessionHandle,
    ctx: &RunContext<'_>,
    rec: &Recording<'_>,
) -> Result<(), NodeFailure> {
    let Some(contract) = ctx.contract else {
        return Ok(());
    };
    // The first prompt is already spent, so this is what is left. A node
    // asking for one turn gets no second chance at all — which is what
    // `max_turns: 1` means.
    let mut left = ctx.max_turns.saturating_sub(1);
    while left > 0 {
        let Err(breach) = contract.check() else {
            return Ok(());
        };
        // Asked before the gates below because it is not about this breach:
        // somebody has already said no in this call, and going round asks the
        // same question of the same person. `NodeFailure::forbids_retry`
        // counts a refusal for the same reason.
        //
        // The design had this as "a refused turn does not come out of the
        // cap". Implemented that way it does not terminate — a policy
        // refusing every turn decrements nothing, and the loop runs until the
        // clock or the run's budget stops it, having spent both on turns that
        // were always going to be refused.
        if rec.park.any_refused() {
            return Ok(());
        }
        if !may_go_again(
            &breach,
            ctx.cancel.is_canceled(),
            rec.started.elapsed(),
            rec.park.total(Instant::now()),
            ctx.timeout,
        ) {
            return Ok(());
        }
        // Asked last of the gates, because granting it spends the run's
        // budget: a turn refused for any other reason must not have been
        // charged for. Nothing awaits between here and the send, so no other
        // node can slip in between the reservation and the prompt.
        if !(ctx.reserve_extra_turn)() {
            return Ok(());
        }
        // Two reasons to go round, two things to say. An output that is merely
        // unfinished has nothing broken in it, and telling its author to
        // satisfy the contract would send them to re-check work that was
        // already right.
        let text = match breach.kind {
            BreachKind::Unfinished { .. } => crate::contract::prompt::continue_from(&breach),
            _ => crate::contract::prompt::correction(&breach),
        };
        // Recorded here because `harvest` writes the prompt before `settle`
        // runs; a turn sent from inside it would otherwise be in the
        // transcript as answers to a question nobody asked.
        rec.log.borrow_mut().prompt(&text);
        rec.turns.set(rec.turns.get() + 1);
        session.send_prompt(text);
        let settled = drain(
            events,
            session,
            ctx,
            rec,
            Answering::Policy(&ctx.permission),
        )
        .await;
        if let Err(failure) = settled
            && ended_the_node(&failure)
        {
            return Err(failure);
        }
        left -= 1;
    }
    // Out of turns with the contract still unmet. Nothing is reported from
    // here: `judge` re-reads what is on disk and fails the node on the same
    // breach it would have failed on without any of these turns.
    Ok(())
}

/// Whether a correction's own failure is the *node's* failure.
///
/// Only the two exhaustion kinds. A correction that ran out of context or
/// out of turn requests may have stopped part-way through writing, and the
/// contract afterwards asks only whether something is there — so swallowing
/// these two passes exactly the half-written output `failure_for` refuses a
/// first turn for.
///
/// Every other kind stays swallowed, and this is not a simplification
/// waiting to happen: `forbids_retry` counts `Refused`, so a *correction's*
/// refusal propagated here would cap the attempts of a node whose own
/// failure was retryable — and the rest say nothing about the file, which
/// the first turn already wrote or did not.
fn ended_the_node(failure: &NodeFailure) -> bool {
    matches!(
        failure,
        NodeFailure::ContextExhausted | NodeFailure::TurnLimit
    )
}

#[cfg(test)]
mod tests;
