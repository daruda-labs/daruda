use super::*;
use crate::test_support::{workspace_for_control, workspace_with_agent_chat};
use gpui::BorrowAppContext as _;

#[gpui::test]
async fn listing_spans_every_window(cx: &mut gpui::TestAppContext) {
    let _a = workspace_with_agent_chat(cx);
    let _b = workspace_with_agent_chat(cx);
    cx.update(|cx| {
        let listing = listing(cx);
        assert_eq!(listing.windows.len(), 2, "both windows enumerated");
        assert_eq!(listing.windows[0].index, 0);
        assert_eq!(listing.windows[1].index, 1);
    });
}

/// Calling `listing` twice against an unmutated `HashMap` yields the same
/// order whether or not `collect_rows` sorts — `RandomState` varies per
/// map, not per iteration — so equality alone proves nothing. This asserts
/// the *expected* sequence over a fixture with several lanes, which is
/// what actually fails if the sort is dropped.
#[gpui::test]
async fn listing_order_follows_the_sort_key_not_hashmap_order(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    // Extra lanes, each with a chat pane, so `runtimes` holds enough
    // entries for hash order to diverge from tab order.
    let extra = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_panes_in_fresh_lanes_for_test(3, window, cx)
            })
        })
        .expect("window is live");

    let panes: Vec<u64> = cx.update(|cx| {
        listing(cx)
            .windows
            .into_iter()
            .flat_map(|w| w.projects)
            .flat_map(|p| p.lanes)
            .flat_map(|l| l.chats)
            .map(|c| c.target.pane)
            .collect()
    });

    let mut expected = vec![fixture.pane()];
    expected.extend(extra);
    expected.sort_unstable();
    assert_eq!(
        panes, expected,
        "rows must follow (window, project, lane order, pane), not hash order"
    );
}

/// The counts `/brief` answers with, each non-zero and each distinct — the
/// earlier test only checked `total`, so three of the four fields of the
/// reply were unverified.
#[gpui::test]
async fn brief_counts_each_state_separately(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let panes = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_panes_in_fresh_lanes_for_test(2, window, cx)
            })
        })
        .expect("window is live");

    fixture.workspace.update(cx, |ws, cx| {
        ws.set_pane_working_for_test(fixture.pane(), cx);
        ws.set_pane_awaiting_permission_for_test(panes[0], cx);
        ws.set_pane_errored_for_test(panes[1], cx);
    });

    cx.update(|cx| {
        let brief = brief(cx);
        assert_eq!(brief.total, 3);
        assert_eq!(brief.working, 1, "{brief:?}");
        assert_eq!(brief.awaiting_permission, 1, "{brief:?}");
        assert_eq!(brief.error, 1, "{brief:?}");
    });
}

#[gpui::test]
async fn brief_counts_across_windows(cx: &mut gpui::TestAppContext) {
    let _a = workspace_with_agent_chat(cx);
    let _b = workspace_with_agent_chat(cx);
    cx.update(|cx| {
        assert_eq!(brief(cx).total, 2);
    });
}

#[gpui::test]
async fn a_window_with_no_chat_pane_contributes_no_rows(cx: &mut gpui::TestAppContext) {
    let _empty = workspace_for_control(cx);
    cx.update(|cx| {
        assert!(listing(cx).windows.is_empty());
        assert_eq!(brief(cx).total, 0);
    });
}

/// Run a gated `ChatNew` with a bridge that can actually deliver the card.
/// Enabled *and* paired, because `send_approval_card` refuses otherwise —
/// an undeliverable card settles the request instead of waiting.
fn paired_bridge_run_gated(lane: LaneHandle, cx: &mut gpui::App) -> Dispatch {
    crate::settings_store::SettingsStore::init(cx);
    crate::telegram::global::install_for_test(true, Some(42), cx);
    cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
        store.set_user_for_testing(daruda_config::Config {
            telegram: daruda_config::TelegramConfig {
                enabled: true,
                authorized_chat_id: Some(42),
                ..Default::default()
            },
            ..daruda_config::Config::default()
        });
    });
    run_gated(
        GatedCommand::ChatNew { lane, agent: None },
        crate::control::exec::CommandOrigin::Orchestrator,
        cx,
    )
}

