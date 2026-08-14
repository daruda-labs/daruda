//! Accounts section of the Settings window — one block per auth domain
//! (`AccountRecipeId`), each listing a "System" row plus that domain's
//! managed accounts, with the per-domain default and delete/add/reauth
//! actions. Visual language mirrors the Agent-catalog section's bordered row
//! (see `sections::mod::render_agent_catalog_row`).
//!
//! Every domain renders whether or not it has accounts: the System row is how
//! the user says "no managed default here", and an absent `default_by_recipe`
//! entry is exactly that state.
//!
//! INVARIANT: `accounts.json` is shared across windows through the app-wide
//! `accounts_global::AccountsGlobal`. This section writes the file and then
//! publishes via `accounts_global::replace`, which fires `observe_global` on
//! every window symmetrically — never a manual per-window broadcast, which is
//! what let logins go stale in other windows.
//!
//! The add-account buttons are the one case needing a `Workspace` (the login
//! command comes from that window's agent catalog), so `start_add_account`
//! runs the login in `WindowRegistry::first_workspace`. The process-wide
//! login marker in `AccountsGlobal` disables competing Settings actions while
//! the target Workspace retains ownership of the process handle and Cancel.

use std::time::{SystemTime, UNIX_EPOCH};

use daruda_store::accounts::{AccountId, AccountRecipeId, ManagedAccount};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::{
    SettingsWindow, settings_button as button, settings_button_danger as button_danger,
};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::{ButtonVariant, Disableable as _};
use crate::window_registry::WindowRegistry;
use crate::workspace::accounts_global;
use crate::workspace::dialog_helpers::open_confirm_dialog;

/// Element-id fragment for a recipe's rows — internal, never displayed.
fn recipe_slug(recipe: AccountRecipeId) -> &'static str {
    match recipe {
        AccountRecipeId::Claude => "claude",
        AccountRecipeId::Codex => "codex",
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
/// this account — see `Workspace::panes_referencing_account`. The path
/// those panes revert to is the deleted account's own auth domain's
/// ambient home, not a fixed one.
fn remove_confirm_body(count: usize, recipe: AccountRecipeId) -> String {
    s::settings_accounts_remove_confirm_body(
        count,
        daruda_agent::accounts::recipe_for(recipe).system_home_hint(),
    )
}

/// Apply a default-account choice to `state`: `Some(id)` pins that account
/// for `recipe`, `None` removes the entry — the absent entry *is* the
/// "System" choice (see `AccountsState::default_by_recipe`).
fn apply_default_choice(
    state: &mut daruda_store::accounts::AccountsState,
    recipe: AccountRecipeId,
    account: Option<AccountId>,
) {
    match account {
        Some(id) => {
            state.default_by_recipe.insert(recipe, id);
        }
        None => {
            state.default_by_recipe.remove(&recipe);
        }
    }
}

/// Bordered card shared by the System row and every account row, so the
/// two can't drift apart visually.
fn row_card(cx: &gpui::App) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::MODAL_PANEL_GAP))
        .p(px(theme::MODAL_PANEL_GAP))
        .border_1()
        .border_color(theme::current(cx).border)
        .rounded(px(theme::RADIUS_MD))
}

