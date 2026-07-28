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

/// Process-wide managed-accounts snapshot. Newtype so it can be a GPUI
/// `Global` (orphan rule) and so the mutation surface is just [`replace`].
pub(crate) struct AccountsGlobal(pub(crate) AccountsState);

impl Global for AccountsGlobal {}

/// Install the global from `initial` if it isn't already present
/// (idempotent). Called once at app startup (`main.rs`) and defensively
/// from `Workspace`/Settings construction so a window built before startup
/// ran — every test harness — still finds it. The first install wins: a
/// single process has a single profile (`data_dir`), so every window's
/// `initial` load is the same content.
pub(crate) fn install_if_absent(cx: &mut App, initial: AccountsState) {
    if !cx.has_global::<AccountsGlobal>() {
        cx.set_global(AccountsGlobal(initial));
    }
}

/// Replace the shared accounts state, firing `observe_global` on every
/// window. The single cross-window propagation path — writers
/// (`finish_login_success`, `finish_reauth_success`, Settings
/// `set_default_account` / `remove_account`) call this after persisting to
/// disk. Falls back to `set_global` if somehow not yet installed.
pub(crate) fn replace(cx: &mut App, state: AccountsState) {
    if cx.has_global::<AccountsGlobal>() {
        cx.update_global::<AccountsGlobal, _>(|g, _| g.0 = state);
    } else {
        cx.set_global(AccountsGlobal(state));
    }
}

/// Clone of the current shared state — the value each window copies into
/// its `accounts` read-cache (on install and from `observe_global`).
pub(crate) fn snapshot(cx: &App) -> AccountsState {
    cx.global::<AccountsGlobal>().0.clone()
}
