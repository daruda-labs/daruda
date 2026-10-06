//! Waiting for one lane agent's answer.
//!
//! The same shape as [`super::approval`] — a table of waiting callers, one
//! bounded channel each, a single settle path, a timeout — because the problem
//! is the same: a tool call has to block on something that happens later. Only
//! the trigger differs. An approval waits for a person to tap; this waits for a
//! pane's activity span to settle, which `Workspace::fire_activity_completion`
//! is the single point of (CLAUDE.md pitfall 11).
//!
//! Keyed by pane, because the completion tee has a pane and nothing else to
//! look a waiter up by. At most one waiter per pane holds by construction: a
//! waiter is only registered when the prompt was *delivered*, which means the
//! pane was idle, and a second call arriving while a turn is in flight is
//! answered `Queued` instead.
//!
//! But a pane is unique in space, not in time. A second ask on the same pane
//! later is a *different* wait, so every entry carries an [`AskId`] and
//! anything that names a wait from outside — a timer armed 300 s ago, a
//! cancellation for a call already answered — has to match it. Without that a
//! settled wait's timer answers whichever wait happens to be there when it
//! fires, and the second ask on a pane is cut short by the first ask's clock.
//!
//! Cancelling the tool call does **not** cancel the turn. The turn belongs to
//! the pane, and the person at the desk — or another surface — may still want
//! it; giving up on hearing about it is all a withdrawal can mean here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{App, Global};
use smol::channel::{Receiver, Sender};

use crate::control::result::PaneAnswer;
use crate::telegram::bridge::PaneRef;

/// How long a call waits for the turn it started.
///
/// Its own constant, not [`super::approval::APPROVAL_TIMEOUT`], despite
/// matching it today: that one bounds how long a person is given to answer,
/// this one bounds how long a turn is given to finish, and the two would move
/// for different reasons.
///
/// Passing it is not a failure — the turn is still running, and the answer is
/// still readable later — so the ceiling the orchestrator's own MCP client
/// imposes (unknown to us, and possibly lower) degrades the same benign way.
pub(crate) const ASK_TIMEOUT: Duration = Duration::from_secs(300);

/// Which wait, not just which pane. Minted per registration so a timer or a
/// cancellation that outlived its wait cannot answer the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AskId(u64);

/// Calls waiting on a pane's turn.
#[derive(Default)]
struct Asks {
    next_id: u64,
    /// An entry's presence *is* the wait being live: whichever of the settle
    /// edge and the timeout gets here first removes it, so the loser finds
    /// nothing and does nothing.
    ///
    /// `None` on the channel is a caller that took its call back. It is owed
    /// no reply at all — it has freed its request id — which no [`PaneAnswer`]
    /// can express, so the channel carries the absence instead.
    waiting: HashMap<PaneRef, (AskId, Sender<Option<PaneAnswer>>)>,
}

impl Global for Asks {}

/// Wait for `pane`'s current turn, answering on the returned channel exactly
/// once.
///
/// The id and deadline come back so the caller can name *this* wait later —
/// to prune a record of it, or to take it back. Bounded at one and never
/// awaited by the sender, so a caller that has given up cannot block the
/// resolution.
pub(crate) fn wait_for(
    pane: PaneRef,
    cx: &mut App,
) -> (AskId, Instant, Receiver<Option<PaneAnswer>>) {
    let (tx, rx) = smol::channel::bounded(1);
    let asks = cx.default_global::<Asks>();
    asks.next_id += 1;
    let id = AskId(asks.next_id);
    let displaced = asks.waiting.insert(pane, (id, tx));
    // Unreachable while a waiter is only registered for a delivered prompt,
    // but a stranded caller is the wrong way to be wrong: tell the displaced
    // one the turn outlived it rather than leaving it on a dead channel.
    if let Some((_, displaced)) = displaced {
        let _ = displaced.try_send(Some(PaneAnswer::StillWorking));
    }
    spawn_timeout(pane, id, cx);
    (id, Instant::now() + ASK_TIMEOUT, rx)
}

