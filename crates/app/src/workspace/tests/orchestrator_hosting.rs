use super::{PaneId, Workspace};
use crate::workspace::tests::build_workspace;
use crate::workspace::tests::build_workspace_with;
use daruda_store::accounts::{AccountId, AccountSelection};
use futures::{FutureExt as _, StreamExt as _};
use gpui::{AppContext as _, TestAppContext};
use gpui::{Context, Window};

fn seed(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> PaneId {
    ws.seed_orchestrator_chat_pane_unrevealed_for_test(
        ws.agents[0].id.clone(),
        std::env::temp_dir(),
        AccountSelection::SystemDefault,
        None,
        window,
        cx,
    )
    .expect("slot seeded")
}

#[gpui::test]
fn orchestrator_tab_toggle_retains_view_and_restores_user_focus(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let original_focus = ws.active_runtime().focused_pane_id;
            let pane = seed(ws, window, cx);
            let view = ws.agent_chat_view(pane).unwrap().entity_id();
            ws.toggle_orchestrator_tab(window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, pane);
            assert_eq!(ws.active_runtime().tabs.len(), 2);
            assert_eq!(
                ws.every_agent_chat().filter(|(id, _)| *id == pane).count(),
                1
            );
            assert!(ws.lane_agent_chats().all(|(id, _)| id != pane));
            ws.toggle_orchestrator_tab(window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, original_focus);
            assert_eq!(ws.active_runtime().tabs.len(), 1);
            assert_eq!(ws.agent_chat_view(pane).unwrap().entity_id(), view);
        })
    })
    .unwrap();
}

#[gpui::test]
fn closing_the_only_orchestrator_tab_keeps_the_window_and_slot(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let view = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.empty_active_lane_runtime(window, cx);
                let pane = seed(ws, window, cx);
                let view = ws.agent_chat_view(pane).unwrap().entity_id();
                assert!(ws.show_orchestrator_tab(window, cx));
                assert_eq!(ws.total_open_tabs(), 0);
                ws.close_tab_at(0, window, cx);
                assert!(ws.active_runtime().tabs.is_empty());
                view
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(
        cx.update_window(window.into(), |_, _, _| ()).is_ok(),
        "hiding must keep the host alive"
    );
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.orchestrator_chat.as_ref().unwrap().view.entity_id(),
            view
        )
    });
}

#[gpui::test]
fn closing_last_user_tab_with_orchestrator_visible_closes_window(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            seed(ws, window, cx);
            assert!(ws.show_orchestrator_tab(window, cx));
            assert_eq!(ws.active_runtime().tabs.len(), 2);
            assert_eq!(ws.total_open_tabs(), 1);
            ws.close_tab_at(0, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        cx.update_window(window.into(), |_, _, _| ()).is_err(),
        "the orchestrator must not count as a user tab"
    );
}

#[gpui::test]
fn visible_orchestrator_moves_to_another_worktree_as_the_same_view(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let previous = ws.active;
            let mut target = previous;
            target.lane = ws.alloc_id();
            ws.projects[0]
                .lanes
                .push(crate::lane::Lane::default_for_project(
                    target.lane,
                    root.path().to_path_buf(),
                ));
            let pane = seed(ws, window, cx);
            let view = ws.agent_chat_view(pane).unwrap().entity_id();
            let header = ws.telegram_header(pane, cx);
            assert!(ws.show_orchestrator_tab(window, cx));
            assert_eq!(ws.telegram_header(pane, cx), header);
            ws.activate_lane(target, window, cx);
            assert_eq!(ws.active, target);
            assert_eq!(ws.active_runtime().focused_pane_id, pane);
            assert!(
                ws.main_area.runtimes[&previous]
                    .panes
                    .iter()
                    .all(|p| p.id != pane)
            );
            assert_eq!(
                ws.active_runtime()
                    .panes
                    .iter()
                    .find(|p| p.id == pane)
                    .unwrap()
                    .agent_chat_view()
                    .unwrap()
                    .entity_id(),
                view
            );
            assert_eq!(
                ws.lane_ref_for_pane(pane),
                None,
                "a visible slot must not inherit its host worktree's session host"
            );
            ws.hide_orchestrator_tab(window, cx);
            assert_eq!(ws.agent_chat_view(pane).unwrap().entity_id(), view);
        })
    })
    .unwrap();
}

#[gpui::test]
fn moving_the_orchestrator_does_not_connect_the_lane_being_left(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus;

    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let first = ws.active;
            let mut second = first;
            second.lane = ws.alloc_id();
            ws.projects[0]
                .lanes
                .push(crate::lane::Lane::default_for_project(
                    second.lane,
                    root.path().to_path_buf(),
                ));

            ws.activate_lane(second, window, cx);
            let dormant = ws
                .insert_agent_chat_pane(ws.agents[0].id.clone(), ws.active_lane_cwds(), window, cx)
                .unwrap();
            ws.set_focused_pane(dormant, window, cx);
            assert!(matches!(
                ws.agent_chat_view(dormant).unwrap().read(cx).status,
                AgentSessionStatus::Idle
            ));

            ws.activate_lane(first, window, cx);
            seed(ws, window, cx);
            assert!(ws.show_orchestrator_tab(window, cx));
            ws.activate_lane(second, window, cx);
            ws.activate_lane(first, window, cx);

            assert!(matches!(
                ws.agent_chat_view(dormant).unwrap().read(cx).status,
                AgentSessionStatus::Idle
            ));
        })
    })
    .unwrap();
}

