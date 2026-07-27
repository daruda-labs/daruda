//! Status bar's account slot — the focused pane's account (or the
//! "System" fallback), rendered as a dropdown trigger + menu for
//! switching between managed accounts.

use crate::ui::theme;
use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, button_status_pill_bare, spinner};
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::{AddManagedAccount, OpenSettings, Workspace};
use daruda_store::accounts::{AccountSelection, AgentProvider, ManagedAccount};
use gpui::{App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

const ACCOUNT_SLOT_MAX_WIDTH: f32 = 220.0;
const ACCOUNT_SLOT_COMPACT_MAX_WIDTH: f32 = 150.0;

/// Display + dropdown data for the focused pane's account, shown as a
/// clickable slot in the status bar's right section. Resolved by the
/// snapshot builder in `render/mod.rs` from the focused pane's
/// [`AccountSelection`] + `Workspace.accounts`.
#[derive(Clone)]
pub(in crate::workspace) struct AccountSlot {
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
    pub(in crate::workspace) fn resolve(
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

/// Render the account slot's dropdown trigger button.
pub(super) fn render(
    slot: &AccountSlot,
    density: super::StatusBarDensity,
    cx: &App,
) -> impl IntoElement {
    let label = (density != super::StatusBarDensity::IconOnly).then(|| slot.label.clone());
    let max_width = if density == super::StatusBarDensity::Full {
        ACCOUNT_SLOT_MAX_WIDTH
    } else {
        ACCOUNT_SLOT_COMPACT_MAX_WIDTH
    };
    let slot = slot.clone();
    button_status_pill_bare("status-account", cx)
        .max_w(px(max_width))
        .overflow_hidden()
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .min_w_0()
                .gap(px(theme::STATUS_BAR_USAGE_CHIP_GAP))
                .children(label.map(|label| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .truncate()
                        .child(label)
                }))
                .child(div().flex_none().child(SharedString::from(
                    crate::surface::strings::TASK_PILL_CHEVRON.trim_start(),
                ))),
        )
        .dropdown_menu(crate::ui::menu_builder(move |menu, _window, _cx| {
            build_account_menu(&slot, menu)
        }))
}

/// `"{label} ▾"` normally; at `IconOnly` the text label is dropped —
/// only the dropdown chevron remains, so the dropdown stays discoverable
/// without the row growing wide enough to force a wrap.
#[cfg(test)]
fn trigger_label(label: &str, density: super::StatusBarDensity) -> String {
    if density == super::StatusBarDensity::IconOnly {
        crate::surface::strings::TASK_PILL_CHEVRON
            .trim_start()
            .to_string()
    } else {
        format!("{label}{}", crate::surface::strings::TASK_PILL_CHEVRON)
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
    use super::{account_label, trigger_label};
    use crate::workspace::status_bar::StatusBarDensity;

    #[test]
    fn account_slot_label_prefers_email_then_falls_back() {
        assert_eq!(
            account_label(Some("alice@x.com"), Some("Team")),
            "alice@x.com (Team)"
        );
        assert_eq!(account_label(Some("alice@x.com"), None), "alice@x.com");
        assert_eq!(account_label(None, None), "System (~/.claude)");
    }

    #[test]
    fn trigger_label_keeps_text_outside_icon_only() {
        assert!(trigger_label("alice@x.com", StatusBarDensity::Full).starts_with("alice@x.com"));
        assert!(trigger_label("alice@x.com", StatusBarDensity::Compact).starts_with("alice@x.com"));
    }

    #[test]
    fn trigger_label_drops_text_at_icon_only() {
        assert!(!trigger_label("alice@x.com", StatusBarDensity::IconOnly).contains("alice@x.com"));
    }
}
