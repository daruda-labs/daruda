//! Per-pane account switching (Task 8, A+C hybrid): an idle pane
//! reconnects in place under the new account; a busy pane keeps running
//! and the switch opens a fresh pane instead of tearing down live work.
//!
//! **Agent chat** panes get the full A+C treatment: idle reconnects in
//! place by reusing [`Workspace::reset_agent_chat_session`] (the same
//! production path the `/clear` slash command already exercises — no new
//! session-teardown machinery), busy opens a new pane.
//!
//! **Terminal** panes always open a new pane, regardless of
//! [`switch_kind`]'s idle/busy verdict. There is no existing safe
//! in-place PTY-respawn primitive in this codebase (tearing down and
//! rebuilding a live shell's PTY resources — master, stdout poll task,
//! stdin channel — mid-pane-lifetime is exactly the kind of invented
//! session-teardown mechanism the project avoids shipping without a
//! proven, reused path). `switch_kind` stays a pane-kind-agnostic pure
//! function either way; Terminal's dispatch simply doesn't consult it.

use gpui::{Context, Window};

use daruda_store::accounts::{AccountId, AgentProvider};

use super::SwitchPaneAccount;
use crate::surface::strings as s;
use crate::ui::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::main_area::pane;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// Whether a pane-account switch reconnects the same pane or opens a new
/// one. Purely a function of whether the pane is currently doing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum SwitchKind {
    /// Pane is idle: tear down and reconnect the same pane under the new
    /// account.
    InPlace,
    /// Pane is busy: leave it running and open a new pane under the new
    /// account instead.
    NewPane,
}

/// Pure decision: a busy pane never gets torn down mid-work, so switching
/// its account opens a new pane; an idle pane reconnects in place.
pub(in crate::workspace) fn switch_kind(pane_busy: bool) -> SwitchKind {
    if pane_busy {
        SwitchKind::NewPane
    } else {
        SwitchKind::InPlace
    }
}

/// Which pane constructor a new-pane account switch should use — mirrors
/// `main_area::tab_ops::NewPaneKind` but scoped to this module (that enum
/// also carries split-tree wiring this flow doesn't need).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewPaneAccountKind {
    Terminal,
    AgentChat,
}

impl Workspace {
    /// Action handler: switch the *focused* pane's account. Thin shim over
    /// [`Self::switch_pane_account`] — the concrete dispatch entry point is
    /// this action for a keyboard/programmatic trigger, or a direct call
    /// from the status-bar dropdown's menu-item click (which already knows
    /// the exact pane it's targeting).
    pub(in crate::workspace) fn on_switch_pane_account(
        &mut self,
        action: &SwitchPaneAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.active_runtime().focused_pane_id;
        self.switch_pane_account(pane_id, action.0, window, cx);
    }

    /// Switch `pane_id`'s account to `account_id`. Dispatches on pane kind:
    /// an Agent chat pane consults [`switch_kind`] (idle → in-place
    /// reconnect, busy → new pane); a Terminal pane always opens a new pane
    /// (see the module doc); File/TaskEdit panes don't track an account and
    /// are a no-op.
    pub(in crate::workspace) fn switch_pane_account(
        &mut self,
        pane_id: PaneId,
        account_id: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let switchable = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .is_some_and(|p| p.account_id().is_some());
        if !switchable {
            return;
        }

        if self.agent_chat_view(pane_id).is_some() {
            let busy = self.agent_chat_pane_is_busy_for_switch(pane_id, cx);
            match switch_kind(busy) {
                SwitchKind::InPlace => {
                    self.switch_agent_chat_account_in_place(pane_id, account_id, cx);
                }
                SwitchKind::NewPane => {
                    self.confirm_open_pane_with_account(
                        pane_id,
                        NewPaneAccountKind::AgentChat,
                        account_id,
                        window,
                        cx,
                    );
                }
            }
        } else {
            // Terminal pane — see the module doc: every switch opens a new
            // pane, idle or busy alike.
            self.confirm_open_pane_with_account(
                pane_id,
                NewPaneAccountKind::Terminal,
                account_id,
                window,
                cx,
            );
        }
    }

    /// Busy signal for an Agent chat pane's account switch: a live
    /// prompt/subagent turn ([`AgentChatView::is_busy`] — the single
    /// activity-state source), or a connect handshake already in flight.
    /// The latter isn't covered by `is_busy` (that's about turns, not
    /// connection lifecycle) but re-triggering `connect_agent_chat` while
    /// one is already running would race two connect attempts, so it
    /// counts as busy here too.
    fn agent_chat_pane_is_busy_for_switch(&self, pane_id: PaneId, cx: &Context<Self>) -> bool {
        let Some(view) = self.agent_chat_view(pane_id) else {
            return false;
        };
        let v = view.read(cx);
        v.is_busy()
            || matches!(
                v.status,
                AgentSessionStatus::PreparingRuntime(_)
                    | AgentSessionStatus::Connecting
                    | AgentSessionStatus::Handshaking(_)
            )
    }

    /// `SwitchKind::InPlace` for an Agent chat pane: set the pane's account
    /// override, then reuse [`Self::reset_agent_chat_session`] — the same
    /// path the `/clear` slash command runs — to tear down and reconnect
    /// the session. `connect_agent_chat` resolves the new `CLAUDE_CONFIG_DIR`
    /// from the override this just wrote (`agent_chat_account_id`), so the
    /// fresh session runs under the newly selected account.
    fn switch_agent_chat_account_in_place(
        &mut self,
        pane_id: PaneId,
        account_id: AccountId,
        cx: &mut Context<Self>,
    ) {
        {
            let Some(pane) = self
                .active_runtime_mut()
                .panes
                .iter_mut()
                .find(|p| p.id == pane_id)
            else {
                return;
            };
            let Some(content) = pane.agent_chat_content_mut() else {
                return;
            };
            content.account_id = Some(account_id);
        }
        self.reset_agent_chat_session(pane_id, cx);
    }

