//! Cached "how did this sign-in happen" readings, app-wide.
//!
//! Kept in a Global for the same reason `accounts_global` is: the Settings
//! window shows it, a `Workspace` produces it, and neither owns the other.
//!
//! Read by *asking the CLI*, which costs a process spawn (through `npx` on a
//! default install), so this is a cache with named refresh points rather than
//! a poll: a login this app performed, and opening the Settings window. Those
//! are the only two moments the answer can have changed in a way the user is
//! about to look at.
//!
//! A scope with no entry means "not read yet", which the UI shows as nothing.
//! That is deliberately distinct from a reading that came back without a
//! method (an older CLI): the first is our ignorance, the second is theirs,
//! and neither may be rendered as a claim about how the user signed in.

use std::collections::HashMap;

use gpui::{App, BorrowAppContext, Global};

use daruda_agent::accounts::auth_status::AuthStatus;

pub(crate) use super::account_login_ops::LoginTarget;

/// Process-wide auth-status cache, keyed by the credentials it describes.
pub(crate) struct AuthStatusGlobal {
    readings: HashMap<LoginTarget, AuthStatus>,
    /// Newest probe issued per scope, and the newest that has answered.
    ///
    /// Two probes for one scope can overlap — one from opening Settings, one
    /// from a login that just landed — and they take however long a subprocess
    /// takes. Without an order the slower one wins the slot whichever it was,
    /// so a reading from *before* a login can overwrite the reading of that
    /// login. The same shape as `LoginAttempt` guards the login state machine.
    issued: HashMap<LoginTarget, u64>,
    answered: HashMap<LoginTarget, u64>,
    next: u64,
}

impl Global for AuthStatusGlobal {}

/// Install the (empty) cache if absent. Idempotent; called defensively from
/// every window that reads it, since either window kind may open first.
pub(crate) fn install_if_absent(cx: &mut App) {
    if !cx.has_global::<AuthStatusGlobal>() {
        cx.set_global(AuthStatusGlobal {
            readings: HashMap::new(),
            issued: HashMap::new(),
            answered: HashMap::new(),
            next: 1,
        });
    }
}

/// One probe about to start. `None` when a probe for this scope is already in
/// flight and `supersede` is false — an equivalent request, so the caller has
/// nothing to add by spawning a second subprocess.
///
/// `supersede: true` is for a caller whose reading is *newer than* whatever is
/// running: a login that just changed the credentials. That one has to start
/// even mid-flight, and the ticket it gets outranks the one already out.
pub(in crate::workspace) fn begin_probe(
    cx: &mut App,
    target: LoginTarget,
    supersede: bool,
) -> Option<ProbeTicket> {
    install_if_absent(cx);
    cx.update_global::<AuthStatusGlobal, Option<ProbeTicket>>(|g, _| {
        let in_flight = g.issued.get(&target).copied().unwrap_or(0)
            > g.answered.get(&target).copied().unwrap_or(0);
        if in_flight && !supersede {
            return None;
        }
        let seq = g.next;
        g.next += 1;
        g.issued.insert(target, seq);
        Some(ProbeTicket { target, seq })
    })
}

/// Permission to record one reading for one scope, in issue order.
#[derive(Debug, Clone, Copy)]
pub(in crate::workspace) struct ProbeTicket {
    target: LoginTarget,
    seq: u64,
}

/// Record a reading, firing `observe_global` on every window.
///
/// Dropped when a newer probe for the same scope has already answered: this
/// one describes credentials that have since been replaced.
pub(in crate::workspace) fn record(cx: &mut App, ticket: ProbeTicket, status: AuthStatus) {
    install_if_absent(cx);
    cx.update_global::<AuthStatusGlobal, _>(|g, _| {
        let answered = g.answered.get(&ticket.target).copied().unwrap_or(0);
        if ticket.seq < answered {
            return;
        }
        g.answered.insert(ticket.target, ticket.seq);
        g.readings.insert(ticket.target, status);
    });
}

/// Release a scope whose probe produced nothing, so a later one is not treated
/// as a duplicate of it forever.
pub(in crate::workspace) fn abandon_probe(cx: &mut App, ticket: ProbeTicket) {
    if cx.has_global::<AuthStatusGlobal>() {
        cx.update_global::<AuthStatusGlobal, _>(|g, _| {
            let answered = g.answered.get(&ticket.target).copied().unwrap_or(0);
            if ticket.seq > answered {
                g.answered.insert(ticket.target, ticket.seq);
            }
        });
    }
}

/// Forget a reading — used when the credentials it described are gone, so a
/// stale method cannot outlive the account it belonged to.
pub(in crate::workspace) fn forget(cx: &mut App, target: LoginTarget) {
    if cx.has_global::<AuthStatusGlobal>() {
        cx.update_global::<AuthStatusGlobal, _>(|g, _| {
            g.readings.remove(&target);
            g.issued.remove(&target);
            g.answered.remove(&target);
        });
    }
}

