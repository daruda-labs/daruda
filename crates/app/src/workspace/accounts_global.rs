//! App-wide managed-accounts state as a GPUI `Global` — the single source
//! of truth for `accounts.json`'s in-memory shape, shared across every
//! `Workspace` window and the Settings window.
//!
//! Mirrors the `GlobalTasks` / `SkillsState` / `McpState` pattern: one
//! process-wide global, mutated in exactly one way ([`replace`]), with each
//! window subscribing via `cx.observe_global::<AccountsGlobal>` to refresh
//! its own read-cache (`Workspace.accounts` / the Settings section's copy)
//! and repaint. This replaces the former per-window mirrors + manual
//! `broadcast_accounts_state` / `sync_accounts_state` fan-out, whose login
//! writers forgot to broadcast — leaving a login in window A invisible to
//! window B until restart. `observe_global` fires on *every* window
//! symmetrically, so no writer can forget a window.
//!
//! The window-side `accounts` field is still a cache (not deleted), exactly
//! like `Workspace`'s config fields cache `SettingsStore`: it exists so the
//! many cx-free read sites (`resolve_pane_account` callers,
//! `focused_account`, the status-bar slot) don't each need an `&App`.
//! Its *only* refresh site is the `observe_global` callback, so the mirror
//! still has a single update path (MVU "one update site for mirrored
//! state").

use gpui::{App, BorrowAppContext, Global};

use daruda_store::accounts::AccountsState;

use super::account_login_ops::LoginTarget;

/// Process-wide managed-accounts snapshot and authentication operation marker.
/// State replacement and login-slot ownership stay behind this module's API.
pub(crate) struct AccountsGlobal {
    state: AccountsState,
    login_target: Option<LoginTarget>,
}

impl Global for AccountsGlobal {}

/// Install the global from `initial` if it isn't already present
/// (idempotent). Called once at app startup (`main.rs`) and defensively
/// from `Workspace`/Settings construction so a window built before startup
/// ran — every test harness — still finds it. The first install wins: a
/// single process has a single profile (`data_dir`), so every window's
/// `initial` load is the same content.
pub(crate) fn install_if_absent(cx: &mut App, initial: AccountsState) {
    if !cx.has_global::<AccountsGlobal>() {
        cx.set_global(AccountsGlobal {
            state: initial,
            login_target: None,
        });
    }
}

/// Replace the shared accounts state, firing `observe_global` on every
/// window. The single cross-window propagation path — writers
/// (`finish_login_success`, `finish_reauth_success`, Settings
/// `set_default_account` / `remove_account`) call this after persisting to
/// disk. Falls back to `set_global` if somehow not yet installed.
pub(crate) fn replace(cx: &mut App, state: AccountsState) {
    if cx.has_global::<AccountsGlobal>() {
        cx.update_global::<AccountsGlobal, _>(|g, _| g.state = state);
    } else {
        cx.set_global(AccountsGlobal {
            state,
            login_target: None,
        });
    }
}

/// Clone of the current shared state — the value each window copies into
/// its `accounts` read-cache (on install and from `observe_global`).
pub(crate) fn snapshot(cx: &App) -> AccountsState {
    cx.global::<AccountsGlobal>().state.clone()
}

/// Reserve the single process-wide account authentication slot. The
/// Workspace still owns the process handle, while this shared marker keeps
/// Settings and other Workspace windows from starting a competing flow.
pub(in crate::workspace) fn begin_login(cx: &mut App, target: LoginTarget) -> bool {
    cx.update_global::<AccountsGlobal, bool>(|global, _| {
        if global.login_target.is_some() {
            return false;
        }
        global.login_target = Some(target);
        true
    })
}

/// Clear the authentication slot only when it still belongs to `account_id`.
/// A stale async completion must not clear a newer login.
pub(in crate::workspace) fn finish_login(cx: &mut App, target: LoginTarget) {
    cx.update_global::<AccountsGlobal, _>(|global, _| {
        clear_login_marker(global, target);
    });
}

pub(in crate::workspace) fn clear_login_marker(global: &mut AccountsGlobal, target: LoginTarget) {
    if global.login_target == Some(target) {
        global.login_target = None;
    }
}

pub(crate) fn login_busy(cx: &App) -> bool {
    cx.global::<AccountsGlobal>().login_target.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::accounts::{AccountId, AccountRecipeId};
    use gpui::TestAppContext;

    fn managed() -> LoginTarget {
        LoginTarget::Managed {
            id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
        }
    }

    #[gpui::test]
    fn login_slot_rejects_competitors_and_ignores_stale_completion(cx: &mut TestAppContext) {
        let owner = managed();
        let competitor = managed();

        cx.update(|cx| {
            install_if_absent(cx, AccountsState::default());
            assert!(begin_login(cx, owner));
            assert!(!begin_login(cx, competitor));
            finish_login(cx, competitor);
            assert!(login_busy(cx));
            finish_login(cx, owner);
            assert!(!login_busy(cx));
        });
    }
}
