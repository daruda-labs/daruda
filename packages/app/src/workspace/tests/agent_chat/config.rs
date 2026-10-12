//! A config edit reaching panes that are already open: each pane's own agent
//! defaults, the vocabulary its session reports, and live mode/model defaults.

use super::*;

/// A live `[agent]` edit must reach an already-open pane, not wait for the next
/// restore: a pane the user never touched follows config, a pane they chose for
/// keeps its own settings.
#[gpui::test]
async fn a_config_reload_moves_an_untouched_panes_transcript_settings(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let before = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_live_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (untouched_id, chosen_id) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let open = |ws: &mut Workspace, window: &mut gpui::Window, cx: &mut _| {
                    let pane = ws.create_agent_chat_pane(
                        cwd.clone(),
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
                    pane_id
                };
                let untouched_id = open(ws, window, cx);
                let chosen_id = open(ws, window, cx);
                (untouched_id, chosen_id)
            })
        })
        .unwrap();

    // The second pane's user makes an explicit choice on all three settings.
    let chosen = workspace.read_with(cx, |ws, _| agent_view(ws, chosen_id));
    cx.update_window(window_handle.into(), |_, window, cx| {
        chosen.update(cx, |v, cx| {
            v.set_tail_window(TailLevel::Steps, TailWindow::Last(2), cx);
            v.toggle_display_facet(FilterFacet::Prose, cx);
            v.set_fold_mode(FoldPreset::Expanded.mode(), window, cx);
        });
    })
    .expect("the test window is live");
    cx.run_until_parked();

    let after = daruda_config::Config {
        agents: vec![claude_entry_with(5, "summary")],
        ..daruda_config::Config::default()
    };
    assert_ne!(FoldPreset::Summary.mode(), FoldMode::default());

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let untouched = workspace.read_with(cx, |ws, _| agent_view(ws, untouched_id));
    untouched.read_with(cx, |v, _| {
        assert_eq!(
            v.tail_steps_for_test(),
            PaneChoice::Seeded(TailWindow::Last(5)),
            "a reloaded tail window must reach an untouched pane"
        );
        assert_eq!(
            v.display_filter_for_test(),
            PaneChoice::Seeded(DisplayFilter::default()),
            "the reload states no filter, so an untouched pane stays unfiltered"
        );
        assert_eq!(
            v.fold_mode_for_test().value(),
            FoldPreset::Summary.mode(),
            "a reloaded fold mode must reach an untouched pane"
        );
        assert_eq!(
            v.pane_choices().fold_mode,
            None,
            "and it is still only a seed"
        );
    });

    chosen.read_with(cx, |v, _| {
        assert_eq!(
            v.tail_steps_for_test(),
            PaneChoice::Chosen(TailWindow::Last(2)),
            "config must not overwrite a chosen tail window"
        );
        assert_eq!(
            v.tail_calls_for_test(),
            PaneChoice::Seeded(TailWindow::All),
            "the reload states no call level, so the level the user never picked \
             follows it to the built-in rather than inheriting the step window"
        );
        assert_eq!(
            v.display_filter_for_test(),
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose)),
            "config must not overwrite a chosen display filter"
        );
        assert_eq!(
            v.pane_choices().fold_mode,
            Some(FoldPreset::Expanded.mode()),
            "config must not overwrite a chosen fold mode"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A reload must resolve the defaults per pane, against that pane's own agent.
/// Two agents are what makes this observable: with one agent in the window,
/// resolving once for the whole window and resolving per pane land every pane
/// on the same values, so only disagreeing agents can tell the two apart.
#[gpui::test]
async fn a_config_reload_moves_each_pane_to_its_own_agents_transcript_settings(
    cx: &mut TestAppContext,
) {
    let before = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 3, "expanded", FilterFacet::Thinking),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 7, "summary", FilterFacet::Tools),
        ],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_per_agent_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (pane_a, pane_b) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                (
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx),
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_B, window, cx),
                )
            })
        })
        .unwrap();

    // Every axis of both agents changes, and the two stay disagreeing.
    let after = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 2, "summary", FilterFacet::Tools),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 6, "expanded", FilterFacet::Thinking),
        ],
        ..daruda_config::Config::default()
    };
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view_a = workspace.read_with(cx, |ws, _| agent_view(ws, pane_a));
    let view_b = workspace.read_with(cx, |ws, _| agent_view(ws, pane_b));
    let (tail_a, fold_a, filter_a) = transcript_settings(&view_a, cx);
    let (tail_b, fold_b, filter_b) = transcript_settings(&view_b, cx);

    assert_eq!(
        tail_a,
        PaneChoice::Seeded(TailWindow::Last(2)),
        "the first pane must land on its own agent's reloaded tail window"
    );
    assert_eq!(
        fold_a,
        FoldPreset::Summary.mode(),
        "the first pane must land on its own agent's reloaded fold mode"
    );
    assert_eq!(
        filter_a,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        "the first pane must land on its own agent's reloaded display filter"
    );

    assert_eq!(
        tail_b,
        PaneChoice::Seeded(TailWindow::Last(6)),
        "the second pane must land on its own agent's reloaded tail window"
    );
    assert_eq!(
        fold_b,
        FoldPreset::Expanded.mode(),
        "the second pane must land on its own agent's reloaded fold mode"
    );
    assert_eq!(
        filter_b,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Thinking.token()])),
        "the second pane must land on its own agent's reloaded display filter"
    );

    assert_ne!(
        tail_a, tail_b,
        "one resolve for the window would tie the two panes' tail windows together"
    );
    assert_ne!(
        fold_a, fold_b,
        "one resolve for the window would tie the two panes' fold modes together"
    );
    assert_ne!(
        filter_a, filter_b,
        "one resolve for the window would tie the two panes' display filters together"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A pane whose owning agent was removed falls back at reconnect time. That id
/// rewrite must move untouched transcript axes to the fallback agent's own
/// defaults, not leave them on the global defaults used while the id was stale.
#[gpui::test]
async fn stale_agent_reconnect_reseeds_to_the_fallback_agents_transcript_settings(
    cx: &mut TestAppContext,
) {
    let before = daruda_config::Config {
        agents: vec![transcript_agent_entry(
            TRANSCRIPT_AGENT_A,
            3,
            "expanded",
            FilterFacet::Thinking,
        )],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_stale_agent_reseed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx)
            })
        })
        .unwrap();

    let after = daruda_config::Config {
        agents: vec![transcript_agent_entry(
            TRANSCRIPT_AGENT_B,
            6,
            "summary",
            FilterFacet::Tools,
        )],
        ..daruda_config::Config::default()
    };
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.reload_config(&after, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    assert_eq!(
        transcript_settings(&view, cx),
        (
            PaneChoice::Seeded(TailWindow::All),
            FoldMode::default(),
            PaneChoice::Seeded(DisplayFilter::default()),
        ),
        "while the id is stale no entry states these axes, so reload can only \
         fall back to the built-in values"
    );

    workspace.update(cx, |ws, cx| {
        ws.resolve_pane_launch_for_test(pane_id, cx)
            .expect("the fallback agent is launchable");
    });

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    let agent_id = view.read_with(cx, |v, _| v.agent_id().to_owned());
    assert_eq!(agent_id, TRANSCRIPT_AGENT_B);
    // The status bar names the pane's auth domain from the wrapper's copy, so
    // it has to follow the view there too.
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_chat_identity(pane_id).map(|(id, _)| id),
            Some(TRANSCRIPT_AGENT_B)
        );
    });
    assert_eq!(
        transcript_settings(&view, cx),
        (
            PaneChoice::Seeded(TailWindow::Last(6)),
            FoldPreset::Summary.mode(),
            PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        ),
        "the reconnect fallback must reseed against the agent it now runs"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A new pane is born on its own agent's transcript defaults, not on whichever
/// agent the catalog lists first. As with the reload above, a single agent
/// cannot show the difference — both panes would be born identical either way.
#[gpui::test]
async fn a_new_pane_is_born_on_its_own_agents_transcript_settings(cx: &mut TestAppContext) {
    let config = daruda_config::Config {
        agents: vec![
            transcript_agent_entry(TRANSCRIPT_AGENT_A, 3, "expanded", FilterFacet::Thinking),
            transcript_agent_entry(TRANSCRIPT_AGENT_B, 7, "summary", FilterFacet::Tools),
        ],
        ..daruda_config::Config::default()
    };
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_per_agent_birth_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (pane_a, pane_b) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                (
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_A, window, cx),
                    open_agent_chat_pane_on(ws, TRANSCRIPT_AGENT_B, window, cx),
                )
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view_a = workspace.read_with(cx, |ws, _| agent_view(ws, pane_a));
    let view_b = workspace.read_with(cx, |ws, _| agent_view(ws, pane_b));
    let (tail_a, fold_a, filter_a) = transcript_settings(&view_a, cx);
    let (tail_b, fold_b, filter_b) = transcript_settings(&view_b, cx);

    assert_eq!(
        tail_a,
        PaneChoice::Seeded(TailWindow::Last(3)),
        "the first pane must be born on its own agent's tail window"
    );
    assert_eq!(
        fold_a,
        FoldPreset::Expanded.mode(),
        "the first pane must be born on its own agent's fold mode"
    );
    assert_eq!(
        filter_a,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Thinking.token()])),
        "the first pane must be born on its own agent's display filter"
    );

    assert_eq!(
        tail_b,
        PaneChoice::Seeded(TailWindow::Last(7)),
        "the second pane must be born on its own agent's tail window"
    );
    assert_eq!(
        fold_b,
        FoldPreset::Summary.mode(),
        "the second pane must be born on its own agent's fold mode"
    );
    assert_eq!(
        filter_b,
        PaneChoice::Seeded(DisplayFilter::from_tokens([FilterFacet::Tools.token()])),
        "the second pane must be born on its own agent's display filter"
    );

    assert_ne!(
        tail_a, tail_b,
        "resolving against one catalog entry would tie the two panes' tail windows together"
    );
    assert_ne!(
        fold_a, fold_b,
        "resolving against one catalog entry would tie the two panes' fold modes together"
    );
    assert_ne!(
        filter_a, filter_b,
        "resolving against one catalog entry would tie the two panes' display filters together"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
async fn connect_and_later_events_replace_only_their_vocabulary_axis(cx: &mut TestAppContext) {
    use daruda_acp::{AcpEvent, ModeStateView, SessionCapabilitiesView, SessionModeView};
    use daruda_store::agent_vocabulary::{AgentVocabularyCache, VocabEntry};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let agent = daruda_config::AgentDefinition::claude_default();
    let source = crate::lane::session_host::adapter_command(&agent.launch)
        .trim()
        .to_string();
    let agent_id = agent.id;
    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    agent_id.clone(),
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    // What the adapter advertises at connect: both axes at once.
    let connected = AcpEvent::Connected {
        program: None,
        session_id: "s1".into(),
        modes: Some(ModeStateView {
            available: vec![
                SessionModeView {
                    id: "default".into(),
                    name: "Default".into(),
                    description: None,
                },
                SessionModeView {
                    id: "plan".into(),
                    name: "Plan".into(),
                    description: None,
                },
            ],
            current: "default".into(),
        }),
        config_options: model_options(&[("opus", "Opus"), ("sonnet", "Sonnet")]),
        capabilities: SessionCapabilitiesView::default(),
        login_methods: Vec::new(),
    };

    let data_dir = workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &connected, cx);
        ws.data_dir.clone()
    });

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("default", "Default"),
                    VocabEntry::new("plan", "Plan"),
                ]
                .as_slice()
            )
        );
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("opus", "Opus"),
                    VocabEntry::new("sonnet", "Sonnet"),
                ]
                .as_slice()
            ),
            "only the Model-category select feeds the model axis"
        );
    });
    assert_eq!(
        AgentVocabularyCache::load_in(&data_dir).known_models_for(&agent_id, &source),
        Some(
            vec![
                VocabEntry::new("opus", "Opus"),
                VocabEntry::new("sonnet", "Sonnet"),
            ]
            .as_slice()
        ),
        "a changed vocabulary is persisted, not just cached in memory"
    );

    // A later option replacement must leave the mode axis alone.
    let changed = AcpEvent::ConfigOptionsChanged(model_options(&[("haiku", "Haiku")]));
    workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &changed, cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(vec![VocabEntry::new("haiku", "Haiku")].as_slice())
        );
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(
                vec![
                    VocabEntry::new("default", "Default"),
                    VocabEntry::new("plan", "Plan"),
                ]
                .as_slice()
            ),
            "ConfigOptionsChanged replaces options only — modes are untouched"
        );
    });

    // A model switch may rebuild the advertised modes. The reconciled
    // ModeChanged state replaces that axis without disturbing models.
    let mode_changed = AcpEvent::ModeChanged {
        state: ModeStateView {
            available: vec![SessionModeView {
                id: "review".into(),
                name: "Review".into(),
                description: None,
            }],
            current: "review".into(),
        },
    };
    workspace.update(cx, |ws, cx| {
        ws.record_agent_vocabulary(pane_id, &mode_changed, cx);
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.agent_vocabulary.known_modes_for(&agent_id, &source),
            Some(vec![VocabEntry::new("review", "Review")].as_slice())
        );
        assert_eq!(
            ws.agent_vocabulary.known_models_for(&agent_id, &source),
            Some(vec![VocabEntry::new("haiku", "Haiku")].as_slice()),
            "ModeChanged replaces modes only — models are untouched"
        );
    });
    assert_eq!(
        AgentVocabularyCache::load_in(&data_dir).known_modes_for(&agent_id, &source),
        Some(vec![VocabEntry::new("review", "Review")].as_slice()),
        "the changed mode vocabulary is persisted"
    );
}

