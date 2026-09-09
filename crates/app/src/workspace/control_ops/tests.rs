use super::*;
use crate::test_support::workspace_with_agent_chat;
use gpui::AppContext as _;

#[test]
fn health_separates_a_dead_session_from_one_that_never_started() {
    assert_eq!(
        map_health(&AgentSessionStatus::Error {
            message: "boom".into(),
            remedy: daruda_acp::Remedy::NoneAvailable,
        }),
        Health::Error
    );
    assert_eq!(map_health(&AgentSessionStatus::Idle), Health::Unavailable);
    assert_eq!(map_health(&AgentSessionStatus::Connected), Health::Ok);
    assert_eq!(map_health(&AgentSessionStatus::Connecting), Health::Ok);
}

#[test]
fn every_activity_state_maps_to_its_own_control_variant() {
    assert_eq!(map_activity(ActivityState::Idle), Activity::Idle);
    assert_eq!(map_activity(ActivityState::Working), Activity::Working);
    assert_eq!(
        map_activity(ActivityState::AwaitingPermission),
        Activity::AwaitingPermission
    );
}

/// A worktree with no agent chat in it still exists and is still a place
/// to open one — which is exactly why this is not derived from
/// `control_snapshot`.
#[gpui::test]
async fn lane_list_reports_every_lane_not_just_chatty_ones(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let extra = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                let project = ws.active.project;
                let root = ws.project_for(project).expect("project").root.clone();
                let lane_id = ws.alloc_id();
                let lane = crate::lane::Lane::default_for_project(lane_id, root);
                ws.project_for_mut(project)
                    .expect("project")
                    .lanes
                    .push(lane);
                let _ = (window, cx);
                LaneRef {
                    project,
                    lane: lane_id,
                }
            })
        })
        .expect("window is live");

    fixture.workspace.read_with(cx, |ws, _| {
        let lanes = ws.control_lane_list();
        assert!(
            lanes.len() >= 2,
            "the chatless lane is listed too: {lanes:?}"
        );
        assert!(lanes.iter().all(|l| !l.name.is_empty()));
        let chatless = lanes
            .iter()
            .find(|l| l.target.lane_ref() == extra)
            .expect("the added lane");
        assert_eq!(chatless.chats, 0);
        assert!(!chatless.is_active);
        let active = lanes.iter().find(|l| l.is_active).expect("one active lane");
        assert_eq!(active.chats, 1, "the fixture's chat is counted");
    });
}

#[gpui::test]
async fn chat_new_opens_a_pane_in_the_named_lane(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let before = fixture
        .workspace
        .read_with(cx, |ws, cx| ws.control_snapshot(cx).len());
    let target = fixture.workspace.read_with(cx, |ws, _| ws.active);

    // The insert half only: revealing spawns a real ACP adapter, whose
    // background task outlives the test and then trips gpui's determinism
    // assert in whichever test runs next.
    let pane = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                ws.control_insert_chat_for_test(target, window, cx)
            })
        })
        .expect("window is live")
        .expect("created");

    fixture.workspace.read_with(cx, |ws, cx| {
        let snap = ws.control_snapshot(cx);
        assert_eq!(snap.len(), before + 1);
        assert!(snap.iter().any(|(_, s)| s.target.pane == pane));
        assert_eq!(
            ws.control_active_lane(),
            target,
            "the pane went into the worktree that was named"
        );
    });
}

#[gpui::test]
async fn chat_new_on_a_missing_lane_is_target_gone(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let bogus = LaneRef {
        project: 9_999,
        lane: 9_999,
    };
    let result = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture
                .workspace
                .update(cx, |ws, cx| ws.control_chat_new(bogus, None, window, cx))
        })
        .expect("window is live");
    assert_eq!(result, Err(ControlError::TargetGone));
}

#[gpui::test]
async fn a_lane_plan_for_a_missing_project_is_target_gone(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.read_with(cx, |ws, _| {
        assert!(matches!(
            ws.control_lane_plan(9_999, "x", None),
            Err(ControlError::TargetGone)
        ));
    });
}

