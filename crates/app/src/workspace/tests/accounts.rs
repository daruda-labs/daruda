//! `Workspace::clear_account_override` — the per-window account-delete
//! hook. Covers the usage-cache prune (`self.claude.plan_limits_by_account`
//! / `activity_by_account`) added alongside the existing pane-override
//! clear.

use super::*;
use daruda_store::accounts::AccountId;

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
