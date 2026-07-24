//! Accounts section of the Settings window — list managed accounts per
//! provider, set the per-provider default, delete an account. Visual
//! language mirrors the Agent-catalog section's bordered row (see
//! `sections::mod::render_agent_catalog_row`).
//!
//! Cross-window state: `daruda_store::accounts::AccountsState` is
//! persisted once as `accounts.json`, but each open `Workspace` window
//! also caches a copy (`Workspace.accounts`, Task 6/7/8) for its own
//! pane-spawn/status-bar reads. This section is the sole place that
//! writes `accounts.json` (`SetDefaultAccount`/`RemoveAccount`); after
//! every write it re-broadcasts the new state to every open `Workspace`
//! window via `WindowRegistry::for_each_workspace` +
//! `Workspace::sync_accounts_state`, so their caches never go stale
//! waiting for a restart. This mirrors the pattern the Plugin section
//! uses for `SkillsState`, adapted for a per-window field instead of a
//! `cx` Global — see `sections::plugin::run_plugin_op`'s doc comment for
//! why a direct broadcast (not a Global) is the right call here too:
//! deleting/defaulting an account must work even when no Workspace
//! window happens to be open.
//!
//! The "+ Add account" row is a narrower case: unlike delete/default (pure
//! `accounts.json` edits), a headless login (`Workspace::add_managed_account`)
//! needs a `Workspace` to run against — it resolves the login command from
//! that window's own agent catalog / `last_agent_id`. `start_add_account`
//! picks `WindowRegistry::first_workspace` (a no-op toast-free log when none
//! is open) and runs the login there; it deliberately does **not** mirror
//! `Workspace.pending_login` into this window (see `start_add_account`'s doc)
//! — the spinner + Cancel affordance lives in the status-bar dropdown, which
//! reads that state natively in the window the login is actually running in.

use std::time::{SystemTime, UNIX_EPOCH};

use daruda_store::accounts::{AccountId, AgentProvider, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::SettingsWindow;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariant, Disableable as _, button, button_danger};
use crate::window_registry::WindowRegistry;
use crate::workspace::dialog_helpers::open_confirm_dialog;

const PROVIDERS: [AgentProvider; 2] = [AgentProvider::Claude, AgentProvider::Codex];

