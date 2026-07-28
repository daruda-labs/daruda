//! Status bar's account slot — the focused pane's account (or the
//! "System" fallback), rendered as a dropdown trigger + menu for
//! switching between managed accounts.

use crate::surface::strings::account_recipe_label;
use crate::ui::theme;
use crate::ui::{DropdownMenu as _, PopupMenu, PopupMenuItem, button_status_pill_bare, spinner};
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::{AddManagedAccount, OpenSettings, Workspace};
use daruda_config::AgentLaunch;
use daruda_store::accounts::{AccountRecipeId, AccountSelection, AccountsState, ManagedAccount};
use gpui::{App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

const ACCOUNT_SLOT_MAX_WIDTH: f32 = 220.0;
const ACCOUNT_SLOT_COMPACT_MAX_WIDTH: f32 = 150.0;

/// The account-carrying pane kinds, as the render snapshot resolves them.
/// File / TaskEdit panes track no account and never reach here.
pub(in crate::workspace) enum SlotPane {
    Terminal,
    /// Agent chat, carrying the launch of the agent it runs (`None` when
    /// that agent id is no longer in the catalog).
    AgentChat(Option<AgentLaunch>),
}

/// The auth domain the focused pane's dropdown operates in. A pane's
/// account must belong to the domain its process signs into, so the domain
/// decides which accounts are even offered — `pane::resolve_pane_account`
/// refuses a mismatch, and the dropdown must not present a choice the
/// resolver will then reject.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum SlotDomain {
    /// Terminal pane — no agent, so every managed account is selectable and
    /// the account's own recipe decides what gets injected.
    Any,
    /// Agent-chat pane whose adapter has an auth domain.
    Scoped(AccountRecipeId),
    /// Agent-chat pane on a remote or unrecognized adapter — no local
    /// browser OAuth, so it can hold no managed account.
    Unsupported,
}

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
    /// Inert row — an add entry with no login command behind it, or the
    /// note explaining why this pane can hold no managed account.
    Inert(SharedString),
}

impl SlotDomain {
    /// Derive the domain from the focused pane. An agent-chat pane is
    /// scoped to *its own* agent's domain rather than the session's active
    /// agent, so a Codex pane never offers Claude accounts.
    pub(in crate::workspace) fn for_pane(pane: &SlotPane) -> Self {
        match pane {
            SlotPane::Terminal => Self::Any,
            SlotPane::AgentChat(launch) => launch
                .as_ref()
                .and_then(|launch| launch.account_recipe())
                .map_or(Self::Unsupported, Self::Scoped),
        }
    }

    /// Label for the always-present "System" entry. A scoped domain names
    /// the ambient home it falls back to; the other two have no single one,
    /// so the path is left off rather than guessed.
    fn system_label(self) -> String {
        match self {
            Self::Scoped(recipe) => crate::surface::strings::status_bar_account_system(
                daruda_claude::accounts::recipe_for(recipe).system_home_hint(),
            ),
            Self::Any | Self::Unsupported => {
                crate::surface::strings::status_bar_account_system_plain()
            }
        }
    }

