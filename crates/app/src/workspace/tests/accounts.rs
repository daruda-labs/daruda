//! Per-pane account switching + `Workspace::clear_account_override` (the
//! per-window account-delete hook, including its usage-cache prune).

use super::agent_chat::agent_view;
use super::*;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::ChatItem;
use daruda_store::accounts::{AccountId, AccountSelection};
use daruda_store::project::PaneCwd;

/// An Agent chat pane pinned to `selection`, pushed onto the active runtime.
fn seed_agent_pane(
    ws: &mut Workspace,
    selection: AccountSelection,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> PaneId {
    let mut pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        None,
        daruda_config::AgentDefinition::claude_default().id,
        None,
        window,
        cx,
    );
    pane.agent_chat_content_mut().unwrap().account = selection;
    let id = pane.id;
    ws.active_runtime_mut().panes.push(pane);
    id
}

/// A pane already switched to a managed account must be switchable back to
/// the system default — the bug this test guards: before the fix,
/// `switch_pane_account`'s target could not express "system default", so a
/// managed-account pane had no path back to `~/.claude`. With
/// [`AccountSelection`] the two states are distinct and both reachable.
#[gpui::test]
async fn switch_pane_account_reverts_a_managed_account_to_system_default(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::Managed(managed))
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::SystemDefault, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::SystemDefault),
            "switching to SystemDefault must revert the pane to the system default"
        );
    });
}

/// Picking the account the pane is already on must change nothing. Before the
/// guard, the click ran the full in-place switch — `/clear`'s teardown — so
/// re-selecting the checked entry in the status-bar dropdown destroyed the
/// conversation for no gain.
#[gpui::test]
async fn switch_pane_account_to_the_current_selection_is_a_no_op(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let id = seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx);
                agent_view(ws, id).update(cx, |v, _| {
                    v.items.push(ChatItem::UserText("keep me".into()));
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::Managed(managed), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            agent_view(ws, pane_id).read(cx).items.len(),
            1,
            "re-selecting the account the pane already uses must not reset the session"
        );
    });
}

/// An idle pane holding a conversation must not be wiped by a switch. The new
/// account needs its own ACP session and the transcript cannot move to it, so
/// the switch confirms first and opens a *new* pane — until that confirmation
/// the source pane keeps both its conversation and its own account.
#[gpui::test]
async fn switch_pane_account_keeps_an_idle_conversation(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let id = seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx);
                agent_view(ws, id).update(cx, |v, _| {
                    v.items
                        .push(ChatItem::UserText("a long investigation".into()));
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    let panes_before = workspace.read_with(cx, |ws, _| ws.active_runtime().panes.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::SystemDefault, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            agent_view(ws, pane_id).read(cx).items.len(),
            1,
            "an idle pane's conversation must survive an account switch"
        );
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::Managed(managed)),
            "the source pane keeps its own account — the new one goes to the new pane"
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            panes_before,
            "the new pane only opens once the user confirms the dialog"
        );
    });
}

/// A freshly created pane (the `Cmd+T` path) must be seeded with the
/// provider's configured default account, not left `SystemDefault` — since
/// the `resolve_account_config_dir` fix, `SystemDefault` means the explicit
/// system default (`~/.claude`), so a pane that should inherit the default
/// account needs that written onto its own selection at creation time
/// instead of relying on a resolve-time fallback.
#[gpui::test]
async fn create_pane_seeds_the_configured_provider_default(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let default_id = AccountId::new();
    workspace.update(cx, |ws, _| {
        ws.accounts
            .accounts
            .push(daruda_store::accounts::ManagedAccount {
                id: default_id,
                provider: daruda_store::accounts::AgentProvider::Claude,
                email: None,
                organization: None,
                config_dir: std::env::temp_dir(),
                created_at: 0,
                last_authenticated_at: 0,
            });
        ws.accounts
            .default_by_provider
            .insert(daruda_store::accounts::AgentProvider::Claude, default_id);
    });

    let pane = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.create_pane(window, cx))
        })
        .unwrap()
        .expect("fresh pane spawn must succeed");

    assert_eq!(
        pane.account_selection(),
        Some(AccountSelection::Managed(default_id)),
        "a freshly created pane must be seeded with the provider default account"
    );
}

#[gpui::test]
fn clear_account_override_prunes_usage_caches_for_deleted_account_only(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let deleted = AccountId::new();
    let kept = AccountId::new();

    ws.update(cx, |ws, cx| {
        for key in [
            AccountSelection::Managed(deleted),
            AccountSelection::Managed(kept),
            AccountSelection::SystemDefault,
        ] {
            ws.claude
                .usage_by_account
                .set_plan_limits(key, daruda_claude::PlanLimits::default());
            ws.claude
                .usage_by_account
                .set_activity(key, daruda_claude::ActivityStats::default());
        }

        ws.clear_account_override(deleted, cx);

        let usage = &ws.claude.usage_by_account;
        assert!(
            usage
                .plan_limits(AccountSelection::Managed(deleted))
                .is_none()
        );
        assert!(usage.plan_limits(AccountSelection::Managed(kept)).is_some());
        assert!(usage.plan_limits(AccountSelection::SystemDefault).is_some());

        assert!(usage.activity(AccountSelection::Managed(deleted)).is_none());
        assert!(usage.activity(AccountSelection::Managed(kept)).is_some());
        assert!(usage.activity(AccountSelection::SystemDefault).is_some());
    });
}