/// A name git will reject cannot become a worktree however the user
/// answers, so it must not cost them a tap — the same rule the budget
/// follows.
#[gpui::test]
async fn an_unusable_branch_name_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(false, None, cx);
        // `..` is a name git refuses outright, unlike one that is merely
        // already taken (which needs a `git` call to notice).
        let dispatch = run_gated(
            GatedCommand::LaneCreate {
                workspace,
                project,
                name: "..".into(),
                base_ref: None,
                agent: None,
                prompt: None,
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        );
        match dispatch {
            Dispatch::Ready(outcome) => {
                assert_eq!(outcome, Err(ControlError::LaneNameInvalid));
            }
            Dispatch::Deferred { .. } => panic!("an unusable name must not ask"),
        }
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            0,
            "no card was put on the phone"
        );
    });
}

/// A budget the agent has spent is not a decision the user gets to make,
/// so it is answered before a card ever reaches their phone.
#[gpui::test]
async fn a_spent_budget_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let (workspace, project) = fixture
        .workspace
        .read_with(cx, |ws, _| (ws.uuid(), ws.control_active_lane().project));
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(false, None, cx);
        for _ in 0..crate::control::guards::AGENT_LANE_BUDGET {
            crate::control::guards::note_agent_created_lane(cx);
        }
        let dispatch = run_gated(
            GatedCommand::LaneCreate {
                workspace,
                project,
                name: "x".into(),
                base_ref: None,
                agent: None,
                prompt: None,
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        );
        match dispatch {
            Dispatch::Ready(outcome) => {
                assert_eq!(outcome, Err(ControlError::AgentLimitReached));
            }
            Dispatch::Deferred { .. } => panic!("a spent budget must not ask"),
        }
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            0,
            "no card was put on the phone"
        );
    });
}

/// Same reasoning for a worktree that is not there: nothing to approve.
#[gpui::test]
async fn an_unknown_lane_refuses_before_the_card(cx: &mut gpui::TestAppContext) {
    let _fixture = workspace_with_agent_chat(cx);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        crate::telegram::global::install_for_test(false, None, cx);
        let dispatch = run_gated(
            GatedCommand::ChatNew {
                lane: LaneHandle {
                    workspace: daruda_store::project::WorkspaceUuid::new(),
                    project: 9_999,
                    lane: 9_999,
                },
                agent: None,
            },
            crate::control::exec::CommandOrigin::Orchestrator,
            cx,
        );
        match dispatch {
            Dispatch::Ready(outcome) => assert_eq!(outcome, Err(ControlError::TargetGone)),
            Dispatch::Deferred { .. } => panic!("a missing worktree must not ask"),
        }
        assert_eq!(crate::control::approval::waiting_count_for_test(cx), 0);
    });
}

/// A command the guards allow does reach the user — and waits.
#[gpui::test]
async fn an_allowed_command_asks_and_waits(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let lane = fixture.workspace.read_with(cx, |ws, _| {
        LaneHandle::new(ws.uuid(), ws.control_active_lane())
    });
    let dispatch = cx.update(|cx| paired_bridge_run_gated(lane, cx));
    let rx = match dispatch {
        Dispatch::Deferred { outcome, .. } => outcome,
        Dispatch::Ready(outcome) => panic!("expected a wait, got {outcome:?}"),
    };
    cx.update(|cx| {
        assert_eq!(
            crate::control::approval::waiting_count_for_test(cx),
            1,
            "the user is being asked"
        );
    });
    assert!(rx.is_empty(), "nothing is answered until they decide");
}

/// A refusal is the answer, not a silent nothing — the tool call is
/// waiting on it.
#[gpui::test]
async fn a_refused_command_reports_the_refusal(cx: &mut gpui::TestAppContext) {
    let fixture = workspace_with_agent_chat(cx);
    let lane = fixture.workspace.read_with(cx, |ws, _| {
        LaneHandle::new(ws.uuid(), ws.control_active_lane())
    });
    let dispatch = cx.update(|cx| paired_bridge_run_gated(lane, cx));
    let Dispatch::Deferred { outcome: rx, .. } = dispatch else {
        panic!("expected a wait");
    };
    cx.update(|cx| {
        crate::control::approval::resolve_only_pending_for_test(
            crate::control::approval::ApprovalChoice::Refused,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        rx.recv().await,
        Ok(Some(Err(ControlError::ApprovalRefused)))
    );
}

#[gpui::test]
async fn say_to_an_unknown_workspace_is_target_gone(cx: &mut gpui::TestAppContext) {
    let _a = workspace_with_agent_chat(cx);
    cx.update(|cx| {
        let target = PaneRef {
            workspace: daruda_store::project::WorkspaceUuid::new(),
            pane: 1,
        };
        assert_eq!(
            run(
                ResolvedCommand::Say {
                    target,
                    text: "hi".into()
                },
                CommandOrigin::Orchestrator,
                cx
            ),
            Err(ControlError::TargetGone)
        );
    });
}
