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
fn switching_worktrees_closes_the_orchestrator_tab_and_keeps_the_session(cx: &mut TestAppContext) {
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
            assert_eq!(
                ws.lane_ref_for_pane(pane),
                None,
                "a visible slot must not inherit its host worktree's session host"
            );

            ws.activate_lane(target, window, cx);
            assert_eq!(ws.active, target);
            // The orchestrator belongs to no worktree, so it does not follow
            // the user into the next one.
            assert!(
                !ws.orchestrator_tab_is_visible(),
                "switching worktrees takes the tab down"
            );
            for (lane_ref, rt) in &ws.main_area.runtimes {
                assert!(
                    rt.panes.iter().all(|p| p.id != pane),
                    "no worktree keeps a wrapper for it: {lane_ref:?}"
                );
            }
            assert_ne!(
                ws.active_runtime().focused_pane_id,
                pane,
                "focus lands on the incoming worktree's own pane"
            );

            // The session is what must survive: the slot still owns the same
            // view, and the chip can put it back.
            assert_eq!(ws.agent_chat_view(pane).unwrap().entity_id(), view);
            assert_eq!(ws.telegram_header(pane, cx), header);
            assert!(ws.show_orchestrator_tab(window, cx), "the chip reopens it");
            assert_eq!(ws.agent_chat_view(pane).unwrap().entity_id(), view);
        })
    })
    .unwrap();
}

#[gpui::test]
fn closing_the_orchestrator_tab_does_not_connect_the_worktree_being_left(cx: &mut TestAppContext) {
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

            // A dormant chat in the worktree the user is about to leave. Its
            // session starts on focus, so the close must not hand it focus.
            let dormant = ws
                .insert_agent_chat_pane(ws.agents[0].id.clone(), ws.active_lane_cwds(), window, cx)
                .unwrap();
            assert!(matches!(
                ws.agent_chat_view(dormant).unwrap().read(cx).status,
                AgentSessionStatus::Idle
            ));

            seed(ws, window, cx);
            assert!(ws.show_orchestrator_tab(window, cx));
            ws.activate_lane(second, window, cx);

            assert!(
                !ws.orchestrator_tab_is_visible(),
                "the switch took the tab down"
            );
            assert!(
                matches!(
                    ws.agent_chat_view(dormant).unwrap().read(cx).status,
                    AgentSessionStatus::Idle
                ),
                "taking the tab down must not focus — and so connect — the \
                 chat left behind in the outgoing worktree"
            );
        })
    })
    .unwrap();
}

#[gpui::test]
fn closing_a_project_stashes_the_orchestrator_draft_for_its_next_open(cx: &mut TestAppContext) {
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
            ws.close_active_project(window, cx);
            assert_eq!(ws.active, surviving);
            // Closing the project switches worktrees, which takes the
            // orchestrator's tab down with it.
            assert!(!ws.orchestrator_tab_is_visible());
            assert_ne!(ws.active_runtime().focused_pane_id, pane);
            assert_eq!(
                ws.agent_chat_view(pane).unwrap().entity_id(),
                view,
                "the session survives the project that hosted its tab"
            );

            // The unsent prompt is not lost with the tab — it comes back with
            // it, in whichever project survived.
            assert!(ws.show_orchestrator_tab(window, cx));
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
                let (saved_workspace, projects) = ws.snapshot_for_disk(cx);
                let saved = &projects[0].lanes[0];
                assert_eq!(saved.tabs.len(), 2);
                assert_eq!(saved.active_tab_index, expected, "active index {active}");
                assert!(saved.tabs.iter().all(|tab| tab.last_focused_pane != pane));
                assert_ne!(saved_workspace.focused_pane_id, pane);
            }
            // Filtering a trailing active slot clamps to the preceding user tab.
            ws.active_runtime_mut().tabs.swap(1, 2);
            ws.active_runtime_mut().active_tab_index = 2;
            let (_, projects) = ws.snapshot_for_disk(cx);
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
            tail_window_calls: None,
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
        assert_eq!(view.defaults, crate::workspace::main_area::agent_chat_pane::transcript_defaults::TranscriptDefaults::resolve(Some(&agent), ws.agent_content_width));
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

/// The tab is temporary chrome, so showing and hiding it must leave the
/// user's zoom exactly as they left it.
#[gpui::test]
fn showing_and_hiding_the_tab_restores_the_users_zoom(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let zoomed = ws.active_runtime().focused_pane_id;
            ws.main_area.zoomed_pane_id = Some(zoomed);
            seed(ws, window, cx);

            assert!(ws.show_orchestrator_tab(window, cx));
            assert_eq!(
                ws.main_area.zoomed_pane_id, None,
                "the tab has to be visible, so the zoom stands down"
            );

            ws.hide_orchestrator_tab(window, cx);
            assert_eq!(
                ws.main_area.zoomed_pane_id,
                Some(zoomed),
                "and comes back when the tab goes away"
            );
        });
    })
    .expect("live window");
}