/// A `default_mode` / `default_model` edit reaches a session that is already
/// running, for every axis its user has not picked — and stays a default,
/// never becoming the pane's own pick.
#[gpui::test]
async fn a_config_edit_moves_live_sessions_off_unpicked_defaults(cx: &mut TestAppContext) {
    use daruda_acp::{ModeStateView, SessionModeView};

    let config_with = |model: &str, mode: &str| daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(
            daruda_config::AgentDefinition {
                default_model: Some(model.to_string()),
                default_mode: Some(mode.to_string()),
                ..daruda_config::AgentDefinition::claude_default()
            },
        )],
        ..daruda_config::Config::default()
    };
    let agent_id = daruda_config::AgentDefinition::claude_default().id;
    let (window_handle, workspace) =
        super::super::build_workspace_with(cx, &config_with("sonnet", "default"), None);
    cx.run_until_parked();

    // `untouched` follows the agent on both axes; `picked` chose its mode.
    let (untouched, picked, mut probes) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let untouched = open_agent_chat_pane_on(ws, &agent_id, window, cx);
                let picked = open_agent_chat_pane_on(ws, &agent_id, window, cx);
                let mut probes = Vec::new();
                for id in [untouched, picked] {
                    ws.fold_agent_chat_event(
                        id,
                        daruda_acp::AcpEvent::Connected {
                            program: None,
                            session_id: format!("s{id}"),
                            modes: Some(ModeStateView {
                                available: ["default", "plan", "bypassPermissions"]
                                    .into_iter()
                                    .map(|id| SessionModeView {
                                        id: id.into(),
                                        name: id.into(),
                                        description: None,
                                    })
                                    .collect(),
                                current: "default".into(),
                            }),
                            config_options: model_options(&[
                                ("sonnet", "Sonnet"),
                                ("opus", "Opus"),
                            ]),
                            capabilities: daruda_acp::SessionCapabilitiesView::default(),
                            login_methods: Vec::new(),
                        },
                        cx,
                    );
                    let (handle, probe) = daruda_acp::AcpSessionHandle::detached_for_test();
                    probes.push(probe);
                    agent_view(ws, id).update(cx, |v, _| v.attach_handle(handle));
                }
                agent_view(ws, picked).update(cx, |v, cx| v.set_mode("plan".into(), cx));
                for probe in &mut probes {
                    probe.drain();
                }
                (untouched, picked, probes)
            })
        })
        .unwrap();

    workspace.update(cx, |ws, cx| {
        ws.reload_config(&config_with("opus", "bypassPermissions"), cx)
    });
    cx.run_until_parked();

    let state = |id: PaneId, cx: &mut TestAppContext| {
        workspace.read_with(cx, |ws, cx| {
            let v = agent_view(ws, id).read(cx);
            let options = v.session_config().config_options_for_chips(&[]);
            let model = daruda_chat::session_config::model_select(&options)
                .map(|(_, current, _)| current.to_owned());
            (
                model,
                v.session_config()
                    .mode_for_chip()
                    .map(|m| m.current.clone()),
                v.picked_model_id().map(str::to_owned),
                v.pane_choices().mode_id,
            )
        })
    };
    assert_eq!(
        state(untouched, cx),
        (
            Some("opus".to_string()),
            Some("bypassPermissions".to_string()),
            None,
            None
        ),
        "an untouched pane follows both edited defaults without picking them"
    );
    assert_eq!(probes[0].drain().0, 2, "one request per moved axis");
    assert_eq!(
        state(picked, cx),
        (
            Some("opus".to_string()),
            Some("plan".to_string()),
            None,
            Some("plan".to_string())
        ),
        "a picked mode stands; the unpicked model still follows"
    );
    assert_eq!(probes[1].drain().0, 1, "only the model is requested");
}
