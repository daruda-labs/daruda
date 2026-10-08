//! The Dock badge: how many worktrees, across every window, want a look — an
//! agent waiting on the user or failed, or a finished turn not yet seen.
//! Recounted once per effect cycle however many status changes asked.

use gpui::{App, Global};

use daruda_agent::SessionStatus;

use crate::workspace::Workspace;

/// What the badge shows and whether a recount is already queued.
#[derive(Default)]
struct DockBadge {
    shown: usize,
    recount_queued: bool,
}

impl Global for DockBadge {}

impl Workspace {
    /// Worktrees in this window that want a look. One lane counts once,
    /// however many of its sessions — or reasons — there are.
    pub(in crate::workspace) fn lanes_wanting_attention(&self, cx: &App) -> usize {
        let pane_lane = self.pane_lane_index();
        let acp = self.agent_chat_statuses(cx);
        let (per_lane, _) = crate::workspace::claude_status_aggregate::aggregate_over_panes(
            &pane_lane,
            &self.claude.pty_claude_bindings,
            &self.claude.claude_status,
            &acp,
        );
        let mut lanes: std::collections::HashSet<_> = per_lane
            .into_iter()
            .filter(|(_, status)| {
                matches!(
                    status,
                    SessionStatus::NeedsAttention | SessionStatus::Failed
                )
            })
            .map(|(lane, _)| lane)
            .collect();
        lanes.extend(
            self.projects
                .lanes()
                .filter(|(_, _, lane)| lane.is_unread)
                .map(|(target, _, _)| target),
        );
        lanes.len()
    }

    /// Recount every window and put the total on the Dock icon. Deferred and
    /// coalesced: a status change can fire inside a workspace's own update,
    /// and the recount reads every workspace.
    pub(crate) fn refresh_dock_badge(cx: &mut App) {
        let badge = cx.default_global::<DockBadge>();
        if badge.recount_queued {
            return;
        }
        badge.recount_queued = true;
        cx.defer(|cx| {
            let mut total = 0;
            crate::window_registry::WindowRegistry::for_each_workspace(cx, |ws, _, cx| {
                total += ws.lanes_wanting_attention(cx);
            });
            let badge = cx.default_global::<DockBadge>();
            badge.recount_queued = false;
            if badge.shown != total {
                badge.shown = total;
                crate::platform::attention::set_badge_count(total);
            }
        });
    }

    #[cfg(test)]
    pub(in crate::workspace) fn dock_badge_shown(cx: &mut App) -> usize {
        cx.default_global::<DockBadge>().shown
    }
}