/// A row's title + subtitle, with the default badge trailing it when this
/// row is the domain's current default.
fn row_header(
    title: String,
    subtitle: String,
    is_default: bool,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    div()
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
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(subtitle),
                ),
        )
        .when(is_default, |header| {
            header.child(
                div()
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(t.text_muted)
                    .child(s::settings_accounts_default_badge()),
            )
        })
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

        for recipe in AccountRecipeId::all() {
            let default_id = self.accounts.default_by_recipe.get(&recipe).copied();
            body = body
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(theme::current(cx).text_primary)
                        .child(s::account_recipe_label(recipe)),
                )
                .child(self.render_system_row(recipe, default_id.is_none(), cx));
            for account in self.accounts.accounts.iter().filter(|a| a.recipe == recipe) {
                body = body.child(self.render_account_row(account, default_id, cx));
            }
            body = body.child(self.render_add_account_row(recipe, cx));
        }

        body.into_any_element()
    }

    /// The always-present first row of every auth domain: the ambient
    /// credentials a new pane runs under while that domain has no default
    /// account. Picking it removes the domain's `default_by_recipe` entry.
    fn render_system_row(
        &self,
        recipe: AccountRecipeId,
        is_default: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let slug = recipe_slug(recipe);
        let home = daruda_agent::accounts::recipe_for(recipe).system_home_hint();
        let actions = div()
            .flex()
            .flex_row()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .child(
                button(
                    SharedString::from(format!("settings-accounts-system-default-{slug}")),
                    s::settings_accounts_set_default(),
                )
                .disabled(is_default)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.set_default_account(recipe, None, cx);
                })),
            )
            // The ambient home can expire like any managed account, and it is
            // the default selection — without this row it is the one credential
            // set with no way back inside the app.
            .child(
                button(
                    SharedString::from(format!("settings-accounts-system-reauth-{slug}")),
                    if self.account_login_busy {
                        s::settings_accounts_authentication_in_progress()
                    } else {
                        s::settings_accounts_system_reauthenticate()
                    },
                )
                .disabled(self.account_login_busy)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.start_reauthenticate_system(recipe, cx);
                })),
            );

        row_card(cx)
            .child(row_header(
                s::settings_accounts_system_title(),
                home.to_string(),
                is_default,
                cx,
            ))
            .child(actions)
            .into_any_element()
    }

    fn render_account_row(
        &self,
        account: &ManagedAccount,
        default_id: Option<AccountId>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let account_id = account.id;
        let recipe = account.recipe;
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
                    this.set_default_account(recipe, Some(account_id), cx);
                })),
            )
            .child(
                button(
                    SharedString::from(format!("settings-accounts-reauth-{row_key}")),
                    if self.account_login_busy {
                        s::settings_accounts_authentication_in_progress()
                    } else {
                        s::settings_accounts_reauthenticate()
                    },
                )
                .disabled(self.account_login_busy)
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

        row_card(cx)
            .child(row_header(email, subtitle, is_default, cx))
            .child(actions)
            .into_any_element()
    }

    /// Starts a headless add-account login for `recipe` on click. The shared
    /// login marker disables this button as soon as the target Workspace
    /// begins preparing authentication.
    fn render_add_account_row(
        &self,
        recipe: AccountRecipeId,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        div().flex().flex_row().child(
            button(
                SharedString::from(format!("settings-accounts-add-{}", recipe_slug(recipe))),
                if self.account_login_busy {
                    s::settings_accounts_authentication_in_progress()
                } else {
                    s::settings_accounts_add(&s::account_recipe_label(recipe))
                },
            )
            .disabled(self.account_login_busy)
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.start_add_account(recipe, cx);
            })),
        )
    }

    /// Starts a headless add-account login for `recipe` against the first
    /// live `Workspace` window (`WindowRegistry::first_workspace` — see
    /// its doc for why "first" rather than the OS-active window: this is
    /// a Settings-window click handler, so `cx.active_window()` would
    /// resolve to the Settings window itself, not a workspace). The domain
    /// is the button the user pressed, not that window's active agent —
    /// `add_managed_account` resolves a login command for it.
    fn start_add_account(&mut self, recipe: AccountRecipeId, cx: &mut gpui::Context<Self>) {
        let Some((handle, weak)) = WindowRegistry::first_workspace(cx) else {
            self.error = Some(SharedString::from(s::settings_accounts_workspace_required()));
            cx.notify();
            LogWriter::log(
                ErrorReport::new("Add-account login has no open Workspace window to run against")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("settings.accounts.add_no_workspace")
                    .build(),
            );
            return;
        };
        self.error = None;
        let result = cx.update_window(handle, |_root, window, cx_w| {
            if let Some(ws) = weak.upgrade() {
                ws.update(cx_w, |ws, cx| {
                    ws.add_managed_account(recipe, window, cx);
                });
                true
            } else {
                false
            }
        });
        if !matches!(&result, Ok(true)) {
            self.error = Some(SharedString::from(s::settings_accounts_workspace_required()));
            cx.notify();
        }
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
    fn start_reauthenticate_account(
        &mut self,
        account_id: AccountId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.dispatch_login_action(
            Box::new(crate::workspace::ReauthenticateAccount(account_id)),
            "reauth",
            cx,
        );
    }

    /// Re-run the login for `recipe`'s ambient home — the credentials a pane
    /// with no managed account uses. Same dispatch shape as
    /// [`Self::start_reauthenticate_account`]; only the action differs,
    /// because a system login has no account id to name.
    fn start_reauthenticate_system(
        &mut self,
        recipe: AccountRecipeId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.dispatch_login_action(
            Box::new(crate::workspace::ReauthenticateSystem(recipe)),
            "system_reauth",
            cx,
        );
    }

    /// Hand a login action to the first open `Workspace` window, which is
    /// where the headless login machinery lives.
    ///
    /// Both failure modes surface the same user-facing message — there is no
    /// Workspace window to run against — and differ only in the diagnostic
    /// they log; `flow` names the caller in that log and its dedup key.
    fn dispatch_login_action(
        &mut self,
        action: Box<dyn gpui::Action>,
        flow: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((handle, _weak)) = WindowRegistry::first_workspace(cx) else {
            self.report_no_workspace(
                format!("{flow} login has no open Workspace window to run against"),
                format!("settings.accounts.{flow}_no_workspace"),
                None,
                cx,
            );
            return;
        };
        self.error = None;
        let result = cx.update_window(handle, |_root, window, cx_w| {
            window.dispatch_action(action, cx_w);
        });
        if let Err(e) = result {
            self.report_no_workspace(
                format!("Failed to start {flow} login: target Workspace window is gone"),
                format!("settings.accounts.{flow}_target_window_gone"),
                Some(e.to_string()),
                cx,
            );
        }
    }

    /// Surface "no Workspace window" inline in Settings and log why.
    fn report_no_workspace(
        &mut self,
        title: String,
        dedup: String,
        detail: Option<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.error = Some(SharedString::from(s::settings_accounts_workspace_required()));
        cx.notify();
        let mut report = ErrorReport::new(title)
            .severity(ErrorSeverity::Warning)
            .at(file!(), line!())
            .dedup(dedup);
        if let Some(detail) = detail {
            report = report.message(detail);
        }
        LogWriter::log(report.build());
    }

    /// Immediate (no confirm) — sets which account new panes of `recipe`
    /// start on (`None` = the system credentials), persists, and publishes
    /// to every window.
    fn set_default_account(
        &mut self,
        recipe: AccountRecipeId,
        account: Option<AccountId>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Apply the change to freshly-loaded disk state, not to this
        // window's possibly-stale snapshot: a Workspace may have added an
        // account since this Settings window was built, and a full
        // overwrite from `self.accounts` would silently destroy it. This
        // load-mutate-save narrows the cross-window race to the load→save
        // gap below rather than closing it — `save_accounts` renames into
        // place but takes no file lock, so two windows racing this exact
        // sequence can still each read the same state and one's write can
        // still overwrite the other's delta. A real fix needs a file lock
        // (fs4 is already a dependency but unused here) — tracked as a
        // follow-up, not done in this pass.
        let mut state = daruda_store::accounts::load_accounts().unwrap_or_default();
        apply_default_choice(&mut state, recipe, account);
        if let Err(e) = daruda_store::accounts::save_accounts(&state) {
            log_io_error(
                "Failed to save accounts.json after set-default",
                "settings.accounts.save_default_failed",
                &e,
            );
        }
        // Publish to the app-wide Global — `observe_global` refreshes every
        // window's mirror (including this one). Set this section's mirror
        // eagerly so its own render below is immediate.
        self.accounts = state.clone();
        accounts_global::replace(cx, state);
        cx.notify();
    }

    /// G9 confirm dialog before delete — destructive, and (per the
    /// brief) the body must show how many live panes revert to the
    /// system default. `referencing` sums
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
        // Gone already (a concurrent delete in another window): nothing to
        // confirm, and `remove_account` would be a no-op anyway.
        let Some(recipe) = self.accounts.find(account_id).map(|a| a.recipe) else {
            return;
        };
        let mut referencing = 0usize;
        WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
            referencing += ws.panes_referencing_account(account_id);
        });
        let weak = cx.weak_entity();
        open_confirm_dialog(
            s::settings_accounts_remove_confirm_title(),
            remove_confirm_body(referencing, recipe),
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
    /// isolated config dir, drops it from `AccountsState`,
    /// persists, then clears the override on every pane that referenced
    /// it (across every open Workspace window) and syncs their caches.
    fn remove_account(&mut self, account_id: AccountId, cx: &mut gpui::Context<Self>) {
        // Operate on freshly-loaded disk state (see `set_default_account`'s
        // comment for the same load-mutate-save shape and why it narrows,
        // but does not close, the cross-window race).
        let mut state = daruda_store::accounts::load_accounts().unwrap_or_default();
        let Some(account) = state.find(account_id).cloned() else {
            self.accounts = state.clone();
            accounts_global::replace(cx, state);
            cx.notify();
            return;
        };
        // The account's own auth domain owns the removal — its config dir plus
        // whatever OS credential entry is scoped to it.
        daruda_agent::accounts::recipe_for(account.recipe).cleanup(&account.config_dir);
        state.accounts.retain(|a| a.id != account_id);
        state.default_by_recipe.retain(|_, id| *id != account_id);
        if let Err(e) = daruda_store::accounts::save_accounts(&state) {
            log_io_error(
                "Failed to save accounts.json after delete",
                "settings.accounts.save_delete_failed",
                &e,
            );
        }
        // Reset every pane pinned to this account back to the system
        // default (+ prune its per-account usage cache) in every open
        // Workspace window — this is pane/cache state the Global doesn't
        // carry, so it stays a direct `for_each_workspace` sweep.
        WindowRegistry::for_each_workspace(cx, move |ws, _window, cx| {
            ws.clear_account_override(account_id, cx);
        });
        // Publish the account removal itself once — `observe_global`
        // refreshes every window's `accounts` mirror symmetrically.
        self.accounts = state.clone();
        accounts_global::replace(cx, state);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::accounts::{AccountsState, load_accounts_in, save_accounts_in};

    #[test]
    fn default_choice_round_trips_through_accounts_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let id = AccountId::new();
        let mut state = AccountsState::default();

        apply_default_choice(&mut state, AccountRecipeId::Codex, Some(id));
        save_accounts_in(dir.path(), &state).expect("save with a default");
        let loaded = load_accounts_in(dir.path()).expect("load with a default");
        assert_eq!(
            loaded.default_by_recipe.get(&AccountRecipeId::Codex),
            Some(&id)
        );

        let mut state = loaded;
        apply_default_choice(&mut state, AccountRecipeId::Codex, None);
        save_accounts_in(dir.path(), &state).expect("save without a default");
        let loaded = load_accounts_in(dir.path()).expect("load without a default");
        assert!(loaded.default_by_recipe.is_empty());
    }

    #[test]
    fn default_choice_leaves_other_recipes_alone() {
        let claude = AccountId::new();
        let mut state = AccountsState::default();
        apply_default_choice(&mut state, AccountRecipeId::Claude, Some(claude));
        apply_default_choice(&mut state, AccountRecipeId::Codex, Some(AccountId::new()));

        apply_default_choice(&mut state, AccountRecipeId::Codex, None);

        assert_eq!(
            state.default_by_recipe.get(&AccountRecipeId::Claude),
            Some(&claude)
        );
        assert!(
            !state
                .default_by_recipe
                .contains_key(&AccountRecipeId::Codex)
        );
    }

    #[test]
    fn every_recipe_has_a_label_and_a_system_home_hint() {
        for recipe in AccountRecipeId::all() {
            assert!(!s::account_recipe_label(recipe).is_empty());
            assert!(
                daruda_agent::accounts::recipe_for(recipe)
                    .system_home_hint()
                    .starts_with("~/")
            );
        }
    }

    #[test]
    fn recipe_slugs_are_unique_per_recipe() {
        assert_ne!(
            recipe_slug(AccountRecipeId::Claude),
            recipe_slug(AccountRecipeId::Codex)
        );
    }

    #[test]
    fn remove_confirm_counts_referencing_panes() {
        assert_eq!(
            remove_confirm_body(3, AccountRecipeId::Claude),
            "3 pane(s) using this account will revert to the system default (~/.claude)."
        );
    }

    #[test]
    fn remove_confirm_body_zero_panes() {
        assert_eq!(
            remove_confirm_body(0, AccountRecipeId::Claude),
            "0 pane(s) using this account will revert to the system default (~/.claude)."
        );
    }

    #[test]
    fn remove_confirm_body_names_the_accounts_own_domain() {
        assert_eq!(
            remove_confirm_body(1, AccountRecipeId::Codex),
            "1 pane(s) using this account will revert to the system default (~/.codex)."
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
