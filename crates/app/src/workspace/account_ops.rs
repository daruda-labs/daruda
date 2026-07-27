//! Per-pane account switching. The invariant: **a switch never destroys
//! work.** A pane holding something — a live turn or a conversation — keeps
//! it, and the switch opens a fresh pane under the new account instead. Only
//! a pane with nothing to lose reconnects in place.
//!
//! **Agent chat** panes let [`switch_kind`] decide. In place reuses
//! [`Workspace::reset_agent_chat_session`] (the same production path the
//! `/clear` slash command already exercises — no new session-teardown
//! machinery), which is safe precisely because there is no transcript to
//! lose. A conversation can never follow the user to another account: the new
//! account is a different `CLAUDE_CONFIG_DIR`, so it needs its own ACP session
//! (neither `session/load` nor a resume crosses accounts) and the transcript
//! stays readable only in the pane that owns it. Wanting the same pane back is
//! spelled `/clear` first — that leaves nothing to protect, so the switch that
//! follows is an in-place one.
//!
//! **Terminal** panes always open a new pane, without consulting
//! [`switch_kind`]. There is no existing safe in-place PTY-respawn primitive
//! in this codebase (tearing down and rebuilding a live shell's PTY resources
//! — master, stdout poll task, stdin channel — mid-pane-lifetime is exactly
//! the kind of invented session-teardown mechanism the project avoids shipping
//! without a proven, reused path).
//!
//! This file also owns the per-window delete-cleanup side of account
//! state ([`Workspace::clear_account_override`] — resetting a deleted
//! account's panes back to the system default and pruning its usage
//! cache). The shared [`daruda_store::accounts::AccountsState`] itself is
//! propagated app-wide through [`super::accounts_global`] (a GPUI Global +
//! `observe_global`), not a per-window broadcast. The headless add-account
//! / reauthenticate
//! *login* flow that creates and refreshes accounts in the first place
//! lives in the sibling `account_login_ops.rs` — split out because it is
//! a distinct, self-contained domain (spawn/poll/finish machinery) large
//! enough to warrant its own file; both mirror
//! `settings_window/sections/accounts.rs` owning the Settings-side half
//! of the same overall account domain (default/delete).

use gpui::{Context, Window};

use daruda_store::accounts::{AccountId, AccountSelection, AgentProvider};

use super::SwitchPaneAccount;
use crate::surface::strings as s;
use crate::ui::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::has_conversation;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::main_area::pane;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// What a pane would lose if the switch tore it down — so it doesn't. Also
/// selects which pane constructor the new pane uses and which sentence the
/// confirm dialog shows, keeping the two in lockstep (a Terminal cause can
/// never draw an Agent chat body, or vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum NewPaneCause {
    /// Terminal pane: no safe in-place PTY respawn exists — see the module doc.
    Terminal,
    /// Agent chat mid-turn: a teardown would interrupt live work.
    AgentChatBusy,
    /// Agent chat holding a transcript the new account's session can't carry.
    AgentChatConversation,
}

/// Whether a pane-account switch reconnects the same pane or opens a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum SwitchKind {
    /// Nothing to lose: tear down and reconnect the same pane under the new
    /// account.
    InPlace,
    /// Something must survive: leave the pane alone and open a new one under
    /// the new account instead.
    NewPane(NewPaneCause),
}

