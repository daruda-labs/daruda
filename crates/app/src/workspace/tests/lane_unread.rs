//! The sidebar's unread dot: an agent that finishes in a lane the user is not
//! looking at marks that lane, and visiting it clears the mark.

use super::*;
use crate::workspace::main_area::agent_chat_pane::view::TurnOutcome;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_store::project::LaneRef;

/// A project with an agent-chat pane in its first lane, plus an empty second
/// lane to move to.
fn two_lanes_with_an_agent(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    PaneId,
    LaneRef,
    LaneRef,
    tempfile::TempDir,
) {
    // A root that does not exist leaves the pane without a cwd, so it parks
    // in `Error` instead of spawning an adapter — the suite stays offline.
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path().join("gone"));
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let (pane_id, home, other) = ws.update(cx, |ws, _| {
        let pane_id = ws.active_runtime().focused_pane_id;
        let home = ws.active;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, dir.path().join("other"));
        lane.tab_order = 1;
        ws.project_for_mut(home.project).unwrap().lanes.push(lane);
        let other = LaneRef {
            project: home.project,
            lane: lane_id,
        };
        (pane_id, home, other)
    });
    (wh, ws, pane_id, home, other, dir)
}

fn activate(
    wh: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    target: LaneRef,
    cx: &mut TestAppContext,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(target, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

fn finish(
    ws: &gpui::Entity<Workspace>,
    pane_id: PaneId,
    outcome: TurnOutcome,
    cx: &mut TestAppContext,
) {
    ws.update(cx, |ws, cx| {
        ws.fire_activity_completion(pane_id, outcome, cx)
    });
    cx.run_until_parked();
}

fn unread(ws: &gpui::Entity<Workspace>, lane: LaneRef, cx: &mut TestAppContext) -> bool {
    ws.read_with(cx, |ws, _| ws.lane_for(lane).unwrap().is_unread)
}

#[gpui::test]
async fn a_turn_finishing_in_a_parked_lane_marks_it_unread(cx: &mut TestAppContext) {
    let (wh, ws, pane_id, home, other, _dir) = two_lanes_with_an_agent(cx);
    activate(wh, &ws, other, cx);
    finish(&ws, pane_id, TurnOutcome::Completed, cx);
    assert!(unread(&ws, home, cx), "the user has not seen this answer");
    assert!(!unread(&ws, other, cx));
}

#[gpui::test]
async fn a_turn_finishing_in_the_active_lane_leaves_it_read(cx: &mut TestAppContext) {
    let (_wh, ws, pane_id, home, _other, _dir) = two_lanes_with_an_agent(cx);
    finish(&ws, pane_id, TurnOutcome::Completed, cx);
    assert!(!unread(&ws, home, cx), "it finished in front of the user");
}

#[gpui::test]
async fn a_turn_the_user_stopped_leaves_the_lane_read(cx: &mut TestAppContext) {
    let (wh, ws, pane_id, home, other, _dir) = two_lanes_with_an_agent(cx);
    activate(wh, &ws, other, cx);
    finish(&ws, pane_id, TurnOutcome::Stopped, cx);
    assert!(!unread(&ws, home, cx), "a Stop is the user's own doing");
}

#[gpui::test]
async fn visiting_an_unread_lane_clears_it(cx: &mut TestAppContext) {
    let (wh, ws, pane_id, home, other, _dir) = two_lanes_with_an_agent(cx);
    activate(wh, &ws, other, cx);
    finish(&ws, pane_id, TurnOutcome::Errored, cx);
    assert!(unread(&ws, home, cx), "a failed turn wants a look too");
    activate(wh, &ws, home, cx);
    assert!(!unread(&ws, home, cx));
}