fn provider_label(provider: AgentProvider) -> String {
    match provider {
        AgentProvider::Claude => s::status_bar_account_provider_claude(),
        AgentProvider::Codex => s::status_bar_account_provider_codex(),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Bucket "time since last authenticated" into a display label. `0` is
/// the never-authenticated sentinel (`ManagedAccount` has no captured
/// login yet — unreachable once Plan B always stamps this on login, but
/// the field predates that work). Mirrors `right_dock::usage`'s
/// `cache_age_bucket` shape; kept as its own function since account
/// freshness and fetch-cache age are different concepts that could
/// diverge in wording later.
fn last_authenticated_label(now: u64, last_authenticated_at: u64) -> String {
    if last_authenticated_at == 0 {
        return s::settings_accounts_last_auth_never();
    }
    let age = now.saturating_sub(last_authenticated_at);
    if age < 60 {
        s::settings_accounts_last_auth_just_now()
    } else if age < 3_600 {
        s::settings_accounts_last_auth_minutes(age / 60)
    } else if age < 86_400 {
        s::settings_accounts_last_auth_hours(age / 3_600)
    } else {
        s::settings_accounts_last_auth_days(age / 86_400)
    }
}

/// Body text for the delete-account confirm dialog. `count` is how many
/// panes (across every open Workspace window) currently override to
/// this account — see `Workspace::panes_referencing_account`.
fn remove_confirm_body(count: usize) -> String {
    s::settings_accounts_remove_confirm_body(count)
}

/// Log an I/O failure without surfacing a toast — the Settings window
/// has no `Workspace::report_error`; `LogWriter::log` is the documented
/// fallback for GPUI-free / pre-Workspace error sites (see project
/// CLAUDE.md §Error reporting).
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

impl SettingsWindow {
    pub(in crate::settings_window) fn render_accounts(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_accounts(), cx));

        if self.accounts.accounts.is_empty() {
            body = body.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(theme::current(cx).text_muted)
                    .child(s::settings_accounts_empty()),
            );
        } else {
            for provider in PROVIDERS {
                let rows: Vec<&ManagedAccount> = self
                    .accounts
                    .accounts
                    .iter()
                    .filter(|a| a.provider == provider)
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                body = body.child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(theme::current(cx).text_primary)
                        .child(provider_label(provider)),
                );
                let default_id = self.accounts.default_by_provider.get(&provider).copied();
                for account in rows {
                    body = body.child(self.render_account_row(account, default_id, cx));
                }
            }
        }

        body.child(self.render_add_account_row(cx))
            .into_any_element()
    }

    fn render_account_row(
        &self,
        account: &ManagedAccount,
        default_id: Option<AccountId>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let account_id = account.id;
        let provider = account.provider;
        let is_default = default_id == Some(account_id);
        let row_key = account.id.0.to_string();

        let email = account
            .email
            .clone()
            .unwrap_or_else(s::settings_accounts_unknown_email);
        let last_auth = last_authenticated_label(now_unix(), account.last_authenticated_at);
        let subtitle = match account.organization.as_deref() {
            Some(org) => format!("{org} · {last_auth}"),
            None => last_auth,
        };

        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SKILL_ROW_GAP))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(t.text_primary)
                            .child(email),
                    )
                    .child(
                        div()
                            .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                            .text_color(t.text_muted)
                            .child(subtitle),
                    ),
            );
        if is_default {
            header = header.child(
                div()
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(s::settings_accounts_default_badge()),
            );
        }

        let actions = div()
            .flex()
            .flex_row()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .child(
                button(
                    SharedString::from(format!("settings-accounts-default-{row_key}")),
                    s::settings_accounts_set_default(),
                )
                .disabled(is_default)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.set_default_account(account_id, provider, cx);
                })),
            )
            .child(
                button(
                    SharedString::from(format!("settings-accounts-reauth-{row_key}")),
                    s::settings_accounts_reauthenticate(),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.start_reauthenticate_account(account_id, cx);
                })),
            )
            .child(
                button_danger(
                    SharedString::from(format!("settings-accounts-delete-{row_key}")),
                    s::settings_accounts_delete(),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_remove_confirm(account_id, window, cx);
                })),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .p(px(theme::MODAL_PANEL_GAP))
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_MD))
            .child(header)
            .child(actions)
            .into_any_element()
    }

    /// Starts a headless add-account login on click — see
    /// [`Self::start_add_account`] for why there's no live spinner here
    /// (that lives in the status-bar dropdown, which reads
    /// `Workspace.pending_login` natively; this window has no `Workspace`
    /// of its own to observe it on).
    fn render_add_account_row(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement + use<> {
        div().flex().flex_row().child(
            button("settings-accounts-add", s::settings_accounts_add()).on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.start_add_account(cx);
                },
            )),
        )
    }

    /// Starts a headless add-account login (Plan B) against the first
    /// live `Workspace` window (`WindowRegistry::first_workspace` — see
    /// its doc for why "first" rather than the OS-active window: this is
    /// a Settings-window click handler, so `cx.active_window()` would
    /// resolve to the Settings window itself, not a workspace).
    ///
    /// This window has no progress/spinner affordance of its own:
    /// `Workspace.pending_login` is `pub(in crate::workspace)` and
    /// observing it live from here would mean either polling every open
    /// Workspace window on every Settings repaint (no precedent for
    /// cross-window reads during `render()` in this codebase — see the
    /// project CLAUDE.md's "No GPUI re-entry in render()" convention) or
    /// a dedicated broadcast mirror, neither of which this task's scope
    /// covers. The login itself still runs for real; its spinner + Cancel
    /// affordance is the status-bar dropdown (`status_bar::build_account_menu`),
    /// which reads the state natively in the Workspace window it belongs
    /// to. A toast in that target window reports success/failure.
    fn start_add_account(&self, cx: &mut gpui::Context<Self>) {
        let Some((handle, weak)) = WindowRegistry::first_workspace(cx) else {
            LogWriter::log(
                ErrorReport::new("Add-account login has no open Workspace window to run against")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("settings.accounts.add_no_workspace")
                    .build(),
            );
            return;
        };
        let result = cx.update_window(handle, |_root, window, cx_w| {
            if let Some(ws) = weak.upgrade() {
                ws.update(cx_w, |ws, cx| {
                    ws.add_managed_account(AgentProvider::Claude, window, cx);
                });
            }
        });
        if let Err(e) = result {
            LogWriter::log(
                ErrorReport::new(
                    "Failed to start add-account login: target Workspace window is gone",
                )
                .message(e.to_string())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("settings.accounts.add_target_window_gone")
                .build(),
            );
        }
    }

    /// Starts a headless reauthenticate-account login (Plan B, Task 6)
    /// against the first live `Workspace` window — same
    /// `WindowRegistry::first_workspace` target-resolution rationale as
    /// [`Self::start_add_account`]. Unlike that method, this dispatches
    /// the [`crate::workspace::ReauthenticateAccount`] action into the
    /// resolved window (rather than calling a `Workspace` method
    /// directly): the action already carries the target `AccountId` and
    /// is registered on `Workspace`'s root render tree
    /// (`Workspace::on_reauthenticate_account`), so dispatching it here
    /// reaches the same handler without this Settings-window module
    /// needing `pub(crate)` access into `crate::workspace`'s internals.
    ///
    /// Same "no spinner here" rationale as `start_add_account`: the
    /// in-flight indicator lives in the status-bar dropdown of the
    /// window the login actually runs in.
    fn start_reauthenticate_account(&self, account_id: AccountId, cx: &mut gpui::Context<Self>) {
        let Some((handle, _weak)) = WindowRegistry::first_workspace(cx) else {
            LogWriter::log(
                ErrorReport::new(
                    "Reauthenticate-account login has no open Workspace window to run against",
                )
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("settings.accounts.reauth_no_workspace")
                .build(),
            );
            return;
        };
        let result = cx.update_window(handle, |_root, window, cx_w| {
            window.dispatch_action(
                Box::new(crate::workspace::ReauthenticateAccount(account_id)),
                cx_w,
            );
        });
        if let Err(e) = result {
            LogWriter::log(
                ErrorReport::new(
                    "Failed to start reauthenticate-account login: target Workspace window is gone",
                )
                .message(e.to_string())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("settings.accounts.reauth_target_window_gone")
                .build(),
            );
        }
    }

    /// Immediate (no confirm) — makes `account_id` the provider default
    /// for new panes, persists, and syncs every open Workspace window.
    fn set_default_account(
        &mut self,
        account_id: AccountId,
        provider: AgentProvider,
        cx: &mut gpui::Context<Self>,
    ) {
        // Apply the change to freshly-loaded disk state, not to this
        // window's possibly-stale snapshot: a Workspace may have added an
        // account since this Settings window was built, and a full
        // overwrite from `self.accounts` would silently destroy it.
        let mut state = daruda_store::accounts::load_accounts().unwrap_or_default();
        state.default_by_provider.insert(provider, account_id);
        if let Err(e) = daruda_store::accounts::save_accounts(&state) {
            log_io_error(
                "Failed to save accounts.json after set-default",
                "settings.accounts.save_default_failed",
                &e,
            );
        }
        self.accounts = state;
        self.broadcast_accounts_state(cx);
        cx.notify();
    }

    /// G9 confirm dialog before delete — destructive, and (per the
    /// brief) the body must show how many live panes fall back to the
    /// provider default. `referencing` sums
    /// `Workspace::panes_referencing_account` across every open
    /// Workspace window; a lane not visited this session in a closed or
    /// not-yet-opened window isn't counted (see that method's doc for
    /// why this is an accepted simplification rather than a fake count).
    fn open_remove_confirm(
        &mut self,
        account_id: AccountId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut referencing = 0usize;
        WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
            referencing += ws.panes_referencing_account(account_id);
        });
        let weak = cx.weak_entity();
        open_confirm_dialog(
            s::settings_accounts_remove_confirm_title(),
            remove_confirm_body(referencing),
            s::settings_accounts_remove_confirm_ok(),
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(this) = weak.upgrade() {
                    this.update(app_cx, |this, cx| this.remove_account(account_id, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Runs after the delete confirm: best-effort removes the account's
    /// isolated `CLAUDE_CONFIG_DIR`, drops it from `AccountsState`,
    /// persists, then clears the override on every pane that referenced
    /// it (across every open Workspace window) and syncs their caches.
    fn remove_account(&mut self, account_id: AccountId, cx: &mut gpui::Context<Self>) {
        // Operate on freshly-loaded disk state (see set_default_account) so a
        // stale snapshot can't resurrect or clobber accounts another window
        // changed while this Settings window was open.
        let mut state = daruda_store::accounts::load_accounts().unwrap_or_default();
        let Some(account) = state.find(account_id).cloned() else {
            self.accounts = state;
            cx.notify();
            return;
        };
        // Remove the isolated config dir AND its scoped Keychain item —
        // otherwise the OS credential entry leaks on every deletion.
        daruda_claude::accounts::delete_scoped_credentials(&account.config_dir);
        if let Err(e) = std::fs::remove_dir_all(&account.config_dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log_io_error(
                "Failed to remove account config dir",
                "settings.accounts.remove_config_dir_failed",
                &e,
            );
        }
        state.accounts.retain(|a| a.id != account_id);
        state.default_by_provider.retain(|_, id| *id != account_id);
        if let Err(e) = daruda_store::accounts::save_accounts(&state) {
            log_io_error(
                "Failed to save accounts.json after delete",
                "settings.accounts.save_delete_failed",
                &e,
            );
        }
        self.accounts = state.clone();
        WindowRegistry::for_each_workspace(cx, move |ws, _window, cx| {
            ws.clear_account_override(account_id, cx);
            ws.sync_accounts_state(state.clone(), cx);
        });
        cx.notify();
    }

    fn broadcast_accounts_state(&self, cx: &mut gpui::Context<Self>) {
        let state = self.accounts.clone();
        WindowRegistry::for_each_workspace(cx, move |ws, _window, cx| {
            ws.sync_accounts_state(state.clone(), cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_confirm_counts_referencing_panes() {
        assert_eq!(
            remove_confirm_body(3),
            "3 pane(s) using this account will fall back to the provider default."
        );
    }

    #[test]
    fn remove_confirm_body_zero_panes() {
        assert_eq!(
            remove_confirm_body(0),
            "0 pane(s) using this account will fall back to the provider default."
        );
    }

    #[test]
    fn last_authenticated_label_never_when_zero() {
        assert_eq!(last_authenticated_label(1_000, 0), "Never authenticated");
    }

    #[test]
    fn last_authenticated_label_buckets() {
        assert_eq!(
            last_authenticated_label(1_000, 1_000),
            "Authenticated just now"
        );
        assert_eq!(
            last_authenticated_label(1_000 + 120, 1_000),
            "Authenticated 2m ago"
        );
        assert_eq!(
            last_authenticated_label(1_000 + 7_200, 1_000),
            "Authenticated 2h ago"
        );
        assert_eq!(
            last_authenticated_label(1_000 + 2 * 86_400, 1_000),
            "Authenticated 2d ago"
        );
    }
}
