//! Bottom status bar — displays project/branch context and focused
//! pane title. Always visible at the bottom of the workspace.

use crate::ui::theme;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenu, PopupMenuItem, Sizable as _, button,
    menu_builder, spinner,
};
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::{AddManagedAccount, OpenSettings, Workspace};
use daruda_store::accounts::{AccountSelection, AgentProvider, ManagedAccount};
use gpui::{App, IntoElement, RenderOnce, SharedString, WeakEntity, Window, div, prelude::*, px};

/// Fixed height of the status bar in pixels.
pub(super) const STATUS_BAR_HEIGHT: f32 = theme::STATUS_BAR_HEIGHT;

/// Display + dropdown data for the focused pane's account, shown as a
/// clickable slot in the status bar's right section. Resolved by the
/// snapshot builder in `render/mod.rs` from the focused pane's
/// [`AccountSelection`] + `Workspace.accounts`.
#[derive(Clone)]
pub(super) struct AccountSlot {
    /// `"email (plan)"`, `"email"`, or the "System" fallback — see
    /// [`account_label`].
    pub label: SharedString,
    /// The account's provider (Claude / Codex) — filters which managed
    /// accounts the dropdown lists. Resolved from the current account
    /// when set, else `AgentProvider::default()` (Claude) — every
    /// switchable pane kind resolves against Claude only until Codex
    /// account management ships (see `pane::resolve_account_config_dir`).
    pub provider: AgentProvider,
    /// The focused pane the dropdown's menu items dispatch
    /// `Workspace::switch_pane_account` against.
    pub pane_id: PaneId,
    /// The pane's own selection, so the dropdown can mark the active entry.
    pub current: AccountSelection,
    /// Managed accounts matching `provider`, for the dropdown's entries.
    pub accounts: Vec<ManagedAccount>,
    /// Dispatch target for the dropdown's menu-item clicks.
    pub workspace: WeakEntity<Workspace>,
    /// Mirrors `Workspace::active_agent_login_unavailable` — disables the
    /// dropdown's "+ Add account" entry when the session's active agent
    /// launch is remote (SSH/Docker) or not in the catalog, so the
    /// affordance doesn't invite a click `add_managed_account` would
    /// immediately reject.
    pub login_unavailable: bool,
    /// Mirrors `Workspace::is_login_pending` — swaps "+ Add account" for
    /// an in-progress row + Cancel while a headless login is running.
    pub login_pending: bool,
}

impl AccountSlot {
    /// Resolve the status-bar account slot for a Terminal/AgentChat pane's
    /// [`AccountSelection`] against the workspace's `AccountsState`. Always
    /// produces a slot: a [`AccountSelection::Managed`] id that no longer
    /// resolves (deleted account) falls back to the same "System" label as
    /// [`AccountSelection::SystemDefault`], rather than surfacing a dangling
    /// reference to the user.
    pub(super) fn resolve(
        pane_id: PaneId,
        selection: AccountSelection,
        accounts: &daruda_store::accounts::AccountsState,
        workspace: WeakEntity<Workspace>,
        login_unavailable: bool,
        login_pending: bool,
    ) -> Self {
        let (label, provider) = match selection.account_id().and_then(|id| accounts.find(id)) {
            Some(account) => (
                // Status-bar slot shows the email only — the organization is
                // omitted here (it's still shown in the account dropdown to
                // disambiguate same-email accounts).
                account_label(account.email.as_deref(), None),
                account.provider,
            ),
            None => (account_label(None, None), AgentProvider::default()),
        };
        let matching = accounts
            .accounts
            .iter()
            .filter(|a| a.provider == provider)
            .cloned()
            .collect();
        Self {
            label: label.into(),
            provider,
            pane_id,
            current: selection,
            accounts: matching,
            workspace,
            login_unavailable,
            login_pending,
        }
    }
}

/// Pure label formatter for [`AccountSlot::label`]: prefers `email (plan)`,
/// falls back to just `email`, and finally to the "system account" label
/// when neither is available (no account resolved for the pane).
pub(in crate::workspace) fn account_label(email: Option<&str>, plan: Option<&str>) -> String {
    match (email, plan) {
        (Some(email), Some(plan)) => format!("{email} ({plan})"),
        (Some(email), None) => email.to_string(),
        (None, _) => crate::surface::strings::status_bar_account_system(),
    }
}

