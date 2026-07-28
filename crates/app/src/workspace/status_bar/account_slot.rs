//! Status bar's account slot — the focused pane's account (or the
//! "System" fallback), rendered as a dropdown trigger + menu for
//! switching between managed accounts.

use crate::surface::strings::account_recipe_label;
use crate::ui::theme;
use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, button_status_pill_bare, spinner};
use crate::workspace::main_area::pane::AccountDomain;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::{AddManagedAccount, OpenSettings, Workspace};
use daruda_store::accounts::{AccountRecipeId, AccountSelection, AccountsState, ManagedAccount};
use gpui::{App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

const ACCOUNT_SLOT_MAX_WIDTH: f32 = 220.0;
const ACCOUNT_SLOT_COMPACT_MAX_WIDTH: f32 = 150.0;

/// One recipe's block of switchable accounts in the dropdown.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct AccountSection {
    pub recipe: AccountRecipeId,
    /// Header naming the auth domain.
    pub label: SharedString,
    pub accounts: Vec<ManagedAccount>,
}

/// One row of the dropdown's add-account block.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) enum AddAccountRow {
    /// Clickable entry; starts a headless login filing the account under
    /// `recipe`.
    Add {
        recipe: AccountRecipeId,
        label: SharedString,
    },
    /// Inert row — the note explaining why this pane can hold no managed
    /// account.
    Inert(SharedString),
}

/// Full "System" wording for the dropdown entry and the pill's tooltip. A
/// scoped domain names the ambient home it falls back to; the other two have
/// no single one, so the path is left off rather than guessed.
fn system_label(domain: AccountDomain) -> String {
    match domain {
        AccountDomain::Exactly(recipe) => crate::surface::strings::status_bar_account_system(
            daruda_claude::accounts::recipe_for(recipe).system_home_hint(),
        ),
        AccountDomain::Any | AccountDomain::Unsupported => {
            crate::surface::strings::status_bar_account_system_plain()
        }
    }
}

/// Managed accounts this pane may switch to, grouped one section per recipe.
/// Empty sections are dropped so the menu never shows a bare header.
fn sections(domain: AccountDomain, accounts: &AccountsState) -> Vec<AccountSection> {
    let recipes: Vec<AccountRecipeId> = match domain {
        AccountDomain::Any => AccountRecipeId::ALL.to_vec(),
        AccountDomain::Exactly(recipe) => vec![recipe],
        AccountDomain::Unsupported => Vec::new(),
    };
    recipes
        .into_iter()
        .filter_map(|recipe| {
            let matching: Vec<ManagedAccount> = accounts
                .accounts
                .iter()
                .filter(|a| a.recipe == recipe)
                .cloned()
                .collect();
            (!matching.is_empty()).then(|| AccountSection {
                recipe,
                label: account_recipe_label(recipe).into(),
                accounts: matching,
            })
        })
        .collect()
}

/// The dropdown's add-account block. Every offered domain has a login command
/// (`Workspace::login_command_for_recipe` is total), so an add row is inert
/// only where no domain applies at all.
fn add_rows(domain: AccountDomain) -> Vec<AddAccountRow> {
    let add_row = |recipe: AccountRecipeId, label: String| AddAccountRow::Add {
        recipe,
        label: label.into(),
    };
    match domain {
        AccountDomain::Exactly(recipe) => vec![add_row(
            recipe,
            crate::surface::strings::status_bar_add_account(),
        )],
        // A terminal may run any adapter, so each domain gets its own named
        // entry — "+ Add account" alone wouldn't say which credentials it
        // signs into.
        AccountDomain::Any => AccountRecipeId::ALL
            .iter()
            .map(|&recipe| {
                add_row(
                    recipe,
                    crate::surface::strings::settings_accounts_add(&account_recipe_label(recipe)),
                )
            })
            .collect(),
        AccountDomain::Unsupported => vec![
            AddAccountRow::Inert(crate::surface::strings::status_bar_add_account().into()),
            AddAccountRow::Inert(crate::surface::strings::status_bar_account_unsupported().into()),
        ],
    }
}

