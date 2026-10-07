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

/// The tab strip's mark rides the same entry: a lane switch is a visibility
/// change like any other, so going back clears the pane's unseen outcome.
#[gpui::test]
async fn visiting_the_lane_clears_its_tab_outcome(cx: &mut TestAppContext) {
    let (wh, ws, pane_id, home, other, dir) = two_lanes_with_an_agent(cx);
    ws.update(cx, |ws, cx| ws.set_window_active(true, cx));
    activate(wh, &ws, other, cx);
    finish(&ws, pane_id, TurnOutcome::Completed, cx);
    let unseen = |ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext| {
        ws.read_with(cx, |ws, _| ws.unseen_outcomes.for_panes(&[pane_id]).next())
    };
    assert_eq!(unseen(&ws, cx), Some(daruda_agent::AgentOutcome::Completed));
    // The fixture's root is missing so the pane stays offline; a lane whose
    // root is missing draws an empty state, not the pane. Give it a root so
    // going back actually puts the pane on screen.
    std::fs::create_dir_all(dir.path().join("gone")).unwrap();
    activate(wh, &ws, home, cx);
    assert_eq!(unseen(&ws, cx), None);
}

/// A terminal Claude's `Stop` reaches the lane mark through the same entry
/// as a chat turn's settle edge.
#[gpui::test]
async fn a_terminal_claude_finishing_in_a_parked_lane_marks_it_unread(cx: &mut TestAppContext) {
    use daruda_agent::SessionStatus;
    use daruda_agent::hooks::status_file::{StatusFile, write_atomic};

    let (wh, ws, pane_id, home, other, dir) = two_lanes_with_an_agent(cx);
    ws.update(cx, |ws, _| {
        ws.claude.pty_claude_bindings.insert(
            pane_id,
            daruda_agent::pty_tracker::PtyBinding {
                claude_pid: 4242,
                session_id: "sess-lane".into(),
            },
        );
    });
    activate(wh, &ws, other, cx);
    let path = dir.path().join("sess-lane.json");
    for (status, event) in [
        (SessionStatus::Working, "UserPromptSubmit"),
        (SessionStatus::Idle, "Stop"),
    ] {
        write_atomic(
            &path,
            &StatusFile::new_hook("sess-lane", dir.path(), status, event),
        )
        .unwrap();
        ws.update(cx, |ws, cx| {
            ws.apply_claude_status_event(
                crate::hooks::watcher::StatusEvent::Changed(path.clone()),
                cx,
            )
        });
        cx.run_until_parked();
    }
    assert!(unread(&ws, home, cx));
}
