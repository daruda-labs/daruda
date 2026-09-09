//! Waiting for one lane agent's answer.
//!
//! The same shape as [`super::approval`] — a table of waiting callers, one
//! bounded channel each, a single settle path, a timeout — because the problem
//! is the same: a tool call has to block on something that happens later. Only
//! the trigger differs. An approval waits for a person to tap; this waits for a
//! pane's activity span to settle, which `Workspace::fire_activity_completion`
//! is the single point of (CLAUDE.md pitfall 11).
//!
//! Keyed by pane rather than by a minted id, because the completion tee has a
//! pane and nothing else to look a waiter up by. At most one waiter per pane
//! holds by construction: a waiter is only registered when the prompt was
//! *delivered*, which means the pane was idle, and a second call arriving
//! while a turn is in flight is answered `Queued` instead.
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

/// Calls waiting on a pane's turn.
#[derive(Default)]
struct Asks {
    /// An entry's presence *is* the wait being live: whichever of the settle
    /// edge and the timeout gets here first removes it, so the loser finds
    /// nothing and does nothing.
    waiting: HashMap<PaneRef, Sender<PaneAnswer>>,
}

impl Global for Asks {}

/// Wait for `pane`'s current turn, answering on the returned channel exactly
/// once.
///
/// The deadline comes back so the caller can prune a wait it is still holding
/// a record of. Bounded at one and never awaited by the sender, so a caller
/// that has given up cannot block the resolution.
pub(crate) fn wait_for(pane: PaneRef, cx: &mut App) -> (Instant, Receiver<PaneAnswer>) {
    let (tx, rx) = smol::channel::bounded(1);
    let displaced = cx.default_global::<Asks>().waiting.insert(pane, tx);
    // Unreachable while a waiter is only registered for a delivered prompt,
    // but a stranded caller is the wrong way to be wrong: tell the displaced
    // one the turn outlived it rather than leaving it on a dead channel.
    if let Some(displaced) = displaced {
        let _ = displaced.try_send(PaneAnswer::StillWorking);
    }
    spawn_timeout(pane, cx);
    (Instant::now() + ASK_TIMEOUT, rx)
}

/// Answer the call waiting on `pane`, if there is one. `false` when nothing
/// was waiting — the ordinary case, since most turns nobody asked about.
///
/// Removing the entry *is* the answer: whichever of the settle edge and the
/// timeout arrives first wins.
pub(crate) fn resolve(pane: PaneRef, answer: PaneAnswer, cx: &mut App) -> bool {
    if !cx.has_global::<Asks>() {
        return false;
    }
    let Some(tx) = cx.global_mut::<Asks>().waiting.remove(&pane) else {
        return false;
    };
    // The receiver may be gone (the call's connection dropped). Nothing to
    // report: the answer simply has no reader.
    let _ = tx.try_send(answer);
    true
}

/// Stop waiting on `pane`. The turn keeps running — see the module note.
pub(crate) fn withdraw(pane: PaneRef, cx: &mut App) -> bool {
    if !cx.has_global::<Asks>() {
        return false;
    }
    cx.global_mut::<Asks>().waiting.remove(&pane).is_some()
}

/// Whether a call is still waiting on `pane`.
pub(crate) fn is_waiting(pane: PaneRef, cx: &App) -> bool {
    cx.try_global::<Asks>()
        .is_some_and(|a| a.waiting.contains_key(&pane))
}

/// Close the wait once the window passes, so a turn that never settles cannot
/// hold a tool call for good.
fn spawn_timeout(pane: PaneRef, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(ASK_TIMEOUT).await;
        // No `Result` to handle, for `approval::spawn_timeout`'s reason: a
        // released app drops this task before it runs. The `bool` is discarded
        // because losing the race is the normal case — the turn settled first.
        cx.update(|cx| {
            resolve(pane, PaneAnswer::StillWorking, cx);
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