    /// `SwitchKind::NewPane`: confirm with the user first (G9 — this opens
    /// an extra pane they didn't explicitly ask to open), then build it via
    /// [`Self::open_new_pane_with_account`].
    fn confirm_open_pane_with_account(
        &mut self,
        source_pane_id: PaneId,
        kind: NewPaneAccountKind,
        account_id: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        open_confirm_dialog(
            s::switch_account_new_pane_title(),
            s::switch_account_new_pane_body(),
            s::switch_account_new_pane_confirm(),
            ButtonVariant::Primary,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(app_cx, |ws, cx| {
                        ws.open_new_pane_with_account(source_pane_id, kind, account_id, window, cx);
                    });
                }
            },
            window,
            cx,
        );
    }

    /// Build a fresh pane under `account_id` in the active lane's current
    /// tab set and focus it. Terminal reuses [`Self::create_pane_with_cwd`]
    /// (the same constructor `Self::create_pane` wraps); Agent chat reuses
    /// [`Self::create_new_agent_chat_pane`], inheriting `source_pane_id`'s
    /// agent (mirrors `split_focused_pane_kind`'s inheritance) and patching
    /// the account override onto the result the same way session restore
    /// does. Mirrors `add_tab` / `open_agent_chat_pane_with_agent`'s
    /// tab-append + focus flow — no new pane-opening machinery.
    fn open_new_pane_with_account(
        &mut self,
        source_pane_id: PaneId,
        kind: NewPaneAccountKind,
        account_id: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_lane_is_inaccessible() {
            return;
        }
        let new_pane = match kind {
            NewPaneAccountKind::Terminal => {
                let cwd = self.default_cwd_for_new_pane();
                let account_config_dir = pane::resolve_account_config_dir(
                    &self.accounts,
                    &self.data_dir,
                    Some(account_id),
                    AgentProvider::Claude,
                );
                match self.create_pane_with_cwd(
                    cwd,
                    Some(account_id),
                    account_config_dir.as_deref(),
                    window,
                    cx,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        self.report_pane_error("switch account", e, cx);
                        return;
                    }
                }
            }
            NewPaneAccountKind::AgentChat => {
                let (local_cwd, remote_cwd) = self.active_lane_cwds();
                let agent_id = self
                    .agent_chat_view(source_pane_id)
                    .map(|v| v.read(cx).agent_id.clone())
                    .unwrap_or_else(|| {
                        resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref())
                    });
                let mut new_pane =
                    self.create_new_agent_chat_pane(agent_id, local_cwd, remote_cwd, window, cx);
                if let Some(content) = new_pane.agent_chat_content_mut() {
                    content.account_id = Some(account_id);
                }
                new_pane
            }
        };

        let pane_id = new_pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(new_pane);
        self.active_runtime_mut().tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;

        if matches!(kind, NewPaneAccountKind::AgentChat) && !self.bottom_dock.read(cx).is_open {
            self.bottom_dock.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        self.mark_dirty_and_save(cx);
    }

    /// Count of this workspace's currently-*loaded* panes (every lane
    /// runtime visited this session, not just the active one) whose
    /// account override is `account_id`. `pub(crate)`: the Settings
    /// window (Task 9's account-delete confirm) sums this across every
    /// open `Workspace` window via `WindowRegistry::for_each_workspace`
    /// to build its confirm-body count. A lane never opened this session
    /// has no entry in `main_area.runtimes` yet, so this can undercount
    /// — an accepted simplification (see `settings_window/sections/accounts.rs`).
    pub(crate) fn panes_referencing_account(&self, account_id: AccountId) -> usize {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter(|p| p.account_id() == Some(Some(account_id)))
            .count()
    }

    /// Clear `account_id`'s override on every pane that carries it
    /// (Terminal + AgentChat, across every loaded lane runtime) — run
    /// when the Settings window deletes that account, so no pane is left
    /// pointing at a config dir that no longer exists. A Terminal pane's
    /// shell keeps running under whatever env it already spawned with;
    /// only the cached label used for persistence/display is cleared.
    pub(crate) fn clear_account_override(&mut self, account_id: AccountId, cx: &mut Context<Self>) {
        let mut changed = false;
        for rt in self.main_area.runtimes.values_mut() {
            for p in rt.panes.iter_mut() {
                match &mut p.content {
                    pane::PaneContent::Terminal(t) if t.account_id == Some(account_id) => {
                        t.account_id = None;
                        changed = true;
                    }
                    pane::PaneContent::AgentChat(ac) if ac.account_id == Some(account_id) => {
                        ac.account_id = None;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        if changed {
            self.mark_dirty_and_save(cx);
            cx.notify();
        }
    }

    /// Replace this workspace's in-memory accounts snapshot. The
    /// Settings window is the sole writer of `accounts.json` (Task 9's
    /// `SetDefaultAccount`/`RemoveAccount` handlers); this is how each
    /// open `Workspace` window's status-bar dropdown and pane-spawn
    /// resolution (`resolve_account_config_dir`) pick up the change
    /// immediately instead of waiting for their own next restore.
    pub(crate) fn sync_accounts_state(
        &mut self,
        state: daruda_store::accounts::AccountsState,
        cx: &mut Context<Self>,
    ) {
        self.accounts = state;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_kind_busy_new_pane_idle_in_place() {
        assert_eq!(switch_kind(false), SwitchKind::InPlace);
        assert_eq!(switch_kind(true), SwitchKind::NewPane);
    }
}