/// Display + dropdown data for the focused pane's account, shown as a
/// clickable slot in the status bar's right section. Resolved by the
/// snapshot builder in `render/mod.rs` from the focused pane's
/// [`AccountSelection`] + `Workspace.accounts`.
#[derive(Clone)]
pub(in crate::workspace) struct AccountSlot {
    /// Pill text: the account's email, else a bare "System". Kept short —
    /// the status bar has ~150-220px here and an email needs all of it, so
    /// the domain's home path lives in [`Self::tooltip`] and the dropdown's
    /// System entry instead.
    pub label: SharedString,
    /// Untruncated wording for the pill's tooltip, since the pill itself
    /// clips: the email, or "System" naming the domain's ambient home.
    pub tooltip: SharedString,
    /// The pane's auth domain — decides the "System" entry's wording.
    pub domain: AccountDomain,
    /// The focused pane the dropdown's menu items dispatch
    /// `Workspace::switch_pane_account` against.
    pub pane_id: PaneId,
    /// The pane's own selection, so the dropdown can mark the active entry.
    pub current: AccountSelection,
    /// Accounts the pane can switch to, grouped per auth domain.
    pub sections: Vec<AccountSection>,
    /// Dispatch target for the dropdown's menu-item clicks.
    pub workspace: WeakEntity<Workspace>,
    /// The add-account block, already resolved against login availability.
    pub add_rows: Vec<AddAccountRow>,
    /// Mirrors `Workspace::is_login_pending` — swaps the add-account block
    /// for an in-progress row + Cancel while a headless login is running.
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
        domain: AccountDomain,
        accounts: &AccountsState,
        workspace: WeakEntity<Workspace>,
        login_pending: bool,
    ) -> Self {
        let resolved = selection.account_id().and_then(|id| accounts.find(id));
        // Status-bar slot shows the email only — the organization is omitted
        // here (it's still shown in the account dropdown to disambiguate
        // same-email accounts).
        let email = account_label(resolved.and_then(|a| a.email.as_deref()), None);
        let label = email
            .clone()
            .unwrap_or_else(crate::surface::strings::status_bar_account_system_plain);
        let tooltip = email.unwrap_or_else(|| system_label(domain));
        Self {
            label: label.into(),
            tooltip: tooltip.into(),
            domain,
            pane_id,
            current: selection,
            sections: sections(domain, accounts),
            workspace,
            add_rows: add_rows(domain),
            login_pending,
        }
    }
}

/// Pure label formatter for an account's identity: `email (plan)`, else
/// just `email`. `None` when the account has no captured email — callers
/// name their own fallback, since what "no identity" should read as
/// differs per surface.
pub(in crate::workspace) fn account_label(
    email: Option<&str>,
    plan: Option<&str>,
) -> Option<String> {
    match (email, plan) {
        (Some(email), Some(plan)) => Some(format!("{email} ({plan})")),
        (Some(email), None) => Some(email.to_string()),
        (None, _) => None,
    }
}

/// Mark for the trigger pill: only a pane pinned to one auth domain has a
/// single agent to name. A pane that accepts any domain (or none) gets no
/// icon rather than an arbitrary one.
fn trigger_icon(domain: AccountDomain) -> Option<&'static str> {
    match domain {
        AccountDomain::Exactly(recipe) => Some(crate::agent::icons::icon_for_recipe(recipe)),
        AccountDomain::Any | AccountDomain::Unsupported => None,
    }
}

