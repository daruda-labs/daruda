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
//!
//! A second responsibility lives in this same file (per the Plan B SDD
//! task brief): [`Workspace::add_managed_account`] orchestrates the
//! headless add-account login itself — spawning the login process,
//! tracking it in `Workspace::pending_login`, and folding the result back
//! into `Workspace::accounts` via [`Workspace::finish_login`] /
//! [`Workspace::cancel_pending_login`]. It shares this file with the
//! account-*switching* logic above because account state (creation,
//! switching, deletion) is one cohesive domain on `Workspace`, mirrored by
//! `settings_window/sections/accounts.rs` owning the Settings-side half of
//! the same domain (default/delete).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{Context, Window};

use daruda_claude::accounts::{
    LoginOutcome, account_config_dir, delete_scoped_credentials, read_account_identity,
    read_scoped_credentials, spawn_login,
};
use daruda_store::accounts::{AccountId, AccountsState, AgentProvider, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::{AddManagedAccount, PendingLogin, ReauthenticateAccount, SwitchPaneAccount};
use crate::surface::strings as s;
use crate::ui::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;
use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;
use crate::workspace::main_area::pane;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// Timeout for a headless add-account login before it's treated as hung
/// and cancelled. Generous — the flow blocks on the user completing OAuth
/// consent by hand in a system browser window, not on anything this app
/// controls the pace of.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Which of the two headless login flows a [`PendingLogin::InProgress`] is
/// tracking. Carried alongside `account_id` so
/// [`Workspace::cancel_pending_login`] can tell apart an add-account
/// login's throwaway config dir (safe to delete on cancel) from a
/// reauthenticate login's real, permanent one (must survive a cancel) —
/// see [`cleanup_dir_on_cancel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum LoginMode {
    Add,
    Reauth,
}

/// Whether cancelling a pending login should remove its `account_id`'s
/// config dir (+ scoped Keychain item): only for [`LoginMode::Add`], whose
/// `account_id` names a throwaway dir that never became a kept
/// [`ManagedAccount`]. For [`LoginMode::Reauth`] the same `account_id`
/// names an existing account's real credentials — deleting them on cancel
/// would be permanent data loss for a still-good account (see
/// [`Workspace::reauthenticate_account`]'s doc for why
/// [`Workspace::finish_reauth_failed`] already avoids this on the
/// login-failed path; cancel must match it).
pub(in crate::workspace) fn cleanup_dir_on_cancel(mode: LoginMode) -> bool {
    matches!(mode, LoginMode::Add)
}