    /// Managed accounts this pane may switch to, grouped one section per
    /// recipe. Empty sections are dropped so the menu never shows a bare
    /// header.
    fn sections(self, accounts: &AccountsState) -> Vec<AccountSection> {
        let recipes: Vec<AccountRecipeId> = match self {
            Self::Any => AccountRecipeId::ALL.to_vec(),
            Self::Scoped(recipe) => vec![recipe],
            Self::Unsupported => Vec::new(),
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

    /// The dropdown's add-account block. `login_available` answers whether
    /// `Workspace::login_command_for_recipe` resolves anything for a domain
    /// — a row with no command behind it is inert so the affordance doesn't
    /// invite a click `add_managed_account` would immediately reject.
    fn add_rows(self, login_available: impl Fn(AccountRecipeId) -> bool) -> Vec<AddAccountRow> {
        let add_row = |recipe: AccountRecipeId, label: String| {
            if login_available(recipe) {
                AddAccountRow::Add {
                    recipe,
                    label: label.into(),
                }
            } else {
                AddAccountRow::Inert(label.into())
            }
        };
        match self {
            Self::Scoped(recipe) => vec![add_row(
                recipe,
                crate::surface::strings::status_bar_add_account(),
            )],
            // A terminal may run any adapter, so each domain gets its own
            // named entry — "+ Add account" alone wouldn't say which
            // credentials it signs into.
            Self::Any => AccountRecipeId::ALL
                .iter()
                .map(|&recipe| {
                    add_row(
                        recipe,
                        crate::surface::strings::settings_accounts_add(&account_recipe_label(
                            recipe,
                        )),
                    )
                })
                .collect(),
            Self::Unsupported => vec![
                AddAccountRow::Inert(crate::surface::strings::status_bar_add_account().into()),
                AddAccountRow::Inert(
                    crate::surface::strings::status_bar_account_unsupported().into(),
                ),
            ],
        }
    }
}

/// Display + dropdown data for the focused pane's account, shown as a
/// clickable slot in the status bar's right section. Resolved by the
/// snapshot builder in `render/mod.rs` from the focused pane's
/// [`AccountSelection`] + `Workspace.accounts`.
#[derive(Clone)]
pub(in crate::workspace) struct AccountSlot {
    /// `"email (plan)"`, `"email"`, or the domain's "System" fallback.
    pub label: SharedString,
    /// The pane's auth domain — decides the "System" entry's wording.
    pub domain: SlotDomain,
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
        domain: SlotDomain,
        accounts: &AccountsState,
        login_available: impl Fn(AccountRecipeId) -> bool,
        workspace: WeakEntity<Workspace>,
        login_pending: bool,
    ) -> Self {
        let resolved = selection.account_id().and_then(|id| accounts.find(id));
        // Status-bar slot shows the email only — the organization is omitted
        // here (it's still shown in the account dropdown to disambiguate
        // same-email accounts).
        let label = account_label(resolved.and_then(|a| a.email.as_deref()), None)
            .unwrap_or_else(|| domain.system_label());
        Self {
            label: label.into(),
            domain,
            pane_id,
            current: selection,
            sections: domain.sections(accounts),
            workspace,
            add_rows: domain.add_rows(login_available),
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
            PopupMenuItem::new(SharedString::from(slot.domain.system_label()))
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
        section.accounts.iter().fold(menu, |m, account| {
            let workspace = slot.workspace.clone();
            let pane_id = slot.pane_id;
            let account_id = account.id;
            let is_current = slot.current == AccountSelection::Managed(account_id);
            let label = account_label(account.email.as_deref(), account.organization.as_deref())
                .unwrap_or_else(crate::surface::strings::settings_accounts_unknown_email);
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
                    PopupMenuItem::new(label.clone()).on_click(move |_, window, app| {
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

        let claude = SlotDomain::Scoped(AccountRecipeId::Claude).sections(&state);
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].recipe, AccountRecipeId::Claude);
        assert_eq!(emails(&claude[0]), ["alice@x.com", "carol@x.com"]);

        let codex = SlotDomain::Scoped(AccountRecipeId::Codex).sections(&state);
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].recipe, AccountRecipeId::Codex);
        assert_eq!(emails(&codex[0]), ["bob@x.com"]);
    }

    #[test]
    fn any_domain_groups_every_account_in_a_stable_order() {
        let sections = SlotDomain::Any.sections(&mixed_state());
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
        let sections = SlotDomain::Any.sections(&state);
        assert_eq!(
            sections.iter().map(|s| s.recipe).collect::<Vec<_>>(),
            [AccountRecipeId::Codex]
        );
    }

    #[test]
    fn unsupported_domain_lists_no_accounts() {
        assert!(SlotDomain::Unsupported.sections(&mixed_state()).is_empty());
    }

    #[test]
    fn system_label_names_the_scoped_domains_home() {
        assert!(
            SlotDomain::Scoped(AccountRecipeId::Claude)
                .system_label()
                .contains("~/.claude")
        );
        assert!(
            SlotDomain::Scoped(AccountRecipeId::Codex)
                .system_label()
                .contains("~/.codex")
        );
        let any = SlotDomain::Any.system_label();
        assert!(!any.contains("~/.claude") && !any.contains("~/.codex"));
    }

    #[test]
    fn terminal_pane_takes_every_domain() {
        assert_eq!(SlotDomain::for_pane(&SlotPane::Terminal), SlotDomain::Any);
    }

    #[test]
    fn agent_chat_pane_scopes_to_its_own_adapter() {
        let claude = SlotPane::AgentChat(Some(AgentLaunch::Raw(
            "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
        )));
        assert_eq!(
            SlotDomain::for_pane(&claude),
            SlotDomain::Scoped(AccountRecipeId::Claude)
        );

        let codex = SlotPane::AgentChat(Some(AgentLaunch::Raw(
            "npx -y @agentclientprotocol/codex-acp@latest".into(),
        )));
        assert_eq!(
            SlotDomain::for_pane(&codex),
            SlotDomain::Scoped(AccountRecipeId::Codex)
        );
    }

    #[test]
    fn agent_chat_pane_on_a_remote_launch_has_no_domain() {
        let remote = [
            SlotPane::AgentChat(Some(AgentLaunch::Ssh {
                adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
                host: "build-box".into(),
            })),
            SlotPane::AgentChat(Some(AgentLaunch::Docker {
                adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
                container: "dev".into(),
            })),
            SlotPane::AgentChat(Some(AgentLaunch::Raw(
                "ssh box sh -c 'cd \"{{cwd}}\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
                    .into(),
            ))),
            SlotPane::AgentChat(None),
        ];
        for pane in &remote {
            assert_eq!(SlotDomain::for_pane(pane), SlotDomain::Unsupported);
        }
    }

    #[test]
    fn unsupported_domain_offers_no_login_and_says_why() {
        let rows = SlotDomain::Unsupported.add_rows(|_| true);
        assert!(
            rows.iter()
                .all(|row| matches!(row, AddAccountRow::Inert(_))),
            "an unsupported pane must offer no clickable add entry"
        );
        assert_eq!(rows.len(), 2, "add entry plus the explanatory note");
    }

    #[test]
    fn terminal_domain_offers_one_named_add_entry_per_recipe() {
        let rows = SlotDomain::Any.add_rows(|recipe| recipe == AccountRecipeId::Claude);
        assert_eq!(rows.len(), AccountRecipeId::ALL.len());
        assert!(matches!(
            rows[0],
            AddAccountRow::Add {
                recipe: AccountRecipeId::Claude,
                ..
            }
        ));
        // Codex resolves no login command here, so its entry is inert.
        assert!(matches!(rows[1], AddAccountRow::Inert(_)));
    }
}