/// Answer whatever call is waiting on `pane`. `false` when none is — the
/// ordinary case, since most turns nobody asked about.
///
/// Takes no [`AskId`]: this is the settle edge, which knows only the pane, and
/// the wait registered there *is* the one that turn belongs to. Everything
/// that names a wait from further away goes through [`resolve_if`].
pub(crate) fn resolve(pane: PaneRef, answer: PaneAnswer, cx: &mut App) -> bool {
    settle(pane, None, Some(answer), cx)
}

/// Answer `pane`'s wait only if it is still the one `id` names.
///
/// For a caller that has held onto an id across time — the timeout. Without
/// the check, a timer armed for a wait that settled long ago would answer
/// whichever wait is there when it fires.
pub(crate) fn resolve_if(pane: PaneRef, id: AskId, answer: PaneAnswer, cx: &mut App) -> bool {
    settle(pane, Some(id), Some(answer), cx)
}

/// Stop waiting on `pane`, if `id` still names its wait. The turn keeps
/// running — see the module note.
///
/// Sends the withdrawal rather than only dropping the sender: a dropped
/// channel is how the app going away looks, and a caller owed silence must not
/// be told its target vanished.
pub(crate) fn withdraw(pane: PaneRef, id: AskId, cx: &mut App) -> bool {
    settle(pane, Some(id), None, cx)
}

/// Whether the wait `id` names is still live on `pane`.
pub(crate) fn is_waiting(pane: PaneRef, id: AskId, cx: &App) -> bool {
    cx.try_global::<Asks>()
        .and_then(|a| a.waiting.get(&pane))
        .is_some_and(|(live, _)| *live == id)
}

/// Whether *any* call is waiting on `pane`.
///
/// The settle tee's early-out: reading a pane's transcript costs a scan, and
/// most turns nobody asked about. Deliberately id-free — the tee has no id and
/// wants the cheap question, not the precise one.
pub(crate) fn has_waiter(pane: PaneRef, cx: &App) -> bool {
    cx.try_global::<Asks>()
        .is_some_and(|a| a.waiting.contains_key(&pane))
}

/// Take `pane`'s wait off the table and send `answer` on its channel.
///
/// `expect` gates on identity when the caller has one; `answer` is `None` for a
/// withdrawal. Removing the entry *is* the decision — whichever caller arrives
/// first wins and the rest find nothing.
fn settle(pane: PaneRef, expect: Option<AskId>, answer: Option<PaneAnswer>, cx: &mut App) -> bool {
    if !cx.has_global::<Asks>() {
        return false;
    }
    let asks = cx.global_mut::<Asks>();
    match asks.waiting.get(&pane) {
        Some((live, _)) if expect.is_none_or(|id| *live == id) => {}
        _ => return false,
    }
    let Some((_, tx)) = asks.waiting.remove(&pane) else {
        return false;
    };
    // The receiver may be gone (the call's connection dropped). Nothing to
    // report: the answer simply has no reader.
    let _ = tx.try_send(answer);
    true
}

/// Close the wait once the window passes, so a turn that never settles cannot
/// hold a tool call for good.
fn spawn_timeout(pane: PaneRef, id: AskId, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ASK_TIMEOUT).await;
        // No `Result` to handle, for `approval::spawn_timeout`'s reason: a
        // released app drops this task before it runs. The `bool` is discarded
        // because losing the race is the normal case — the turn settled first,
        // and `resolve_if` makes that a no-op rather than a misfire.
        cx.update(|cx| {
            resolve_if(pane, id, PaneAnswer::StillWorking, cx);
        });
    })
    .detach();
}

#[cfg(test)]
pub(crate) fn waiting_count_for_test(cx: &App) -> usize {
    cx.try_global::<Asks>().map_or(0, |a| a.waiting.len())
}

#[cfg(test)]
mod tests;
