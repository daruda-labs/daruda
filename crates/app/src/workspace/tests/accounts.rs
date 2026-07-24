//! `Workspace::clear_account_override` — the per-window account-delete
//! hook. Covers the usage-cache prune (`self.claude.plan_limits_by_account`
//! / `activity_by_account`) added alongside the existing pane-override
//! clear.

use super::*;
use daruda_store::accounts::AccountId;
use daruda_store::project::PaneCwd;

/// A pane already switched to a managed account must be switchable back to
/// the system default (`None`) — the bug this test guards: before the fix,
/// `switch_pane_account`'s `account_id` parameter was a concrete `AccountId`
/// with no way to express "system default", so a managed-account pane had
/// no path back to `~/.claude`.
#[gpui::test]
async fn switch_pane_account_reverts_a_managed_account_to_system_default(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let mut pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                pane.agent_chat_content_mut().unwrap().account_id = Some(managed);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
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
        assert_eq!(pane.account_id(), Some(Some(managed)));
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, None, window, cx);
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
            pane.account_id(),
            Some(None),
            "switching to None must revert the pane to the system default"
        );
    });
}

/// A freshly created pane (the `Cmd+T` path) must be seeded with the
/// provider's configured default account, not left `None` — since the
/// `resolve_account_config_dir` fix, `None` means the explicit system
/// default (`~/.claude`), so a pane that should inherit the default account
/// needs that written onto its own `account_id` at creation time instead of
/// relying on a resolve-time fallback.
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
        pane.account_id(),
        Some(Some(default_id)),
        "a freshly created pane must be seeded with the provider default account"
    );
}

#[gpui::test]
fn clear_account_override_prunes_usage_caches_for_deleted_account_only(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let deleted = AccountId::new();
    let kept = AccountId::new();

    ws.update(cx, |ws, cx| {
        ws.claude
            .plan_limits_by_account
            .insert(Some(deleted), daruda_claude::PlanLimits::default());
        ws.claude
            .plan_limits_by_account
            .insert(Some(kept), daruda_claude::PlanLimits::default());
        ws.claude
            .plan_limits_by_account
            .insert(None, daruda_claude::PlanLimits::default());
        ws.claude
            .activity_by_account
            .insert(Some(deleted), daruda_claude::ActivityStats::default());
        ws.claude
            .activity_by_account
            .insert(Some(kept), daruda_claude::ActivityStats::default());
        ws.claude
            .activity_by_account
            .insert(None, daruda_claude::ActivityStats::default());

        ws.clear_account_override(deleted, cx);

        assert!(
            !ws.claude
                .plan_limits_by_account
                .contains_key(&Some(deleted))
        );
        assert!(ws.claude.plan_limits_by_account.contains_key(&Some(kept)));
        assert!(ws.claude.plan_limits_by_account.contains_key(&None));

        assert!(!ws.claude.activity_by_account.contains_key(&Some(deleted)));
        assert!(ws.claude.activity_by_account.contains_key(&Some(kept)));
        assert!(ws.claude.activity_by_account.contains_key(&None));
    });
}