// The [`LoginOutcome::Denied`] / [`LoginOutcome::TimedOut`] toast-body
// detail folded into the failure toast's `.message()` is authored,
// user-visible copy — it goes through `s::settings_accounts_login_denied_detail()`
// / `s::settings_accounts_login_timed_out_detail()` (i18n), not a fixed
// English constant here. [`LoginOutcome::Failed`]'s captured-output
// string stays as-is: it's a diagnostic dump, not an authored sentence.

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
    /// Settings window's `SetDefaultAccount`/`RemoveAccount` handlers (Task
    /// 9) broadcast through this after every `accounts.json` write; this is
    /// how each open `Workspace` window's status-bar dropdown and
    /// pane-spawn resolution (`resolve_account_config_dir`) pick up the
    /// change immediately instead of waiting for their own next restore.
    /// `Workspace::add_managed_account`'s own login flow (Plan B) writes
    /// `accounts.json` too, but only ever to *this* window's own
    /// `self.accounts` — it does not broadcast to other open windows (see
    /// that method's doc for why).
    pub(crate) fn sync_accounts_state(
        &mut self,
        state: daruda_store::accounts::AccountsState,
        cx: &mut Context<Self>,
    ) {
        self.accounts = state;
        cx.notify();
    }

    /// Action handler for [`AddManagedAccount`]. Thin shim, mirroring
    /// [`Self::on_switch_pane_account`].
    pub(in crate::workspace) fn on_add_managed_account(
        &mut self,
        action: &AddManagedAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_managed_account(action.0, window, cx);
    }

    /// The interactive login command for the session's currently
    /// active/configured agent (`last_agent_id`, same resolution
    /// `add_managed_account` spawns from), or `None` when that agent's
    /// launch is remote (SSH/Docker) or not found in the catalog. Single
    /// source both `add_managed_account`'s own spawn and the status-bar
    /// dropdown's disabled state read, so the two never drift.
    pub(in crate::workspace) fn active_agent_login_command(&self) -> Option<String> {
        let agent_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
        self.agent_launch_for(&agent_id)
            .and_then(|launch| launch.login_command())
    }

    /// Whether the status-bar dropdown's "+ Add account" entry should
    /// render disabled: `true` when [`Self::active_agent_login_command`]
    /// has nothing to run (remote agent, or the configured agent isn't in
    /// the catalog). `add_managed_account` re-checks the same condition
    /// before spawning (defensive second gate — see its doc), so this is
    /// purely a UI-affordance hint, not the enforcement point.
    pub(in crate::workspace) fn active_agent_login_unavailable(&self) -> bool {
        self.active_agent_login_command().is_none()
    }

    /// Whether a headless add-account login is currently in flight —
    /// drives the status-bar dropdown's spinner + Cancel row in place of
    /// "+ Add account" (see [`PendingLogin`]).
    pub(in crate::workspace) fn is_login_pending(&self) -> bool {
        !can_start_login(&self.pending_login)
    }

    /// Start a headless add-account login (Plan B, A2 approach): spawn the
    /// currently active/configured agent's login command
    /// (`AgentLaunch::login_command`) with a fresh per-account config dir
    /// and the isolating env (`daruda_config::account_env`), stash a
    /// cancel handle in `pending_login` (drives the status-bar dropdown's
    /// spinner + Cancel row via [`Self::is_login_pending`]), and let the
    /// process run to completion on the background executor.
    /// [`Self::finish_login`] picks the result back up on this `Workspace`
    /// once the wait resolves.
    ///
    /// Rejects up front (toast, no process spawned) when the active
    /// agent's launch is remote (SSH/Docker) — `login_command()` returns
    /// `None` there since a remote adapter has no local desktop browser to
    /// complete OAuth in. The UI is expected to disable the "+ Add
    /// account" affordance for a remote agent already; this is a defensive
    /// second check, not the primary gate.
    /// `pub(crate)`: the Settings window's Accounts section (Task 5) also
    /// calls this directly — via `WindowRegistry::first_workspace` — since
    /// the headless login has no other entry point that reaches a
    /// specific `Workspace` from outside `crate::workspace` (see that
    /// section's `start_add_account`).
    pub(crate) fn add_managed_account(
        &mut self,
        provider: AgentProvider,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_start_login(&self.pending_login) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        let Some(command) = self.active_agent_login_command() else {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_remote_unsupported())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.add.remote_unsupported")
                    .build(),
                cx,
            );
            return;
        };

        let account_id = AccountId::new();
        let config_dir = account_config_dir(&self.data_dir, account_id);
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_failed())
                    .message(format!(
                        "could not create the account config directory: {e}"
                    ))
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            return;
        }

        let env = daruda_config::account_env(&config_dir);

        // Stash `Preparing` before the async node-resolve gap even starts —
        // it blocks a second concurrent login (`can_start_login`) and gives
        // the status-bar dropdown its spinner immediately, rather than only
        // once a process actually exists. See `PendingLogin::Preparing`'s
        // doc for why this state exists at all (`resolve_node_path_env` is
        // blocking and may download Node.js, so it can't run inline here on
        // the UI thread).
        self.pending_login = PendingLogin::Preparing {
            account_id,
            mode: LoginMode::Add,
        };
        cx.notify();

        let resolve_command = command.clone();
        cx.spawn(async move |this, cx| {
            let path_pair = cx
                .background_executor()
                .spawn(async move { resolve_node_path_env(&resolve_command) })
                .await;

            let closed_window_cleanup_path = config_dir.clone();
            let updated = this.update(cx, move |ws, cx| {
                // The user may have cancelled during the resolve (or, in
                // principle, a newer call superseded it) — bail without
                // touching anything if this `Preparing` isn't the one that
                // was staged above.
                let is_current = matches!(
                    &ws.pending_login,
                    PendingLogin::Preparing { account_id: current, .. }
                        if *current == account_id
                );
                if !is_current {
                    return;
                }

                let mut inject = env.inject.clone();
                if let Some(pair) = path_pair {
                    inject.push(pair);
                }

                match spawn_login(&command, &config_dir, &inject, &env.strip, LOGIN_TIMEOUT) {
                    Ok(login_process) => {
                        let handle = login_process.handle();
                        ws.pending_login = PendingLogin::InProgress {
                            account_id,
                            handle,
                            mode: LoginMode::Add,
                        };
                        cx.notify();

                        let wait_config_dir = config_dir.clone();
                        cx.spawn(async move |this, cx| {
                            let outcome = cx
                                .background_executor()
                                .spawn(async move { login_process.wait() })
                                .await;
                            let cleanup_path = wait_config_dir.clone();
                            if this
                                .update(cx, |ws, cx| {
                                    ws.finish_login(
                                        account_id,
                                        wait_config_dir,
                                        provider,
                                        outcome,
                                        cx,
                                    )
                                })
                                .is_err()
                            {
                                // Workspace window closed mid-login:
                                // `finish_login`'s own cleanup never ran, so
                                // run it directly here (no `self` needed) —
                                // otherwise a closed-window login attempt
                                // would leak an orphaned per-account config
                                // dir.
                                cleanup_account_dir(&cleanup_path);
                            }
                        })
                        .detach();
                    }
                    Err(e) => {
                        cleanup_account_dir(&config_dir);
                        ws.pending_login = PendingLogin::None;
                        ws.report_error(
                            ErrorReport::new(s::settings_accounts_login_failed())
                                .message(e.to_string())
                                .severity(ErrorSeverity::Error)
                                .at(file!(), line!())
                                .build(),
                            cx,
                        );
                        cx.notify();
                    }
                }
            });

            if updated.is_err() {
                // Workspace window closed during the node resolve, before
                // `spawn_login` ever ran: nothing was spawned to leak a
                // process, but the throwaway config dir created above still
                // needs the same cleanup the wait-path's closed-window
                // fallback does.
                cleanup_account_dir(&closed_window_cleanup_path);
            }
        })
        .detach();
    }

    /// Picks up a headless login's result on this `Workspace`, once
    /// [`Self::add_managed_account`]'s background `wait()` resolves.
    ///
    /// Guards against a stale callback first: if `pending_login` no longer
    /// points at `account_id`, this login was already handled —
    /// [`Self::cancel_pending_login`] already cancelled + cleaned it up,
    /// or a newer `add_managed_account` call superseded it — so running
    /// this path anyway would either double-cleanup an already-removed
    /// dir or, worse, clobber a newer `InProgress` this callback knows
    /// nothing about. In that case this is a no-op.
    pub(in crate::workspace) fn finish_login(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        provider: AgentProvider,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let is_current = matches!(
            &self.pending_login,
            PendingLogin::InProgress { account_id: current, .. } if *current == account_id
        );
        if !is_current {
            return;
        }
        self.pending_login = PendingLogin::None;

        match outcome {
            LoginOutcome::Success => {
                self.finish_login_success(account_id, config_dir, provider, cx)
            }
            LoginOutcome::Denied => {
                self.finish_login_failed(config_dir, s::settings_accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => self.finish_login_failed(
                config_dir,
                s::settings_accounts_login_timed_out_detail(),
                cx,
            ),
            LoginOutcome::Failed(detail) => self.finish_login_failed(config_dir, detail, cx),
        }
    }

    /// `LoginOutcome::Success` half of [`Self::finish_login`]: confirms
    /// credentials actually landed, then either bumps an existing
    /// duplicate account's freshness or files a brand-new
    /// [`ManagedAccount`] (making it the provider default when it's the
    /// first account for `provider`), and persists.
    fn finish_login_success(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        provider: AgentProvider,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = read_scoped_credentials(&config_dir) {
            self.finish_login_failed(
                config_dir,
                format!("Login succeeded but no credentials were found afterward: {e}"),
                cx,
            );
            return;
        }

        let identity = read_account_identity(&config_dir);
        let now = now_unix();

        // Re-read the on-disk state and apply this login's change to it,
        // rather than overwriting accounts.json with a possibly-stale
        // in-memory snapshot: another window (Settings, or a second
        // Workspace) may have written the file since this login started,
        // and a full-overwrite from `self.accounts` would silently destroy
        // whatever it added. Applying the delta to the fresh state, then
        // adopting it as the new `self.accounts`, keeps the disk correct
        // regardless of interleaving.
        let mut state =
            daruda_store::accounts::load_accounts_in(&self.data_dir).unwrap_or_default();

        match find_duplicate(
            &state,
            identity.email.as_deref(),
            identity.organization.as_deref(),
        ) {
            Some(existing_id) => {
                if let Some(existing) = state.accounts.iter_mut().find(|a| a.id == existing_id) {
                    existing.last_authenticated_at = now;
                }
                // The freshly logged-in config dir duplicates an account
                // already tracked under a different dir — nothing new to
                // keep.
                cleanup_account_dir(&config_dir);
            }
            None => {
                let is_first_for_provider = !state.accounts.iter().any(|a| a.provider == provider);
                state.accounts.push(ManagedAccount {
                    id: account_id,
                    provider,
                    email: identity.email,
                    organization: identity.organization,
                    config_dir: config_dir.clone(),
                    created_at: now,
                    last_authenticated_at: now,
                });
                if is_first_for_provider {
                    state.default_by_provider.insert(provider, account_id);
                }
            }
        }

        if let Err(e) = daruda_store::accounts::save_accounts_in(&self.data_dir, &state) {
            log_io_error(
                "Failed to save accounts.json after add-account login",
                "account.add.save_failed",
                &e,
            );
        }
        self.accounts = state;

        self.report_error(
            ErrorReport::new(s::settings_accounts_login_added())
                .severity(ErrorSeverity::Info)
                .build(),
            cx,
        );
        cx.notify();
    }

    /// Shared failure tail for [`Self::finish_login`] (`Denied`/`TimedOut`/
    /// `Failed`) and [`Self::finish_login_success`]'s own credentials-check
    /// failure: best-effort remove the config dir, surface a toast, notify.
    fn finish_login_failed(&mut self, config_dir: PathBuf, detail: String, cx: &mut Context<Self>) {
        cleanup_account_dir(&config_dir);
        self.report_error(
            ErrorReport::new(s::settings_accounts_login_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
    }

    /// Cancel the in-flight headless login, if any: clear `pending_login`
    /// and notify, killing the child process via its handle when one
    /// already exists. A no-op when nothing is pending.
    ///
    /// Covers both pending states: [`PendingLogin::Preparing`] (the
    /// async managed-node resolve is still running on the background
    /// executor — see [`resolve_node_path_env`] — and no process has been
    /// spawned yet, so there is no handle to kill; the re-entry that
    /// eventually lands from `add_managed_account` / `reauthenticate_account`
    /// finds `pending_login` no longer matches `account_id` and bails,
    /// same staleness guard as below) and [`PendingLogin::InProgress`]
    /// (a real login process is running; kill it via its handle).
    ///
    /// Whether the config dir it was writing into (recomputed from
    /// `account_id` — see [`PendingLogin`]'s doc for why it isn't stored
    /// redundantly) is also removed depends on [`LoginMode`]
    /// ([`cleanup_dir_on_cancel`]): for [`LoginMode::Add`] that dir is
    /// throwaway and safe to delete; for [`LoginMode::Reauth`] it is the
    /// account's real, permanent config dir + Keychain item, so cancel
    /// must leave it untouched — deleting it would silently destroy a
    /// still-good account's credentials with no way back. This mirrors
    /// [`Self::finish_reauth_failed`], which avoids the same cleanup on
    /// the login-failed path for the same reason.
    ///
    /// The killed process's own `wait()` (still running on the background
    /// executor from `add_managed_account` / `reauthenticate_account`)
    /// resolves shortly after with a `Failed` outcome; `finish_login` /
    /// `finish_reauth`'s staleness guard sees `pending_login` already
    /// cleared and skips it, so it never double-cleans or shows a
    /// redundant failure toast for a login the user explicitly cancelled.
    ///
    /// Wired to the status-bar dropdown's Cancel row
    /// (`status_bar::build_account_menu`), shown in place of "+ Add
    /// account" while [`Self::is_login_pending`] is true.
    pub(in crate::workspace) fn cancel_pending_login(&mut self, cx: &mut Context<Self>) {
        let (account_id, handle, mode) =
            match std::mem::replace(&mut self.pending_login, PendingLogin::None) {
                PendingLogin::None => return,
                PendingLogin::Preparing { account_id, mode } => (account_id, None, mode),
                PendingLogin::InProgress {
                    account_id,
                    handle,
                    mode,
                } => (account_id, Some(handle), mode),
            };
        if let Some(handle) = handle {
            handle.cancel();
        }
        if cleanup_dir_on_cancel(mode) {
            cleanup_account_dir(&account_config_dir(&self.data_dir, account_id));
        }
        cx.notify();
    }

    /// Action handler for [`ReauthenticateAccount`]. Thin shim, mirroring
    /// [`Self::on_add_managed_account`].
    pub(in crate::workspace) fn on_reauthenticate_account(
        &mut self,
        action: &ReauthenticateAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reauthenticate_account(action.0, window, cx);
    }

    /// Re-run a headless login for an **existing** managed account (Plan
    /// B, Task 6) — the reauthenticate counterpart to
    /// [`Self::add_managed_account`]. Reuses the account's current
    /// `config_dir` and `AccountId` rather than minting a fresh pair, so
    /// a failed attempt leaves that account and its on-disk credentials
    /// exactly as they were: [`Self::finish_reauth_failed`] deliberately
    /// does **not** call [`cleanup_account_dir`] (the add flow's
    /// throwaway-dir cleanup) — doing so here would delete a *good*
    /// account's directory just because a reauth attempt failed. This is
    /// the key behavioral difference from `add_managed_account`; the rest
    /// of the flow (guard, spawn, background `wait()`, re-entry) is
    /// otherwise identical.
    ///
    /// Guards up front the same way `add_managed_account` does: rejects
    /// (toast, no process spawned) when a login is already pending
    /// ([`can_start_login`]), when the active agent's login command is
    /// unavailable (remote agent), or when `account_id` no longer exists
    /// (e.g. a concurrent Settings-window delete raced this dispatch).
    /// `_window` is unused today (mirrors `add_managed_account`'s own
    /// unused `_window` — the flow is headless) but kept so a future
    /// UI-visible confirmation can use it without changing the signature.
    pub(in crate::workspace) fn reauthenticate_account(
        &mut self,
        account_id: AccountId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_start_login(&self.pending_login) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_busy())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.login_busy")
                    .build(),
                cx,
            );
            return;
        }

        let Some(account) = self.accounts.find(account_id) else {
            self.report_error(
                ErrorReport::new(s::settings_accounts_reauth_failed())
                    .message("The account being reauthenticated no longer exists.".to_string())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.account_missing")
                    .build(),
                cx,
            );
            return;
        };
        let config_dir = account.config_dir.clone();

        let Some(command) = self.active_agent_login_command() else {
            self.report_error(
                ErrorReport::new(s::settings_accounts_login_remote_unsupported())
                    .severity(ErrorSeverity::Warning)
                    .dedup("account.reauth.remote_unsupported")
                    .build(),
                cx,
            );
            return;
        };

        // Defensive only: `config_dir` is the account's existing
        // directory from its original login, so this is a no-op unless
        // something removed it out from under the account row.
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            self.report_error(
                ErrorReport::new(s::settings_accounts_reauth_failed())
                    .message(format!(
                        "could not access the account config directory: {e}"
                    ))
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            return;
        }

        let env = daruda_config::account_env(&config_dir);

        // See `add_managed_account`'s matching comment / `PendingLogin::Preparing`'s
        // doc: the node resolve below is blocking and may download Node.js, so
        // it runs on the background executor, and this state covers the gap
        // before a real login process (and cancel handle) exists.
        self.pending_login = PendingLogin::Preparing {
            account_id,
            mode: LoginMode::Reauth,
        };
        cx.notify();

        let resolve_command = command.clone();
        cx.spawn(async move |this, cx| {
            let path_pair = cx
                .background_executor()
                .spawn(async move { resolve_node_path_env(&resolve_command) })
                .await;

            // Unlike `add_managed_account`'s closed-window fallback, there is
            // nothing to clean up on a lost `Workspace` here: `config_dir` is
            // the account's permanent directory, not a throwaway one an
            // orphaned attempt would leak.
            // SILENT-OK: workspace window closed mid-reauth (during the node resolve, or after) — nothing to react to.
            let _ = this.update(cx, move |ws, cx| {
                // The user may have cancelled during the resolve — bail
                // without touching anything if this `Preparing` isn't the
                // one staged above.
                let is_current = matches!(
                    &ws.pending_login,
                    PendingLogin::Preparing { account_id: current, .. }
                        if *current == account_id
                );
                if !is_current {
                    return;
                }

                let mut inject = env.inject.clone();
                if let Some(pair) = path_pair {
                    inject.push(pair);
                }

                match spawn_login(&command, &config_dir, &inject, &env.strip, LOGIN_TIMEOUT) {
                    Ok(login_process) => {
                        let handle = login_process.handle();
                        ws.pending_login = PendingLogin::InProgress {
                            account_id,
                            handle,
                            mode: LoginMode::Reauth,
                        };
                        cx.notify();

                        cx.spawn(async move |this, cx| {
                            let outcome = cx
                                .background_executor()
                                .spawn(async move { login_process.wait() })
                                .await;
                            // SILENT-OK: workspace window closed mid-reauth — nothing to react to.
                            let _ = this.update(cx, |ws, cx| {
                                ws.finish_reauth(account_id, config_dir, outcome, cx)
                            });
                        })
                        .detach();
                    }
                    Err(e) => {
                        // No `cleanup_account_dir` here — see this method's
                        // doc: `config_dir` is the account's real,
                        // already-good directory, not a throwaway one for
                        // this attempt alone.
                        ws.pending_login = PendingLogin::None;
                        ws.report_error(
                            ErrorReport::new(s::settings_accounts_reauth_failed())
                                .message(e.to_string())
                                .severity(ErrorSeverity::Error)
                                .at(file!(), line!())
                                .build(),
                            cx,
                        );
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Picks up a reauthenticate-account login's result on this
    /// `Workspace`, once [`Self::reauthenticate_account`]'s background
    /// `wait()` resolves. Mirrors [`Self::finish_login`]'s staleness
    /// guard and dispatch shape.
    fn finish_reauth(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        outcome: LoginOutcome,
        cx: &mut Context<Self>,
    ) {
        let is_current = matches!(
            &self.pending_login,
            PendingLogin::InProgress { account_id: current, .. } if *current == account_id
        );
        if !is_current {
            return;
        }
        self.pending_login = PendingLogin::None;

        match outcome {
            LoginOutcome::Success => self.finish_reauth_success(account_id, config_dir, cx),
            LoginOutcome::Denied => {
                self.finish_reauth_failed(s::settings_accounts_login_denied_detail(), cx)
            }
            LoginOutcome::TimedOut => {
                self.finish_reauth_failed(s::settings_accounts_login_timed_out_detail(), cx)
            }
            LoginOutcome::Failed(detail) => self.finish_reauth_failed(detail, cx),
        }
    }

    /// `LoginOutcome::Success` half of [`Self::finish_reauth`]: confirms
    /// credentials actually landed, re-parses identity, and updates the
    /// row matching the *known* `account_id` directly — bumping
    /// `last_authenticated_at` and refreshing email/organization from
    /// whatever the fresh login reports.
    ///
    /// This deliberately updates by `account_id` rather than going
    /// through [`find_duplicate`] (the add flow's email+org dedup match):
    /// a reauth already knows exactly which account it's reauthenticating,
    /// so keying the update on that id is unambiguous even if the freshly
    /// reparsed identity no longer matches what was on file (e.g. the
    /// organization changed) — `find_duplicate` could otherwise update
    /// the *wrong* row, or file a spurious new account, if the reparsed
    /// identity happens to collide with (or fail to match) a different
    /// existing row.
    fn finish_reauth_success(
        &mut self,
        account_id: AccountId,
        config_dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = read_scoped_credentials(&config_dir) {
            self.finish_reauth_failed(
                format!("Reauthentication succeeded but no credentials were found afterward: {e}"),
                cx,
            );
            return;
        }

        let identity = read_account_identity(&config_dir);
        let now = now_unix();

        // Apply the reauth to freshly-loaded disk state (see
        // finish_login_success) so a stale in-memory snapshot can't clobber
        // accounts another window added while this reauth was in flight.
        let mut state =
            daruda_store::accounts::load_accounts_in(&self.data_dir).unwrap_or_default();

        let Some(existing) = state.accounts.iter_mut().find(|a| a.id == account_id) else {
            // The account was removed (Settings-window delete) while
            // this reauth was in flight — nothing left to update; the
            // delete flow already removed `config_dir` itself.
            self.accounts = state;
            cx.notify();
            return;
        };
        existing.last_authenticated_at = now;
        // Only overwrite identity when the reparse actually yielded a value —
        // a flaky post-success read must not blank a previously-good row.
        if identity.email.is_some() {
            existing.email = identity.email;
        }
        if identity.organization.is_some() {
            existing.organization = identity.organization;
        }

        if let Err(e) = daruda_store::accounts::save_accounts_in(&self.data_dir, &state) {
            log_io_error(
                "Failed to save accounts.json after reauthenticate-account login",
                "account.reauth.save_failed",
                &e,
            );
        }
        self.accounts = state;

        self.report_error(
            ErrorReport::new(s::settings_accounts_reauth_added())
                .severity(ErrorSeverity::Info)
                .build(),
            cx,
        );
        cx.notify();
    }

    /// Shared failure tail for [`Self::finish_reauth`]'s
    /// `Denied`/`TimedOut`/`Failed` outcomes and
    /// [`Self::finish_reauth_success`]'s own credentials-check failure.
    /// Deliberately does **not** call [`cleanup_account_dir`] — see
    /// [`Self::reauthenticate_account`]'s doc for why: `config_dir` here
    /// is the account's real, permanent directory (reused, not freshly
    /// minted for this attempt), so a failed reauth must leave the
    /// existing account and its on-disk credentials exactly as they were
    /// before the attempt.
    fn finish_reauth_failed(&mut self, detail: String, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::settings_accounts_reauth_failed())
                .message(detail)
                .severity(ErrorSeverity::Warning)
                .build(),
            cx,
        );
        cx.notify();
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

/// Best-effort resolve a managed-node runtime for `command`, for the
/// headless login spawn (mirrors the ACP session path's
/// `daruda_acp::node::ensure_node` + `NodeRuntime::wrap_command`, but
/// `spawn_login` can't consume `wrap_command`'s JSON-argv rewrite — it
/// tokenizes `command` itself — so this resolves only the PATH-injection
/// half: on a [`daruda_acp::NodeRuntime::Managed`] runtime, a `("PATH",
/// "<node_dir>/bin:$PATH")` pair for `spawn_login`'s `inject_env`, so the
/// child's `Command::new` lookup of `npx`/`node` finds the managed
/// install even when it isn't on the system PATH).
///
/// Returns `None` when `command` doesn't need node at all, the system
/// already has a usable node ([`daruda_acp::NodeRuntime::System`] — the
/// inherited PATH already covers it), or the resolve itself fails (a
/// download/network error) — in every `None` case `spawn_login` still
/// runs with the inherited PATH, and a genuine "no node" failure surfaces
/// as an ordinary [`daruda_claude::accounts::LoginError`] through the
/// normal failure toast rather than being special-cased here.
///
/// **Blocking** — [`daruda_acp::ensure_node`] may download and extract a
/// node runtime on a first-run machine. Callers must run this on
/// [`gpui::BackgroundExecutor`], never the UI update thread.
pub(in crate::workspace) fn resolve_node_path_env(command: &str) -> Option<(String, String)> {
    if !daruda_acp::node::command_needs_node(command) {
        return None;
    }
    let node_install_dir = daruda_store::persistence::node_install_dir();
    match daruda_acp::ensure_node(&node_install_dir, &mut |_| {}) {
        Ok(daruda_acp::NodeRuntime::Managed { node_dir }) => {
            let existing_path = std::env::var("PATH").unwrap_or_default();
            Some((
                "PATH".to_string(),
                format!("{}:{existing_path}", node_dir.join("bin").display()),
            ))
        }
        Ok(daruda_acp::NodeRuntime::System) | Err(_) => None,
    }
}

/// Matches an existing managed account with the *same* email *and* the
/// same organization — a duplicate login for an account already tracked.
/// Same email under a *different* organization (e.g. the same person's
/// Claude account under two orgs) is treated as a distinct account, since
/// Claude scopes usage/plan per organization. Two accounts that both have
/// no captured email/org (`None`/`None`) are also considered duplicates of
/// each other under this literal-equality rule — an edge case that
/// shouldn't arise once Plan B logins always capture `oauthAccount`.
pub(in crate::workspace) fn find_duplicate(
    state: &AccountsState,
    email: Option<&str>,
    org: Option<&str>,
) -> Option<AccountId> {
    state
        .accounts
        .iter()
        .find(|a| a.email.as_deref() == email && a.organization.as_deref() == org)
        .map(|a| a.id)
}

/// Best-effort remove `config_dir` — and the scoped macOS Keychain item a
/// login into it may have written — after a login attempt didn't produce
/// a (kept) managed account — spawn failed, a `Denied`/`TimedOut`/`Failed`
/// outcome, a duplicate dedup, a cancel, or a closed-window race. Missing
/// (already removed by an earlier cleanup on the same login, or never
/// written because the login didn't get that far) is not an error; any
/// other failure is logged, not surfaced as a toast — losing an orphaned
/// config dir or Keychain item has no functional impact on the account
/// catalog.
fn cleanup_account_dir(config_dir: &Path) {
    delete_scoped_credentials(config_dir);
    if let Err(e) = std::fs::remove_dir_all(config_dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log_io_error(
            "Failed to remove account config dir after a failed add-account login",
            "account.add.cleanup_dir_failed",
            &e,
        );
    }
}

/// Current Unix time in seconds, saturating to `0` on a clock error.
/// Mirrors `settings_window::sections::accounts`'s private helper of the
/// same shape — not shared across that module boundary for a 5-line
/// helper.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Log an I/O failure without surfacing a toast — mirrors
/// `settings_window::sections::accounts`'s private helper of the same
/// shape (see its doc for why: this module has `Workspace::report_error`
/// available and still chooses to log rather than toast, since a
/// best-effort cleanup failure has no functional impact worth interrupting
/// the user for).
fn log_io_error(title: &str, dedup: &str, err: &std::io::Error) {
    LogWriter::log(
        ErrorReport::new(title)
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .with_context("error", format!("{err}"))
            .dedup(dedup)
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_kind_busy_new_pane_idle_in_place() {
        assert_eq!(switch_kind(false), SwitchKind::InPlace);
        assert_eq!(switch_kind(true), SwitchKind::NewPane);
    }

    #[test]
    fn can_start_login_none_is_startable() {
        assert!(can_start_login(&PendingLogin::None));
    }

    #[test]
    fn can_start_login_preparing_is_not_startable() {
        let pending = PendingLogin::Preparing {
            account_id: AccountId::new(),
            mode: LoginMode::Add,
        };
        assert!(!can_start_login(&pending));
    }

    #[test]
    fn resolve_node_path_env_none_when_command_does_not_need_node() {
        assert_eq!(resolve_node_path_env("/usr/local/bin/codex-acp"), None);
    }

    #[test]
    fn cleanup_dir_on_cancel_add_yes_reauth_no() {
        assert!(cleanup_dir_on_cancel(LoginMode::Add));
        assert!(!cleanup_dir_on_cancel(LoginMode::Reauth));
    }

    #[test]
    fn can_start_login_in_progress_is_not_startable() {
        // A real (near-instant, no-op) child process is the only way to
        // build a `LoginProcessHandle` — it has no public constructor
        // other than `LoginProcess::handle()`.
        let login = spawn_login(
            "/usr/bin/true",
            Path::new("/tmp/daruda-test-can-start-login"),
            &[],
            &[],
            Duration::from_secs(5),
        )
        .expect("spawn a trivial process for the test handle");
        let pending = PendingLogin::InProgress {
            account_id: AccountId::new(),
            handle: login.handle(),
            mode: LoginMode::Add,
        };
        assert!(!can_start_login(&pending));
        // Best-effort: the process has almost certainly already exited on
        // its own; this just avoids relying on that timing.
        if let PendingLogin::InProgress { handle, .. } = &pending {
            handle.cancel();
        }
    }

    #[test]
    fn find_duplicate_matches_email_and_org() {
        let mut st = AccountsState::default();
        let id = AccountId::new();
        st.accounts.push(ManagedAccount {
            id,
            provider: AgentProvider::Claude,
            email: Some("a@x.com".into()),
            organization: Some("Org".into()),
            config_dir: "/x".into(),
            created_at: 0,
            last_authenticated_at: 0,
        });
        assert_eq!(find_duplicate(&st, Some("a@x.com"), Some("Org")), Some(id));
        assert_eq!(find_duplicate(&st, Some("a@x.com"), Some("Other")), None); // same email, diff org = distinct
        assert_eq!(find_duplicate(&st, Some("b@x.com"), Some("Org")), None);
    }
}