/// Display name for the dropdown's provider section header.
fn provider_label(provider: AgentProvider) -> String {
    match provider {
        AgentProvider::Claude => crate::surface::strings::status_bar_account_provider_claude(),
        AgentProvider::Codex => crate::surface::strings::status_bar_account_provider_codex(),
    }
}

/// Collected status bar data — snapshot taken before rendering to
/// avoid entity reads during element construction (GPUI re-entrant
/// panic prevention).
pub(super) struct StatusBarData {
    /// `<project>/<branch>` for git-backed active lanes, just
    /// `<project>` for non-git or detached HEAD, `None` in Welcome
    /// state (no project loaded). The detached marker is rendered
    /// separately via [`Self::is_detached`].
    pub project_branch: Option<SharedString>,
    /// True when the active lane is git-backed but on a detached
    /// HEAD. Drives the inline "detached" chip rendered next to
    /// `project_branch`; harmless when `project_branch` is `None`
    /// (the chip suppresses itself).
    pub is_detached: bool,
    /// Focused pane title (process name / shell prompt).
    pub title: SharedString,
    /// Transient error string (pane spawn failures, etc.). When set,
    /// shows in the right section so the user actually notices the
    /// failure.
    pub error: Option<SharedString>,
    /// True when the workspace's project layer has a config.toml on
    /// disk. Drives a small dot in the right section so the user sees
    /// at a glance that some user-global keys are being shadowed.
    pub has_project_config: bool,
    /// The focused pane's resolved account slot. `None` when the focused
    /// pane doesn't track an account (File / TaskEdit panes) — hides the
    /// slot entirely. `Some` for Terminal / AgentChat panes, even when no
    /// account is configured (shows the "System" fallback label).
    pub account: Option<AccountSlot>,
}

/// GPUI render-once wrapper for the status bar element.
#[derive(IntoElement)]
pub(super) struct StatusBar(pub(super) StatusBarData);

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let data = self.0;
        // Snapshot every theme slot once so the chain below reads each
        // colour through the live `DarudaTheme` Global; the four mid-render
        // `.text_color(t.text_muted)` etc. lookups stay consistent even if
        // a theme swap fires between expressions.
        let t = theme::current(cx);
        let muted = t.text_muted;
        let faint = t.text_subtle;
        let project_dot = t.status_bar_project_dot;
        let error_color = theme::ERROR;
        let detached_bg = t.status_bar_detached_bg;
        let detached_text = t.status_bar_detached_text;
        let bg = t.status_bar_bg;
        let border = t.border;

        // Detached chip is meaningful only when there's a
        // project/branch slot to anchor next to; in Welcome state
        // (`project_branch` is `None`) suppress the chip too.
        let show_detached = data.is_detached && data.project_branch.is_some();

        let left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::STATUS_BAR_GAP))
            .when_some(data.project_branch.clone(), |el, pb| {
                el.child(
                    div()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(muted)
                        .child(pb),
                )
            })
            .when(show_detached, |el| {
                el.child(
                    div()
                        .px(px(theme::STATUS_BAR_DETACHED_PAD_X))
                        .py(px(theme::STATUS_BAR_DETACHED_PAD_Y))
                        .rounded(px(theme::STATUS_BAR_DETACHED_RADIUS))
                        .bg(detached_bg)
                        .text_size(px(theme::STATUS_BAR_DETACHED_FONT_SIZE))
                        .text_color(detached_text)
                        .child(SharedString::from(
                            crate::surface::strings::status_bar_detached_chip(),
                        )),
                )
            })
            .when(data.project_branch.is_some(), |el| {
                el.child(
                    div()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(faint)
                        .child(SharedString::from("—")),
                )
            })
            .child(
                div()
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .text_color(muted)
                    .child(data.title.clone()),
            );

        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::STATUS_BAR_GAP))
            .when(data.has_project_config, |el| {
                el.child(
                    div()
                        .id("status-project-config")
                        .w(px(theme::STATUS_BAR_PROJECT_DOT_SIZE))
                        .h(px(theme::STATUS_BAR_PROJECT_DOT_SIZE))
                        .rounded_full()
                        .bg(project_dot)
                        .tooltip(crate::ui::tooltip::text(
                            crate::surface::strings::status_bar_project_config_tooltip(),
                        )),
                )
            })
            .when_some(data.account.clone(), |el, slot| {
                el.child(
                    button("status-account", slot.label.clone())
                        .ghost()
                        .xsmall()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(muted)
                        .dropdown_menu(menu_builder(move |menu, _window, _cx| {
                            build_account_menu(&slot, menu)
                        })),
                )
            })
            .when_some(data.error.clone(), |el, err| {
                el.child(
                    div()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(error_color)
                        .child(err),
                )
            });

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .px(px(theme::STATUS_BAR_PAD_X))
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .child(left)
            .child(right)
    }
}

