//! Login attempt identity and the pure transitions of [`PendingLogin`].

use super::*;

/// One login attempt, distinct from every other — including a later attempt
/// at the same [`LoginTarget`].
///
/// The target alone used to identify the pending login, which held only while
/// a target could have one attempt at a time. Taking one over
/// ([`reclick_restarts`]) breaks that: the replaced attempt's background wait
/// is still running and resolves against whatever is pending *then*. Keyed on
/// the target it would claim its own replacement's slot, report that login's
/// failure, and leave the attempt the user is waiting on orphaned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) struct LoginAttempt(u64);

/// Mint the next attempt id. Process-wide, because the login slot is: two
/// windows must not mint the same id for different attempts.
pub(in crate::workspace) fn next_login_attempt() -> LoginAttempt {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LoginAttempt(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Whether `attempt`'s finish callback owns what is currently pending.
///
/// `Preparing` is deliberately not claimable: no process exists for it yet, so
/// a finish callback cannot belong to one.
pub(in crate::workspace) fn claims_pending(pending: &PendingLogin, attempt: LoginAttempt) -> bool {
    matches!(
        pending,
        PendingLogin::InProgress { attempt: current, .. } if *current == attempt
    )
}

/// Whether a click on sign-in for `target` should take over the attempt
/// already in flight rather than be refused as a duplicate.
///
/// True only when the pending login is for the *same* target. Clicking sign-in
/// again is the user saying the attempt they started is not going to finish —
/// the browser opened and they closed it, or it never appeared. daruda cannot
/// see that: the login process outlives the browser (its OAuth callback server
/// waits on stdin), so the slot stays taken for the whole [`LOGIN_TIMEOUT`] and
/// every later click reports "already signing in" with the only way out parked
/// in the status bar's account menu, which is not where the user is looking.
///
/// A login for a *different* target is not the user's to discard — the slot is
/// single and that attempt is somebody else's intent, so it is still refused.
pub(in crate::workspace) fn reclick_restarts(pending: &PendingLogin, target: LoginTarget) -> bool {
    match pending {
        PendingLogin::None => false,
        PendingLogin::Preparing {
            target: current, ..
        }
        | PendingLogin::InProgress {
            target: current, ..
        } => *current == target,
    }
}

/// Stop whatever `pending` is running — the login process, or the adapter
/// preparation still ahead of it — and say which attempt that was.
pub(super) fn stop_pending(
    pending: PendingLogin,
) -> Option<(LoginTarget, LoginAttempt, LoginFinish)> {
    match pending {
        PendingLogin::None => None,
        PendingLogin::Preparing {
            target,
            attempt,
            finish,
            cancellation,
        } => {
            cancellation.cancel();
            Some((target, attempt, finish))
        }
        PendingLogin::InProgress {
            target,
            attempt,
            handle,
            finish,
            ..
        } => {
            handle.cancel();
            Some((target, attempt, finish))
        }
    }
}

/// Whether a new headless add-account login is safe to start: `true` iff
/// nothing is already pending. Extracted as a pure predicate off
/// [`Workspace::add_managed_account`]'s concurrent-add guard so it's
/// unit-testable without constructing a `Workspace` — a second concurrent
/// login would overwrite `pending_login`'s [`PendingLogin::InProgress`]
/// handle before anything can cancel the first one, leaking that
/// process + its config dir.
pub(in crate::workspace) fn can_start_login(pending: &PendingLogin) -> bool {
    matches!(pending, PendingLogin::None)
}