/// A second window must not claim a session it does not host.
///
/// Before this, every non-host window read its own empty slot and rendered
/// "not started — click to start" while the host was working, and the click
/// did nothing at all.
#[gpui::test]
fn only_the_host_window_shows_a_chip_once_a_session_exists(cx: &mut TestAppContext) {
    use crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState;
    use gpui::BorrowAppContext as _;

    let (host_window, host) = build_workspace(cx);
    let (_other_window, other) = build_workspace(cx);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user()).clone();
            cfg.orchestrator.enabled = true;
            store.set_user_for_testing(cfg);
        });
    });

    // Nothing started yet: both windows offer to start it.
    for ws in [&host, &other] {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.orchestrator_chip_state(cx),
                Some(OrchestratorChipState::NotStarted)
            );
        });
    }

    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register_orchestrator(
            host_window.into(),
            host.downgrade(),
            cx,
        );
    });
    cx.update_window(host_window.into(), |_, window, cx| {
        host.update(cx, |ws, cx| seed(ws, window, cx));
    })
    .expect("live window");

    host.update(cx, |ws, cx| {
        assert_eq!(
            ws.orchestrator_chip_state(cx),
            Some(OrchestratorChipState::Idle),
            "the host reports the session"
        );
    });
    other.update(cx, |ws, cx| {
        assert_eq!(
            ws.orchestrator_chip_state(cx),
            None,
            "and no other window claims it"
        );
    });
}

/// The chip's click must actually start a session.
///
/// It runs inside the host window's event dispatch, so anything it calls that
/// re-enters `cx.update_window` on that same window gets "window not found"
/// (crates/app/src/CLAUDE.md — the May-2026 add-project regression). This
/// drives the click the way gpui does: from inside `update_window`.
#[gpui::test]
fn clicking_the_chip_starts_a_session_from_inside_the_windows_dispatch(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;

    let mut config = daruda_config::Config::default();
    let mut agent = config.resolved_agents().remove(0);
    agent.launch = daruda_config::AgentLaunch::Raw(test_process::command_line(&["--exit", "1"]));
    config.agents = vec![daruda_config::AgentEntry::Custom(agent)];
    config.orchestrator.enabled = true;
    let (window, workspace) = build_workspace_with(cx, &config, None);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(config.clone());
        });
        crate::orchestrator::seed_control_surface_for_test(cx);
        crate::window_registry::WindowRegistry::register(window.into(), workspace.downgrade(), cx);
    });

    let weak = workspace.downgrade();
    cx.update_window(window.into(), |_, win, cx| {
        crate::orchestrator::start_or_toggle_from_chip(&weak, win, cx);
    })
    .expect("live window");

    workspace.update(cx, |ws, _| {
        assert!(
            ws.orchestrator_chat.is_some(),
            "the click has to leave a session behind — the chip is the only \
             way the desktop can start one"
        );
        assert!(
            ws.orchestrator_tab_is_visible(),
            "and show it, which is what the person clicked for"
        );
    });
}

/// The chip is present from launch, before anything has started a session —
/// otherwise the desktop has no way to start one at all, and the orchestrator
/// is deliberately not started at launch.
#[gpui::test]
fn a_configured_orchestrator_shows_a_chip_before_it_has_a_session(cx: &mut TestAppContext) {
    use crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState;
    use gpui::BorrowAppContext as _;

    let (window, workspace) = build_workspace(cx);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user()).clone();
            cfg.orchestrator.enabled = true;
            store.set_user_for_testing(cfg);
        });
    });
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(ws.orchestrator_chat.is_none());
            assert_eq!(
                ws.orchestrator_chip_state(cx),
                Some(OrchestratorChipState::NotStarted),
                "configured but not started is a state, not an absence"
            );

            // Switching the feature off takes the chip away entirely: one that
            // could only refuse is noise.
            cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
                let mut cfg = (*store.user()).clone();
                cfg.orchestrator.enabled = false;
                store.set_user_for_testing(cfg);
            });
            assert_eq!(ws.orchestrator_chip_state(cx), None);

            // With a session, the chip reports the session's own activity.
            cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
                let mut cfg = (*store.user()).clone();
                cfg.orchestrator.enabled = true;
                store.set_user_for_testing(cfg);
            });
            seed(ws, window, cx);
            assert_eq!(
                ws.orchestrator_chip_state(cx),
                Some(OrchestratorChipState::Idle)
            );
        });
    })
    .expect("live window");
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
        telegram_ops::FIRST_RESPONSE_FALLBACK_SECS,
        view::{ActivitySpan, TurnOutcome},
    };
    let (window, workspace) = build_workspace(cx);
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let pane = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.telegram.enabled = true;
                ws.telegram.authorized_chat_id = Some(42);
                ws.telegram.only_when_away = false;
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
                        view.activity.span = ActivitySpan::Busy {
                            started_at: std::time::Instant::now(),
                        };
                        view.activity.pending_completion = Some(TurnOutcome::Completed);
                        view.start_phone_turn_for_test(
                            std::time::Instant::now()
                                - std::time::Duration::from_secs(FIRST_RESPONSE_FALLBACK_SECS + 1),
                        );
                    });
                // Fallback pump first, completion second — the order the
                // real pumps produce (a turn goes quiet, gets acked, then
                // ends) and the one the turn ledger encodes: the completion
                // closes the turn's phone conversation, so an ack owed to it
                // has to have gone out already.
                ws.flush_telegram_first_response_fallbacks(cx);
                ws.pulse_agent_chats(cx);
                let view = ws.agent_chat_view(pane).unwrap().read(cx);
                assert!(!view.activity.span.is_busy());
                assert!(view.activity.pending_completion.is_none());
                assert!(!view.is_phone_turn_waiting());
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
        ws.flush_telegram_first_response_fallbacks(cx);
        ws.pulse_agent_chats(cx);
    });
    cx.run_until_parked();
    assert!(
        outbound.next().now_or_never().is_none(),
        "no duplicate deliveries"
    );
}
