//! What a pane saves and what a restore brings back: its agent, session, title,
//! its own transcript choices and model pick.

use super::*;

/// Persist AgentChat panes with session identity and a non-default owner, then
/// restore both with and without that owner still in the catalog. A surviving
/// owner keeps its session id; a removed owner falls back to the default agent
/// and drops the id. Panes sit in non-active tabs so restore focus never
/// triggers a connect.
#[gpui::test]
async fn agent_chat_agent_id_restore_handles_present_and_removed_owner(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    // Per-process root: a fixed path makes the restore assertions depend on
    // whatever else in the run happened to write here first.
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_restore_agent_present_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));

                // A default-agent pane with persisted session id + title. This
                // proves the snapshot/restore round-trip preserves resume
                // identity and the label shown before a session load completes.
                let titled = ws.create_agent_chat_pane(
                    cwd.clone(),
                    Some("sess-restore-123".to_string()),
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let titled_id = titled.id;
                ws.active_runtime_mut().panes.push(titled);
                let titled_tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: titled_tab_id,
                    layout: PaneLayout::Pane(titled_id),
                    last_focused_pane: titled_id,
                    user_label: None,
                });
                let view = ws
                    .agent_chat_view(titled_id)
                    .cloned()
                    .expect("agent chat view present");
                view.update(cx, |v, cx| {
                    v.set_session_title_for_test("Investigate flaky test");
                    v.restore_pane_choices(ChatPaneChoices {
                        // Not the configured width, so only a saved choice can
                        // bring it back.
                        content_width: Some(ChatContentWidth::Full),
                        // Different windows per level, so a save that collapsed
                        // the axis back to one value would restore the wrong one.
                        tail_steps: Some(TailWindow::Last(3)),
                        tail_calls: Some(TailWindow::Last(10)),
                        // Distinct token vocabularies: each preference's tokens
                        // are unreadable to the others, so a crossed wire
                        // restores a default rather than the wrong-but-plausible
                        // value.
                        display_filter: Some(DisplayFilter::default().toggled(FilterFacet::Prose)),
                        fold_mode: Some(FoldPreset::Summary.mode()),
                        ..ChatPaneChoices::default()
                    });
                    cx.notify();
                });

                // Pane owned by `codex` (a non-default agent), with a session id
                // stamped as a connected session would.
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    Some("sess-codex-1".to_string()),
                    codex_agent().id,
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
                ws.snapshot_for_disk(cx)
            })
        })
        .unwrap();

    // Restore into a fresh workspace whose catalog still includes `codex`.
    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.mirrors.agents = vec![
            daruda_config::AgentDefinition::claude_default(),
            codex_agent(),
        ]
        .into();
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, cx| {
        let views = restored_chats(ws, cx);
        let titled = views
            .iter()
            .find(|v| v.session_id.as_deref() == Some("sess-restore-123"))
            .expect("restored titled agent chat pane present");
        assert_eq!(
            titled.title.as_deref(),
            Some("Investigate flaky test"),
            "persisted title must round-trip (seeds the tab label before load)"
        );
        assert!(
            !titled.replaying,
            "a restored dormant pane is not yet loading"
        );
        assert!(titled.dormant, "no live session until first focus connects");
        assert_eq!(
            titled.content_width,
            Some(ChatContentWidth::Full),
            "a pane's chosen width must round-trip"
        );
        assert_eq!(
            titled.tail_steps,
            PaneChoice::Chosen(TailWindow::Last(3)),
            "per-pane tail window must round-trip as the user's own choice"
        );
        assert_eq!(
            titled.tail_calls,
            PaneChoice::Chosen(TailWindow::Last(10)),
            "and each level keeps its own, not the other's"
        );
        assert_eq!(
            titled.display_filter,
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose)),
            "per-pane display filter must round-trip as the user's own choice"
        );
        assert_eq!(
            titled.fold_mode,
            Some(FoldPreset::Summary.mode()),
            "per-pane fold mode must round-trip as the user's own choice"
        );

        let codex = views
            .iter()
            .find(|v| v.agent_id == "codex")
            .expect("restored codex-owned pane present");
        assert_eq!(
            codex.session_id.as_deref(),
            Some("sess-codex-1"),
            "session id is kept when the owning agent is present (resume valid)"
        );
    });

    // Restore into a fresh workspace whose catalog is the default (claude only) —
    // `codex` was removed. `Workspace::new_with_project_for_test` seeds `agents`
    // from `config`, so no override is needed.
    let fallback_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let fallback = fallback_handle.root(cx).unwrap();

    fallback.read_with(cx, |ws, cx| {
        let default_id = daruda_config::AgentDefinition::claude_default().id;
        let views = restored_chats(ws, cx);
        assert!(
            views.iter().all(|v| v.agent_id == default_id),
            "every restored agent-chat pane falls back to or keeps the default agent"
        );
        let kept = views
            .iter()
            .find(|v| v.session_id.as_deref() == Some("sess-restore-123"))
            .expect("the default-agent session keeps its persisted id");
        assert_eq!(kept.title.as_deref(), Some("Investigate flaky test"));
        assert_eq!(kept.content_width, Some(ChatContentWidth::Full));
        assert_eq!(kept.tail_steps, PaneChoice::Chosen(TailWindow::Last(3)));
        assert_eq!(kept.tail_calls, PaneChoice::Chosen(TailWindow::Last(10)));
        assert_eq!(
            kept.display_filter,
            PaneChoice::Chosen(DisplayFilter::default().toggled(FilterFacet::Prose))
        );
        assert_eq!(kept.fold_mode, Some(FoldPreset::Summary.mode()));
        assert!(
            views
                .iter()
                .any(|v| v.session_id.is_none() && v.title.is_none()),
            "session id is dropped when its owning agent is absent (resume invalid)"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
async fn an_untouched_pane_keeps_following_the_config_defaults(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let before = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_untouched_defaults_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &before, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let pane = ws.create_agent_chat_pane(
                    cwd,
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
                ws.snapshot_for_disk(cx)
            })
        })
        .unwrap();

    let leaf_json = serde_json::to_string(&project_states).expect("serialize project states");
    for key in ["tail_window", "display_filter", "fold_mode"] {
        assert!(
            !leaf_json.contains(key),
            "{key} must stay out of an untouched pane's state: {leaf_json}"
        );
    }

    let after = daruda_config::Config {
        agents: vec![claude_entry_with(5, "summary")],
        ..daruda_config::Config::default()
    };

    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &after,
            None,
            super::super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .expect("restored agent chat pane present")
            .read(cx);
        assert_eq!(
            view.tail_steps_for_test(),
            PaneChoice::Seeded(TailWindow::Last(5)),
            "the new config tail window must reach an untouched pane"
        );
        assert_eq!(
            view.display_filter_for_test(),
            PaneChoice::Seeded(DisplayFilter::default()),
            "a restored pane opens unfiltered — no config key narrows it"
        );
        assert_eq!(
            view.fold_mode_for_test().value(),
            FoldPreset::Summary.mode(),
            "the new config fold mode must reach an untouched pane"
        );
        assert_eq!(
            view.pane_choices().fold_mode,
            None,
            "and it is still the seed, not a choice"
        );
        assert_ne!(FoldPreset::Summary.mode(), FoldMode::default());
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
async fn a_model_pick_is_remembered_and_survives_a_restore(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join(format!(
        "daruda_agent_chat_model_restore_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    Some("sess-model-1".to_string()),
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
                // Advertise the option set the pick lands on, through the same
                // host fold a live session's event takes.
                let view = agent_view(ws, pane_id);
                ws.fold_agent_chat_event(
                    pane_id,
                    daruda_acp::AcpEvent::ConfigOptionsChanged(model_options(&[
                        ("opus", "Opus"),
                        ("sonnet", "Sonnet"),
                    ])),
                    cx,
                );

                // A non-model option must not be mistaken for the model axis.
                ws.set_agent_config_option(
                    pane_id,
                    "effort".to_string(),
                    daruda_acp::ConfigValueView::Id("high".to_string()),
                    cx,
                );
                assert_eq!(view.read(cx).picked_model_id(), None);

                ws.set_agent_config_option(
                    pane_id,
                    "model".to_string(),
                    daruda_acp::ConfigValueView::Id("sonnet".to_string()),
                    cx,
                );
                assert_eq!(
                    view.read(cx).picked_model_id(),
                    Some("sonnet"),
                    "the chip pick is remembered so the next connect reapplies it"
                );

                ws.snapshot_for_disk(cx)
            })
        })
        .unwrap();

    let persisted = project_states
        .iter()
        .flat_map(|p| p.lanes.iter())
        .flat_map(|l| l.tabs.iter())
        .find_map(|t| match &t.layout {
            daruda_store::project::SerializedLayout::Leaf {
                content: daruda_store::project::SerializedPaneContent::AgentChat(ac),
                ..
            } => ac.model_id.clone(),
            _ => None,
        });
    assert_eq!(
        persisted.as_deref(),
        Some("sonnet"),
        "the pick reaches the persisted pane record"
    );

    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();
    restored.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .expect("restored agent chat pane present")
            .read(cx);
        assert_eq!(
            view.picked_model_id(),
            Some("sonnet"),
            "a restored pane still knows its model, so its lazy connect reapplies it"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}
