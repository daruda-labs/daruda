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
