//! The Dock badge: how many worktrees want a look — an agent waiting on the
//! user or failed, or a finished turn the user has not seen.

use super::*;
use daruda_store::project::LaneRef;

/// A project with an agent pane that failed, plus a second, empty lane. The
/// pane is built rather than opened, so no adapter is spawned.
fn failed_agent_and_a_second_lane(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    LaneRef,
    LaneRef,
    tempfile::TempDir,
) {
    use crate::workspace::main_area::pane::TabEntry;
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path());
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                None,
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            let tab_id = ws.alloc_id();
            ws.active_runtime_mut().tabs.push(TabEntry {
                id: tab_id,
                layout: PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            });
            super::agent_chat::agent_view(ws, pane_id).update(cx, |v, _| {
                v.set_status_for_test(
                    crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus::Error {
                        message: "adapter exited".into(),
                        remedy: daruda_acp::Remedy::NoneAvailable,
                    },
                );
            });
        });
    })
    .unwrap();
    let (home, other) = ws.update(cx, |ws, _| {
        let home = ws.active;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, dir.path().join("other"));
        lane.tab_order = 1;
        ws.project_for_mut(home.project).unwrap().lanes.push(lane);
        (
            home,
            LaneRef {
                project: home.project,
                lane: lane_id,
            },
        )
    });
    (wh, ws, home, other, dir)
}

fn count(ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext) -> usize {
    ws.read_with(cx, |ws, cx| ws.lanes_wanting_attention(cx))
}

#[gpui::test]
async fn a_failed_agent_counts_its_worktree(cx: &mut TestAppContext) {
    let (_wh, ws, _home, _other, _dir) = failed_agent_and_a_second_lane(cx);
    assert_eq!(count(&ws, cx), 1);
}

#[gpui::test]
async fn an_unread_worktree_counts_and_one_both_failed_and_unread_counts_once(
    cx: &mut TestAppContext,
) {
    let (_wh, ws, home, other, _dir) = failed_agent_and_a_second_lane(cx);
    ws.update(cx, |ws, _| ws.lane_for_mut(other).unwrap().is_unread = true);
    assert_eq!(count(&ws, cx), 2, "the failed lane and the unread one");
    ws.update(cx, |ws, _| ws.lane_for_mut(home).unwrap().is_unread = true);
    assert_eq!(
        count(&ws, cx),
        2,
        "a lane is one worktree however many reasons"
    );
}

#[gpui::test]
async fn the_badge_counts_the_registered_windows(cx: &mut TestAppContext) {
    let (wh, ws, _home, other, _dir) = failed_agent_and_a_second_lane(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    ws.update(cx, |ws, _| ws.lane_for_mut(other).unwrap().is_unread = true);
    cx.update(Workspace::refresh_dock_badge);
    cx.run_until_parked();
    let shown = cx.update(Workspace::dock_badge_shown);
    assert_eq!(shown, 2);
}

/// Nothing calls a refresh by hand in the app: a finished turn out of view
/// marks its lane unread, and that alone recounts the badge.
#[gpui::test]
async fn a_turn_finishing_out_of_view_raises_the_badge(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::view::TurnOutcome;
    let (wh, ws, _home, other, _dir) = failed_agent_and_a_second_lane(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let pane = ws.read_with(cx, |ws, _| ws.active_runtime().panes.last().unwrap().id);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(other, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        ws.fire_activity_completion(pane, TurnOutcome::Completed, cx)
    });
    cx.run_until_parked();
    let shown = cx.update(Workspace::dock_badge_shown);
    assert_eq!(
        shown, 1,
        "the home lane: failed and now unread, one worktree"
    );
}

/// Closing the tab that held a failed agent takes it off the badge — the
/// count is not left for the next unrelated status change to correct.
#[gpui::test]
async fn closing_a_failed_agents_tab_lowers_the_badge(cx: &mut TestAppContext) {
    let (wh, ws, _home, _other, _dir) = failed_agent_and_a_second_lane(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    cx.update(Workspace::refresh_dock_badge);
    cx.run_until_parked();
    assert_eq!(cx.update(Workspace::dock_badge_shown), 1);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let index = ws.active_runtime().tabs.len() - 1;
            ws.close_tab_at(index, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(Workspace::dock_badge_shown), 0);
}