/// Render the account slot's dropdown trigger button.
pub(super) fn render(
    slot: &AccountSlot,
    density: super::StatusBarDensity,
    cx: &App,
) -> impl IntoElement {
    // The label rides every density: it is short by construction (an email or
    // a bare "System"), and dropping it left a bare chevron with nothing to
    // say which account was active.
    let label = slot.label.clone();
    let max_width = if density == super::StatusBarDensity::Full {
        ACCOUNT_SLOT_MAX_WIDTH
    } else {
        ACCOUNT_SLOT_COMPACT_MAX_WIDTH
    };
    let tooltip = slot.tooltip.clone();
    let slot = slot.clone();
    button_status_pill_bare("status-account", cx)
        .max_w(px(max_width))
        .overflow_hidden()
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .tooltip(tooltip)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .min_w_0()
                .gap(px(theme::STATUS_BAR_USAGE_CHIP_GAP))
                .children(trigger_icon(slot.domain).map(|path| {
                    crate::ui::agent_icon(
                        Some(path),
                        px(theme::STATUS_BAR_AGENT_ICON_SIZE),
                        theme::current(cx).text_muted,
                    )
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .truncate()
                        .child(label),
                )
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

/// Build the account slot's dropdown menu from the resolved slot: the
/// domain-named "System default" entry (always present so a managed-account
/// pane can always be reverted), then one headed section per auth domain
/// with a checkmark on the pane's current override, then either the
/// add-account block or — while a headless login runs (`login_pending`) —
/// an in-progress row + Cancel in its place, and finally a "manage
/// accounts" entry that opens Settings. Each item is a one-line dispatch
/// into a `Workspace` method or action (render purity; no logic here).
fn build_account_menu(slot: &AccountSlot, menu: PopupMenu) -> PopupMenu {
    let menu = {
        let workspace = slot.workspace.clone();
        let pane_id = slot.pane_id;
        let is_current = slot.current == AccountSelection::SystemDefault;
        menu.item(
            PopupMenuItem::new(SharedString::from(system_label(slot.domain)))
                .checked(is_current)
                .on_click(move |_, window, app| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(app, |ws, cx| {
                            ws.switch_pane_account(
                                pane_id,
                                AccountSelection::SystemDefault,
                                window,
                                cx,
                            )
                        });
                    }
                }),
        )
    };
    let menu = slot.sections.iter().fold(menu, |menu, section| {
        let menu = menu.separator().label(section.label.clone()).separator();
        // The section header carries no icon slot (`PopupMenuItem::Label`), so
        // the mark rides each row — which is also what disambiguates a mixed
        // list once more than one domain has accounts.
        let icon = crate::agent::icons::icon_for_recipe(section.recipe);
        section.accounts.iter().fold(menu, |m, account| {
            let workspace = slot.workspace.clone();
            let pane_id = slot.pane_id;
            let account_id = account.id;
            let is_current = slot.current == AccountSelection::Managed(account_id);
            let label = account_label(account.email.as_deref(), account.organization.as_deref())
                .unwrap_or_else(crate::surface::strings::settings_accounts_unknown_email);
            m.item(
                PopupMenuItem::new(SharedString::from(label))
                    .icon(crate::ui::agent_menu_icon(Some(icon)))
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
        })
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
        slot.add_rows.iter().fold(menu, |m, row| match row {
            AddAccountRow::Add { recipe, label } => {
                let recipe = *recipe;
                m.item(
                    PopupMenuItem::new(label.clone())
                        .icon(crate::ui::agent_menu_icon(Some(
                            crate::agent::icons::icon_for_recipe(recipe),
                        )))
                        .on_click(move |_, window, app| {
                            window.dispatch_action(Box::new(AddManagedAccount(recipe)), app);
                        }),
                )
            }
            AddAccountRow::Inert(label) => m.item(PopupMenuItem::new(label.clone()).disabled(true)),
        })
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
    use super::*;
    use crate::workspace::status_bar::StatusBarDensity;
    use daruda_store::accounts::AccountId;
    use std::path::PathBuf;

    fn account(email: &str, recipe: AccountRecipeId) -> ManagedAccount {
        ManagedAccount {
            id: AccountId::new(),
            recipe,
            email: Some(email.to_string()),
            organization: None,
            config_dir: PathBuf::from("/tmp/account"),
            created_at: 0,
            last_authenticated_at: 0,
        }
    }

    fn mixed_state() -> AccountsState {
        AccountsState {
            accounts: vec![
                account("alice@x.com", AccountRecipeId::Claude),
                account("bob@x.com", AccountRecipeId::Codex),
                account("carol@x.com", AccountRecipeId::Claude),
            ],
            ..AccountsState::default()
        }
    }

    fn emails(section: &AccountSection) -> Vec<String> {
        section
            .accounts
            .iter()
            .filter_map(|a| a.email.clone())
            .collect()
    }

    #[test]
    fn account_slot_label_prefers_email_then_falls_back() {
        assert_eq!(
            account_label(Some("alice@x.com"), Some("Team")).as_deref(),
            Some("alice@x.com (Team)")
        );
        assert_eq!(
            account_label(Some("alice@x.com"), None).as_deref(),
            Some("alice@x.com")
        );
        assert_eq!(account_label(None, None), None);
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

    #[test]
    fn scoped_domain_lists_only_its_own_accounts() {
        let state = mixed_state();

        let claude = sections(AccountDomain::Exactly(AccountRecipeId::Claude), &state);
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].recipe, AccountRecipeId::Claude);
        assert_eq!(emails(&claude[0]), ["alice@x.com", "carol@x.com"]);

        let codex = sections(AccountDomain::Exactly(AccountRecipeId::Codex), &state);
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].recipe, AccountRecipeId::Codex);
        assert_eq!(emails(&codex[0]), ["bob@x.com"]);
    }

    #[test]
    fn any_domain_groups_every_account_in_a_stable_order() {
        let sections = sections(AccountDomain::Any, &mixed_state());
        assert_eq!(
            sections.iter().map(|s| s.recipe).collect::<Vec<_>>(),
            AccountRecipeId::ALL
        );
        assert_eq!(emails(&sections[0]), ["alice@x.com", "carol@x.com"]);
        assert_eq!(emails(&sections[1]), ["bob@x.com"]);
    }

    #[test]
    fn any_domain_skips_a_recipe_with_no_accounts() {
        let state = AccountsState {
            accounts: vec![account("bob@x.com", AccountRecipeId::Codex)],
            ..AccountsState::default()
        };
        let sections = sections(AccountDomain::Any, &state);
        assert_eq!(
            sections.iter().map(|s| s.recipe).collect::<Vec<_>>(),
            [AccountRecipeId::Codex]
        );
    }

    #[test]
    fn unsupported_domain_lists_no_accounts() {
        assert!(sections(AccountDomain::Unsupported, &mixed_state()).is_empty());
    }

    #[test]
    fn system_label_names_the_scoped_domains_home() {
        assert!(
            system_label(AccountDomain::Exactly(AccountRecipeId::Claude)).contains("~/.claude")
        );
        assert!(system_label(AccountDomain::Exactly(AccountRecipeId::Codex)).contains("~/.codex"));
        let any = system_label(AccountDomain::Any);
        assert!(!any.contains("~/.claude") && !any.contains("~/.codex"));
    }

    #[test]
    fn unsupported_domain_offers_no_login_and_says_why() {
        let rows = add_rows(AccountDomain::Unsupported);
        assert!(
            rows.iter()
                .all(|row| matches!(row, AddAccountRow::Inert(_))),
            "an unsupported pane must offer no clickable add entry"
        );
        assert_eq!(rows.len(), 2, "add entry plus the explanatory note");
    }

    #[test]
    fn terminal_domain_offers_one_named_add_entry_per_recipe() {
        let rows = add_rows(AccountDomain::Any);
        assert_eq!(rows.len(), AccountRecipeId::ALL.len());
        // Every domain has a login command (the built-in adapter backs any
        // catalog gap), so a terminal offers each one as clickable.
        for (row, expected) in rows.iter().zip(AccountRecipeId::ALL) {
            let AddAccountRow::Add { recipe, label } = row else {
                panic!("every terminal add entry is clickable, got {row:?}");
            };
            assert_eq!(*recipe, expected);
            assert!(
                label.contains(&account_recipe_label(expected)),
                "the entry must name the domain it signs into: {label}"
            );
        }
    }
}