#[gpui::test]
fn closing_a_project_preserves_the_orchestrator_draft_in_the_surviving_project(
    cx: &mut TestAppContext,
) {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(first.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let surviving = ws.active;
            ws.add_project(second.path().to_path_buf(), window, cx)
                .unwrap();
            let pane = seed(ws, window, cx);
            let view = ws.agent_chat_view(pane).unwrap().entity_id();
            ws.show_orchestrator_tab(window, cx);
            ws.terminal_input.update(cx, |input, cx| {
                input.set_value("Keep this unsent prompt", window, cx)
            });
            assert!(ws.close_active_project(window, cx));
            assert_eq!(ws.active, surviving);
            assert_eq!(ws.active_runtime().focused_pane_id, pane);
            assert_eq!(ws.agent_chat_view(pane).unwrap().entity_id(), view);
            assert_eq!(ws.input_owner, Some(pane));
            assert_eq!(
                ws.terminal_input.read(cx).value().as_str(),
                "Keep this unsent prompt"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn orchestrator_tab_rejects_split_merge_and_drag(cx: &mut TestAppContext) {
    use crate::workspace::main_area::{pane_tree::SplitDirection, tab_ops::NewPaneKind};
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let user_pane = ws.active_runtime().focused_pane_id;
            let user_tab = ws.active_runtime().tabs[0].id;
            let pane = seed(ws, window, cx);
            ws.show_orchestrator_tab(window, cx);
            let tab = ws.active_runtime().tabs[1].id;
            ws.split_from_tab(
                1,
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            ws.split_focused_pane_kind(NewPaneKind::Terminal, SplitDirection::Vertical, window, cx);
            assert_eq!(ws.active_runtime().panes.len(), 2);
            assert!(!ws.merge_tab_into_pane(
                user_tab,
                pane,
                SplitDirection::Horizontal,
                false,
                window,
                cx
            ));
            ws.active_runtime_mut().active_tab_index = 0;
            assert!(!ws.merge_tab_into_pane(
                tab,
                user_pane,
                SplitDirection::Horizontal,
                false,
                window,
                cx
            ));
            ws.main_area.tab_reorder_preview = Some((tab, 0));
            ws.drop_tab_onto_bar(tab, window, cx);
            assert_eq!(ws.active_runtime().tabs[1].id, tab);
            assert_eq!(ws.active_runtime().tabs[1].layout.pane_ids(), vec![pane]);
        })
    })
    .unwrap();
}

#[gpui::test]
fn visible_orchestrator_is_excluded_from_listings(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            seed(ws, window, cx);
            ws.show_orchestrator_tab(window, cx);
            let user_chat = ws.open_agent_chat_pane_for_test(window, cx);
            assert_eq!(
                ws.control_snapshot(cx)
                    .iter()
                    .map(|(_, chat)| chat.target.pane)
                    .collect::<Vec<_>>(),
                vec![user_chat]
            );
            assert_eq!(ws.control_lane_list()[0].chats, 1);
        })
    })
    .unwrap();
}

#[gpui::test]
fn visible_orchestrator_is_excluded_from_persisted_tab_indices(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = seed(ws, window, cx);
            ws.show_orchestrator_tab(window, cx);
            ws.open_agent_chat_pane_for_test(window, cx);
            assert_eq!(ws.active_runtime().tabs.len(), 3);
            for (active, expected) in [(0, 0), (1, 1), (2, 1)] {
                ws.active_runtime_mut().active_tab_index = active;
                ws.active_runtime_mut().focused_pane_id =
                    ws.active_runtime().tabs[active].last_focused_pane;
                let (saved_workspace, projects) = ws.snapshot_for_disk(cx).unwrap();
                let saved = &projects[0].lanes[0];
                assert_eq!(saved.tabs.len(), 2);
                assert_eq!(saved.active_tab_index, expected, "active index {active}");
                assert!(saved.tabs.iter().all(|tab| tab.last_focused_pane != pane));
                assert_ne!(saved_workspace.focused_pane_id, pane);
            }
            // Filtering a trailing active slot clamps to the preceding user tab.
            ws.active_runtime_mut().tabs.swap(1, 2);
            ws.active_runtime_mut().active_tab_index = 2;
            let (_, projects) = ws.snapshot_for_disk(cx).unwrap();
            assert_eq!(projects[0].lanes[0].active_tab_index, 1);
        })
    })
    .unwrap();
}