/// Clone of the current cache — what each window copies into its own mirror.
pub(crate) fn snapshot(cx: &App) -> HashMap<LoginTarget, AuthStatus> {
    cx.try_global::<AuthStatusGlobal>()
        .map(|g| g.readings.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::accounts::{AccountId, AccountRecipeId};
    use gpui::TestAppContext;

    fn system() -> LoginTarget {
        LoginTarget::System {
            recipe: AccountRecipeId::Claude,
        }
    }

    fn reading(method: &str) -> AuthStatus {
        AuthStatus {
            logged_in: true,
            auth_method: Some(method.to_owned()),
            ..AuthStatus::default()
        }
    }

    fn method(cx: &App, target: LoginTarget) -> Option<String> {
        snapshot(cx)
            .get(&target)
            .and_then(|s| s.auth_method.clone())
    }

    #[gpui::test]
    fn a_reading_is_kept_per_scope(cx: &mut TestAppContext) {
        let managed = LoginTarget::Managed {
            id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
        };
        cx.update(|cx| {
            install_if_absent(cx);
            let a = begin_probe(cx, system(), false).expect("first probe");
            let b = begin_probe(cx, managed, false).expect("other scope is unrelated");
            record(cx, a, reading("claude.ai"));
            record(cx, b, reading("console"));
            // The two scopes must not alias — one is the user's own home, the
            // other an account, and confusing them is how a metered login gets
            // reported as the free one.
            assert_eq!(method(cx, system()).as_deref(), Some("claude.ai"));
            assert_eq!(method(cx, managed).as_deref(), Some("console"));
        });
    }

    /// An equivalent request while one is in flight adds nothing but a
    /// subprocess — reopening Settings must not pile them up.
    #[gpui::test]
    fn a_duplicate_probe_is_refused_while_one_is_in_flight(cx: &mut TestAppContext) {
        cx.update(|cx| {
            install_if_absent(cx);
            let first = begin_probe(cx, system(), false).expect("first probe");
            assert!(begin_probe(cx, system(), false).is_none());
            record(cx, first, reading("claude.ai"));
            assert!(
                begin_probe(cx, system(), false).is_some(),
                "once it has answered the scope is free again"
            );
        });
    }

    /// A login just changed the credentials, so its reading has to start even
    /// with an older probe still running.
    #[gpui::test]
    fn a_login_supersedes_a_probe_in_flight(cx: &mut TestAppContext) {
        cx.update(|cx| {
            install_if_absent(cx);
            let stale = begin_probe(cx, system(), false).expect("first probe");
            let fresh = begin_probe(cx, system(), true).expect("a login outranks it");
            record(cx, fresh, reading("console"));
            // The older probe answers last, describing credentials that have
            // since been replaced — it must not win the slot.
            record(cx, stale, reading("claude.ai"));
            assert_eq!(method(cx, system()).as_deref(), Some("console"));
        });
    }

    #[gpui::test]
    fn a_later_reading_replaces_the_earlier_one(cx: &mut TestAppContext) {
        cx.update(|cx| {
            install_if_absent(cx);
            let first = begin_probe(cx, system(), false).expect("probe");
            record(cx, first, reading("claude.ai"));
            let second = begin_probe(cx, system(), false).expect("probe");
            record(cx, second, reading("console"));
            assert_eq!(method(cx, system()).as_deref(), Some("console"));
        });
    }

    /// A probe that produced nothing must free its scope, or the next one is
    /// mistaken for a duplicate forever.
    #[gpui::test]
    fn an_abandoned_probe_frees_its_scope(cx: &mut TestAppContext) {
        cx.update(|cx| {
            install_if_absent(cx);
            let ticket = begin_probe(cx, system(), false).expect("probe");
            abandon_probe(cx, ticket);
            assert!(begin_probe(cx, system(), false).is_some());
            assert!(method(cx, system()).is_none(), "and records nothing");
        });
    }

    /// A deleted account's method must not outlive it — the next account to
    /// take that scope would inherit a claim about credentials it never had.
    #[gpui::test]
    fn a_forgotten_scope_reads_as_unknown(cx: &mut TestAppContext) {
        cx.update(|cx| {
            install_if_absent(cx);
            let ticket = begin_probe(cx, system(), false).expect("probe");
            record(cx, ticket, reading("claude.ai"));
            forget(cx, system());
            assert!(!snapshot(cx).contains_key(&system()));
            assert!(
                begin_probe(cx, system(), false).is_some(),
                "and the scope is probeable again"
            );
        });
    }

    /// Reading before anything installed the cache is "not read yet", not a
    /// panic — the Settings window can render before any Workspace exists.
    #[gpui::test]
    fn an_absent_cache_is_empty_rather_than_missing(cx: &mut TestAppContext) {
        cx.update(|cx| assert!(snapshot(cx).is_empty()));
    }
}