/// Pure decision for an Agent chat pane. Busy work and a conversation are both
/// things a switch must not destroy, so each keeps its pane; a pane with
/// neither — choosing an account before the first prompt, the common case —
/// reconnects in place.
pub(in crate::workspace) fn switch_kind(pane_busy: bool, holds_conversation: bool) -> SwitchKind {
    if pane_busy {
        SwitchKind::NewPane(NewPaneCause::AgentChatBusy)
    } else if holds_conversation {
        SwitchKind::NewPane(NewPaneCause::AgentChatConversation)
    } else {
        SwitchKind::InPlace
    }
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

    /// Switch `pane_id`'s account to `selection`
    /// ([`AccountSelection::SystemDefault`] = `~/.claude`). Dispatches on pane
    /// kind: an Agent chat pane consults [`switch_kind`]; a Terminal pane
    /// always opens a new pane (see the module doc); File/TaskEdit panes don't
    /// track an account and are a no-op.
    pub(in crate::workspace) fn switch_pane_account(
        &mut self,
        pane_id: PaneId,
        selection: AccountSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.account_selection())
        else {
            return; // pane gone, or a kind that tracks no account
        };
        // The dropdown lists the pane's current account as a clickable entry,
        // and every branch below either tears a session down or opens an extra
        // pane — so a re-pick of the account already in use stops here.
        if current == selection {
            return;
        }

        let kind = if self.agent_chat_view(pane_id).is_some() {
            self.agent_chat_switch_kind(pane_id, cx)
        } else {
            SwitchKind::NewPane(NewPaneCause::Terminal)
        };
        match kind {
            SwitchKind::InPlace => {
                self.switch_agent_chat_account_in_place(pane_id, selection, cx);
            }
            SwitchKind::NewPane(cause) => {
                self.confirm_open_pane_with_account(pane_id, cause, selection, window, cx);
            }
        }
    }

    /// [`switch_kind`] for an Agent chat pane, reading both of its inputs in
    /// one borrow.
    ///
    /// Busy is [`AgentChatView::is_busy`] (the single activity-state source)
    /// plus a connect handshake already in flight: that is connection
    /// lifecycle rather than a turn, so `is_busy` doesn't cover it, but
    /// re-triggering `connect_agent_chat` under one would race two connect
    /// attempts.
    fn agent_chat_switch_kind(&self, pane_id: PaneId, cx: &Context<Self>) -> SwitchKind {
        let Some(view) = self.agent_chat_view(pane_id) else {
            return SwitchKind::InPlace;
        };
        let v = view.read(cx);
        let busy = v.is_busy()
            || matches!(
                v.status,
                AgentSessionStatus::PreparingRuntime(_)
                    | AgentSessionStatus::Connecting
                    | AgentSessionStatus::Handshaking(_)
            );
        switch_kind(busy, has_conversation(&v.items))
    }

    /// `SwitchKind::InPlace` for an Agent chat pane: set the pane's account
    /// selection, then reuse [`Self::reset_agent_chat_session`] — the same
    /// path the `/clear` slash command runs — to tear down and reconnect the
    /// session. `connect_agent_chat` resolves the new `CLAUDE_CONFIG_DIR`
    /// from the selection this just wrote (`agent_chat_account_selection`),
    /// so the fresh session runs under the newly selected account (or
    /// `~/.claude` for [`AccountSelection::SystemDefault`]).
    fn switch_agent_chat_account_in_place(
        &mut self,
        pane_id: PaneId,
        selection: AccountSelection,
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
            content.account = selection;
        }
        self.reset_agent_chat_session(pane_id, cx);
    }

    /// `SwitchKind::NewPane`: confirm with the user first (G9 — this opens
    /// an extra pane they didn't explicitly ask to open), then build it via
    /// [`Self::open_new_pane_with_account`]. The body names the actual reason
    /// this pane can't be reused, since the three read very differently to a
    /// user deciding whether to go ahead.
    fn confirm_open_pane_with_account(
        &mut self,
        source_pane_id: PaneId,
        cause: NewPaneCause,
        selection: AccountSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = match cause {
            NewPaneCause::Terminal => s::switch_account_new_pane_body_terminal(),
            NewPaneCause::AgentChatBusy => s::switch_account_new_pane_body_busy(),
            NewPaneCause::AgentChatConversation => s::switch_account_new_pane_body_conversation(),
        };
        let weak = cx.weak_entity();
        open_confirm_dialog(
            s::switch_account_new_pane_title(),
            body,
            s::switch_account_new_pane_confirm(),
            ButtonVariant::Primary,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(app_cx, |ws, cx| {
                        ws.open_new_pane_with_account(source_pane_id, cause, selection, window, cx);
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
        cause: NewPaneCause,
        selection: AccountSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_lane_is_inaccessible() {
            return;
        }
        let new_pane = match cause {
            NewPaneCause::Terminal => {
                let cwd = self.default_cwd_for_new_pane();
                let account_config_dir = pane::resolve_account_config_dir(
                    &self.accounts,
                    &self.data_dir,
                    selection,
                    AgentProvider::Claude,
                );
                match self.create_pane_with_cwd(
                    cwd,
                    selection,
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
            NewPaneCause::AgentChatBusy | NewPaneCause::AgentChatConversation => {
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
                    content.account = selection;
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

        if !matches!(cause, NewPaneCause::Terminal) && !self.bottom_dock.read(cx).is_open {
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
            .filter(|p| p.account_selection() == Some(AccountSelection::Managed(account_id)))
            .count()
    }

    /// Reset every pane pinned to `account_id` back to
    /// [`AccountSelection::SystemDefault`] (Terminal + AgentChat, across
    /// every loaded lane runtime) — run when the Settings window deletes
    /// that account, so no pane is left pointing at a config dir that no
    /// longer exists. A Terminal pane's shell keeps running under whatever
    /// env it already spawned with; only the cached selection used for
    /// persistence/display is reset.
    ///
    /// Also prunes this account's entries from the per-account usage caches
    /// (`self.claude.usage_by_account`) — this is the per-window delete hook,
    /// so it is the only place that sees both the deleted `account_id` and
    /// `self.claude`. The system-default entry is never touched.
    pub(crate) fn clear_account_override(&mut self, account_id: AccountId, cx: &mut Context<Self>) {
        let mut changed = false;
        for rt in self.main_area.runtimes.values_mut() {
            for p in rt.panes.iter_mut() {
                match &mut p.content {
                    pane::PaneContent::Terminal(t)
                        if t.account == AccountSelection::Managed(account_id) =>
                    {
                        t.account = AccountSelection::SystemDefault;
                        changed = true;
                    }
                    pane::PaneContent::AgentChat(ac)
                        if ac.account == AccountSelection::Managed(account_id) =>
                    {
                        ac.account = AccountSelection::SystemDefault;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        self.claude
            .usage_by_account
            .remove(AccountSelection::Managed(account_id));
        if changed {
            self.mark_dirty_and_save(cx);
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_kind_reconnects_in_place_only_with_nothing_to_lose() {
        assert_eq!(switch_kind(false, false), SwitchKind::InPlace);
    }

    #[test]
    fn switch_kind_protects_a_conversation_on_an_idle_pane() {
        assert_eq!(
            switch_kind(false, true),
            SwitchKind::NewPane(NewPaneCause::AgentChatConversation)
        );
    }

    #[test]
    fn switch_kind_protects_live_work_whatever_the_transcript_holds() {
        for holds_conversation in [false, true] {
            assert_eq!(
                switch_kind(true, holds_conversation),
                SwitchKind::NewPane(NewPaneCause::AgentChatBusy),
                "a busy pane is never torn down mid-work"
            );
        }
    }
}