/// The plan has to name a sibling of the repo, so a lane an agent made
/// sits where a lane the user made would.
/// An agent-supplied name reaches `git worktree add -b` and a filesystem
/// path, so it is validated before anything is spawned and before the
/// approval card quotes it. Letting git reject it instead would cost the
/// user a tap and then blame them for it.
#[gpui::test]
async fn a_hostile_lane_name_is_refused_before_anything_runs(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.read_with(cx, |ws, _| {
        let project = ws.control_active_lane().project;
        for name in [
            "",
            "   ",
            "../../pwn",
            "has:colon",
            "has space",
            "trailing.",
            "/leading",
            "tilde~",
            "star*",
            "back\\slash",
        ] {
            assert!(
                matches!(
                    ws.control_lane_plan(project, name, None),
                    Err(ControlError::LaneNameInvalid)
                ),
                "{name:?} must not reach git"
            );
        }
    });
}

/// A slash is legal in a branch name but must not nest the checkout: the
/// path suffix folds it, exactly as the create form does.
#[gpui::test]
async fn a_slashed_branch_name_stays_one_directory_deep(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.read_with(cx, |ws, _| {
        let project = ws.control_active_lane().project;
        let root = ws.project_for(project).expect("project").root.clone();
        let plan = ws
            .control_lane_plan(project, "feat/x", None)
            .expect("a slash is a legal branch name");
        assert_eq!(plan.branch, "feat/x");
        assert_eq!(plan.new_path.parent(), root.parent());
        assert!(
            plan.new_path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-feat-x")),
            "{:?}",
            plan.new_path
        );
    });
}

#[gpui::test]
async fn a_lane_plan_puts_the_checkout_beside_its_repo(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.read_with(cx, |ws, _| {
        let project = ws.active.project;
        let root = ws.project_for(project).expect("project").root.clone();
        let plan = ws
            .control_lane_plan(project, "fix-picker", None)
            .expect("planned");
        assert_eq!(plan.branch, "fix-picker");
        assert_eq!(plan.repo_root, root);
        assert_eq!(plan.new_path.parent(), root.parent());
        let name = plan
            .new_path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("name");
        assert!(name.ends_with("-fix-picker"), "{name}");
        assert!(plan.session_host.is_none());
    });
}

#[gpui::test]
async fn snapshot_lists_agent_chat_panes_only(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.read_with(cx, |ws, cx| {
        let snap = ws.control_snapshot(cx);
        assert_eq!(
            snap.len(),
            1,
            "one agent chat pane, terminal panes excluded"
        );
        assert_eq!(snap[0].1.target.pane, fixture.pane());
        assert_eq!(snap[0].1.target.workspace, ws.uuid());
        assert!(
            snap[0].1.is_active_lane,
            "the pane opened in the active lane"
        );
    });
}

#[gpui::test]
async fn snapshot_carries_the_session_title(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.update(cx, |ws, cx| {
        let view = ws.agent_chat_view(fixture.pane()).expect("view").clone();
        view.update(cx, |v, _| {
            v.set_session_title_for_test("restore invariants")
        });
    });
    fixture.workspace.read_with(cx, |ws, cx| {
        let snap = ws.control_snapshot(cx);
        assert_eq!(snap[0].1.title.as_deref(), Some("restore invariants"));
    });
}

/// The fixture pane has no live session, so the prompt lands in the pane
/// queue — which is exactly the branch `Queued` exists to report rather
/// than pass off as delivered.
#[gpui::test]
async fn say_on_a_pane_with_no_live_session_reports_queued(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.update(cx, |ws, cx| {
        assert_eq!(
            ws.control_say(fixture.pane(), "hello".into(), cx),
            Ok(SendDisposition::Queued)
        );
    });
}

#[gpui::test]
async fn say_on_a_missing_pane_is_target_gone(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.update(cx, |ws, cx| {
        assert_eq!(
            ws.control_say(9_999, "hello".into(), cx),
            Err(ControlError::TargetGone)
        );
    });
}

#[gpui::test]
async fn stop_on_an_idle_pane_reports_already_idle(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.update(cx, |ws, cx| {
        assert_eq!(
            ws.control_stop(fixture.pane(), cx),
            Ok(StopDisposition::AlreadyIdle)
        );
    });
}

#[gpui::test]
async fn stop_on_a_missing_pane_is_target_gone(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    fixture.workspace.update(cx, |ws, cx| {
        assert_eq!(ws.control_stop(9_999, cx), Err(ControlError::TargetGone));
    });
}