/// Build the account slot's dropdown menu: a "System default" entry
/// (`~/.claude`, always present so a managed-account pane can always be
/// reverted), then one item per managed account matching the slot's
/// provider (checkmark on the pane's current override), then either "+ Add
/// account" (disabled while the session's active agent launch is remote —
/// see `login_unavailable`) or, while a headless login is running
/// (`login_pending`), an in-progress row + Cancel in its place, and finally
/// a "manage accounts" entry that opens Settings. Each item is a one-line
/// dispatch into a `Workspace` method or action (render purity; no logic
/// here).
fn build_account_menu(slot: &AccountSlot, menu: PopupMenu) -> PopupMenu {
    let menu = {
        let workspace = slot.workspace.clone();
        let pane_id = slot.pane_id;
        let is_current = slot.current == AccountSelection::SystemDefault;
        menu.item(
            PopupMenuItem::new(SharedString::from(
                crate::surface::strings::status_bar_account_system(),
            ))
            .checked(is_current)
            .on_click(move |_, window, app| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(app, |ws, cx| {
                        ws.switch_pane_account(pane_id, AccountSelection::SystemDefault, window, cx)
                    });
                }
            }),
        )
    };
    // A header names the provider once its account list is non-empty, so a
    // future multi-provider catalog reads as grouped sections rather than
    // one flat list (today there's only ever one section — Claude is the
    // only provider Plan A manages).
    let menu = if slot.accounts.is_empty() {
        menu
    } else {
        menu.separator()
            .label(SharedString::from(provider_label(slot.provider)))
            .separator()
    };
    let menu = slot.accounts.iter().fold(menu, |m, account| {
        let workspace = slot.workspace.clone();
        let pane_id = slot.pane_id;
        let account_id = account.id;
        let is_current = slot.current == AccountSelection::Managed(account_id);
        let label = account_label(account.email.as_deref(), account.organization.as_deref());
        m.item(
            PopupMenuItem::new(SharedString::from(label))
                .checked(is_current)
                .on_click(move |_, window, app| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(app, |ws, cx| {
                            ws.switch_pane_account(
                                pane_id,
                                AccountSelection::Managed(account_id),
                                window,
                                cx,
                            )
                        });
                    }
                }),
        )
    });
    let menu = menu.separator();
    let menu = if slot.login_pending {
        let cancel_workspace = slot.workspace.clone();
        menu.item(
            PopupMenuItem::element(|_window, _cx| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::STATUS_BAR_GAP))
                    .child(spinner())
                    .child(SharedString::from(
                        crate::surface::strings::settings_accounts_login_in_progress(),
                    ))
            })
            .disabled(true),
        )
        .item(
            PopupMenuItem::new(SharedString::from(
                crate::surface::strings::settings_account_login_cancel(),
            ))
            .on_click(move |_, _window, app| {
                if let Some(ws) = cancel_workspace.upgrade() {
                    ws.update(app, |ws, cx| ws.cancel_pending_login(cx));
                }
            }),
        )
    } else {
        let provider = slot.provider;
        menu.item(
            PopupMenuItem::new(SharedString::from(
                crate::surface::strings::status_bar_add_account(),
            ))
            .disabled(slot.login_unavailable)
            .on_click(move |_, window, app| {
                window.dispatch_action(Box::new(AddManagedAccount(provider)), app);
            }),
        )
    };
    menu.item(
        PopupMenuItem::new(SharedString::from(
            crate::surface::strings::status_bar_manage_accounts(),
        ))
        .on_click(|_, window, app| {
            window.dispatch_action(
                Box::new(OpenSettings(daruda_config::BuiltinSection::Accounts)),
                app,
            );
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::account_label;

    #[test]
    fn account_slot_label_prefers_email_then_falls_back() {
        assert_eq!(
            account_label(Some("alice@x.com"), Some("Team")),
            "alice@x.com (Team)"
        );
        assert_eq!(account_label(Some("alice@x.com"), None), "alice@x.com");
        assert_eq!(account_label(None, None), "System (~/.claude)");
    }
}