#[gpui::test]
fn hidden_orchestrator_tracks_live_config_updates(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let pane = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| seed(ws, window, cx))
        })
        .unwrap();
    workspace.update(cx, |ws, cx| {
        let agent = daruda_config::AgentDefinition {
            id: ws.agents[0].id.clone(),
            name: "Renamed orchestrator".into(),
            tail_window: Some(3),
            ..ws.agents[0].clone()
        };
        let mut config = daruda_config::Config {
            agents: vec![daruda_config::AgentEntry::Custom(agent.clone())],
            ..daruda_config::Config::default()
        };
        config.file_viewer.syntax_theme = "InspiredGitHub".into();
        ws.apply_config(&config, cx);
        let view = ws.agent_chat_view(pane).unwrap().read(cx);
        assert_eq!(view.agent_name, agent.name);
        assert_eq!(view.defaults, crate::workspace::main_area::agent_chat_pane::transcript_defaults::TranscriptDefaults::resolve(Some(&agent)));
        assert_eq!(view.syntax_theme(), config.file_viewer.syntax_theme);
    });
}

#[gpui::test]
fn hidden_orchestrator_account_cleanup_updates_the_canonical_slot(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let account = AccountId::new();
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws
                .seed_orchestrator_chat_pane_unrevealed_for_test(
                    ws.agents[0].id.clone(),
                    std::env::temp_dir(),
                    AccountSelection::Managed(account),
                    None,
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(ws.panes_referencing_account(account), 1);
            ws.show_orchestrator_tab(window, cx);
            assert_eq!(ws.panes_referencing_account(account), 1);
            ws.hide_orchestrator_tab(window, cx);
            ws.clear_account_override(account, cx);
            assert_eq!(ws.panes_referencing_account(account), 0);
            assert_eq!(
                ws.agent_chat_account_selection(pane),
                AccountSelection::SystemDefault
            );
            ws.show_orchestrator_tab(window, cx);
            assert_eq!(
                ws.active_runtime()
                    .panes
                    .iter()
                    .find(|p| p.id == pane)
                    .unwrap()
                    .account_selection(),
                Some(AccountSelection::SystemDefault)
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn hidden_orchestrator_preserves_user_tabs_and_resolves_its_account(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    let account = AccountSelection::Managed(AccountId::new());
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let before: Vec<_> = ws.active_runtime().panes.iter().map(|p| p.id).collect();
            assert_eq!(ws.orchestrator_chip_state(cx), None);
            let pane = ws.seed_orchestrator_chat_pane_unrevealed_for_test(
                ws.agents[0].id.clone(),
                std::env::temp_dir(),
                account,
                Some("Test briefing".into()),
                window,
                cx,
            ).expect("slot seeded");
            assert_eq!(ws.active_runtime().panes.iter().map(|p| p.id).collect::<Vec<_>>(), before);
            assert!(ws.agent_chat_view(pane).is_some());
            assert_eq!(ws.orchestrator_chip_state(cx), Some(crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState::Idle));
            assert_eq!(ws.agent_chat_account_selection(pane), account);
            ws.agent_chat_view(pane).cloned().unwrap().update(cx, |view, _| {
                view.status = super::super::main_area::agent_chat_pane::view::AgentSessionStatus::Connected;
            });
            assert!(!ws.agent_chat_statuses(cx).iter().any(|(id, _)| *id == pane));
        });
    }).expect("live window");
}

#[gpui::test]
fn hidden_orchestrator_pulse_emits_completion_and_phone_fallback(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::{
        telegram_ops::FIRST_RESPONSE_FALLBACK_SECS, view::TurnOutcome,
    };
    let (window, workspace) = build_workspace(cx);
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let pane = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.telegram.enabled = true;
                ws.telegram.authorized_chat_id = Some(42);
                ws.telegram.defer_while_active = false;
                let pane = ws
                    .seed_orchestrator_chat_pane_unrevealed_for_test(
                        ws.agents[0].id.clone(),
                        std::env::temp_dir(),
                        AccountSelection::SystemDefault,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                ws.agent_chat_view(pane)
                    .cloned()
                    .unwrap()
                    .update(cx, |view, _| {
                        view.activity.was_busy = true;
                        view.activity.pending_completion = Some(TurnOutcome::Completed);
                        view.start_telegram_first_response_watch_for_test(
                            std::time::Instant::now()
                                - std::time::Duration::from_secs(FIRST_RESPONSE_FALLBACK_SECS + 1),
                        );
                    });
                ws.pulse_agent_chats(cx);
                ws.flush_telegram_first_response_fallbacks(cx);
                let view = ws.agent_chat_view(pane).unwrap().read(cx);
                assert!(!view.activity.was_busy);
                assert!(view.activity.pending_completion.is_none());
                assert!(!view.is_waiting_for_telegram_first_response());
                pane
            })
        })
        .unwrap();
    cx.run_until_parked();
    for _ in 0..2 {
        let Some(Some(crate::telegram::bridge::Outbound::Ping(ping))) =
            outbound.next().now_or_never()
        else {
            panic!("both completion and fallback must be emitted for the hidden session");
        };
        assert_eq!(ping.pane.pane, pane);
    }
    workspace.update(cx, |ws, cx| {
        ws.pulse_agent_chats(cx);
        ws.flush_telegram_first_response_fallbacks(cx);
    });
    cx.run_until_parked();
    assert!(
        outbound.next().now_or_never().is_none(),
        "no duplicate deliveries"
    );
}
