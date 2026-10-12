//! Opening a pane, switching or splitting its agent, its links, and the
//! activity it reports to the lane.

use super::*;

/// A pane's diff embeds are fingerprinted against the Workspace-resolved syntax
/// palette, and a fold expand or a seeded transcript can ask for one before any
/// ACP event has arrived. Filling the field from the first event instead left an
/// event-less pane rendering every diff through the inline fallback for the rest
/// of its life, so the seed belongs at construction.
#[gpui::test]
async fn a_new_pane_is_born_with_the_resolved_syntax_theme(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let pane_id = ws.active_runtime().panes.last().expect("pane opened").id;
        let view = agent_view(ws, pane_id);
        assert_eq!(
            view.read(cx).syntax_theme(),
            ws.mirrors.syntax_theme,
            "the pane starts on the Workspace's resolved palette, not on nothing"
        );
    });
}

#[gpui::test]
async fn open_agent_chat_pane_creates_agent_chat_leaf(cx: &mut TestAppContext) {
    use crate::surface::strings as s;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before + 1,
            "opening an agent chat pane appends a tab"
        );
        let pane = ws
            .active_runtime()
            .panes
            .last()
            .expect("open_agent_chat_pane pushed a pane");
        match &pane.content {
            PaneContent::AgentChat(ac) => {
                let view = ac.view.read(cx);
                // No resolvable lane cwd → the pane parks in `Error` rather
                // than attempting a connection, keeping the suite offline.
                let AgentSessionStatus::Error { message, .. } = &view.status() else {
                    panic!(
                        "no lane cwd → error status, not a live connect, got {:?}",
                        view.status()
                    );
                };
                assert_eq!(message.as_str(), s::agent_chat::no_lane_cwd());
                assert!(view.items().is_empty(), "items start empty");
                assert!(view.any_handle().is_none(), "no session without a cwd");
            }
            _ => panic!("expected an AgentChat pane"),
        }
        assert_eq!(ws.active_runtime().focused_pane_id, pane.id);
    });

    let with_cwd = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let remote = ws.create_agent_chat_pane(
                    Some(PaneCwd::Remote("host:/repo/lane".to_string())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                assert_eq!(
                    remote.cwd(),
                    None,
                    "PaneCwd::Remote must not surface through Pane::cwd()"
                );

                let tmp = std::env::temp_dir();
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                assert_eq!(pane.cwd(), Some(tmp.as_path()));
                match &pane.content {
                    PaneContent::AgentChat(ac) => ac.view.read(cx).status().clone(),
                    _ => panic!("expected an AgentChat pane"),
                }
            })
        })
        .unwrap();

    assert_eq!(
        with_cwd,
        AgentSessionStatus::Idle,
        "a pane with a cwd must stay dormant until first focus, got {with_cwd:?}"
    );
}

/// Core perf contract: a `cx.notify()` on the `AgentChatView` must
/// re-render the (cached) view — the mechanism that lets an async event (a
/// streamed chunk, a landed mermaid image) repaint the conversation without the
/// whole window re-rendering. Guards against a future change that breaks the
/// cached-view notify path (the lost-wakeup class).
#[gpui::test]
async fn notify_rerenders_cached_agent_view(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .last()
            .and_then(|p| p.agent_chat_view())
            .cloned()
            .expect("agent chat pane present")
    });
    let before = view.read_with(cx, |v, _| v.render_count_for_test());
    assert!(
        before >= 1,
        "the view should have rendered at least once after open, got {before}"
    );

    // Simulate the async mermaid-completion / re-parse notify.
    cx.update(|cx| view.update(cx, |_v, cx| cx.notify()));
    cx.run_until_parked();

    let after = view.read_with(cx, |v, _| v.render_count_for_test());
    assert!(
        after > before,
        "cx.notify() must re-render the cached view: before={before} after={after}"
    );
}

/// Regression: the agent status indicator disappears after a lane switch. A
/// parked lane's AgentChat status must still reach the rendered left-dock
/// snapshot. Drives a real `activate_lane` switch and asserts on the real
/// `prepare_left_dock_snapshot`'s `agent_status_per_lane` map (the badge
/// source), exercising the production render path.
#[gpui::test]
async fn parked_lane_agent_status_reaches_left_dock_aggregate(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Both roots must exist so activation classifies them `Present`.
    let root_a = std::path::PathBuf::from("/tmp/test_acp_indicator_a");
    let root_b = std::path::PathBuf::from("/tmp/test_acp_indicator_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            super::super::fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let workspace = window_handle.root(cx).unwrap();
    cx.run_until_parked();

    let lane0 = workspace.read_with(cx, |ws, _| ws.active);
    // Seed a second lane to switch into.
    workspace.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });
    let lane1 = daruda_store::project::LaneRef {
        project: lane0.project,
        lane: 1,
    };

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // An agent chat pane in lane 0 (active) with a turn in flight →
            // `to_session_status()` is `Working`. Wire it into a tab layout
            // (pane + TabEntry) so it appears in `pane_lane_index`; do it
            // directly rather than via the open action to skip `focus_pane`'s
            // lazy `maybe_connect`, which would spawn a real adapter.
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(root_a.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let id = pane.id;
            let tab_id = ws.alloc_id();
            ws.active_runtime_mut().panes.push(pane);
            ws.active_runtime_mut()
                .tabs
                .push(crate::workspace::main_area::pane::TabEntry {
                    id: tab_id,
                    layout: crate::workspace::main_area::pane_tree::PaneLayout::Pane(id),
                    last_focused_pane: id,
                    user_label: None,
                });
            agent_view(ws, id).update(cx, |v, _| {
                v.set_status_for_test(AgentSessionStatus::Connected);
                v.set_turn_in_flight();
            });

            // Switch away → lane 0 is now parked, lane 1 active.
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1);
            assert!(
                ws.agent_chat_view(id).is_some(),
                "agent_chat_view must find a pane parked in an inactive lane"
            );

            // Build the actual left-dock snapshot the renderer consumes and
            // assert the parked lane's badge source is `Working`.
            let snap = ws.prepare_left_dock_snapshot(cx);
            assert_eq!(
                snap.agent_status_per_lane.get(&lane0),
                Some(&daruda_agent::SessionStatus::Working),
                "a parked lane's agent Working status must appear in the rendered \
                 left-dock snapshot keyed by its LaneRef"
            );
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// The inaccessible-lane guard in `open_agent_chat_pane_with_agent` fires
/// *before* it records the agent choice: a rejected open adds no pane and leaves
/// `last_agent_id` untouched (so a stale/inaccessible state can't poison the
/// session default).
#[gpui::test]
async fn open_with_agent_guards_before_recording_last_id(cx: &mut TestAppContext) {
    use crate::lane::availability::LaneAvailability;

    let config = daruda_config::Config::default();
    let root = std::env::temp_dir().join("daruda_agent_open_guard");
    let _ = std::fs::create_dir_all(&root);
    let project = daruda_store::project::Project::from_path(&root);
    let (window_handle, workspace) = super::super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // Force the active lane inaccessible → the empty-state; the open must
            // be rejected before it mutates `last_agent_id`.
            ws.active_lane_mut().expect("active lane").availability = LaneAvailability::Missing;
            assert!(ws.active_lane_is_inaccessible());
            assert_eq!(ws.last_agent_id, None, "no prior choice recorded");
            let panes_before = ws.active_runtime().panes.len();

            ws.open_agent_chat_pane_with_agent("codex".to_string(), window, cx);

            assert_eq!(
                ws.active_runtime().panes.len(),
                panes_before,
                "an inaccessible lane rejects the open — no pane added"
            );
            assert_eq!(
                ws.last_agent_id, None,
                "the guard fires before recording, so last_agent_id stays untouched"
            );
        });
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// Switching agents opens a *new* pane under the target agent and leaves the
/// source pane + its (offline) session untouched — the backend swap is a fresh
/// conversation, never a reuse of the existing session. Splitting that focused
/// pane then inherits the source agent, rather than resetting to the catalog
/// default.
#[gpui::test]
async fn switch_agent_preserves_source_and_split_inherits_agent(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_tree::SplitDirection;
    use crate::workspace::main_area::tab_ops::NewPaneKind;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // Two-entry catalog: claude (default) + codex (switch target).
            ws.mirrors.agents = vec![
                daruda_config::AgentDefinition::claude_default(),
                codex_agent(),
            ]
            .into();
            let claude_id = daruda_config::AgentDefinition::claude_default().id;

            // The original agent-chat pane, chatting under the default agent.
            let src = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                claude_id.clone(),
                None,
                window,
                cx,
            );
            let src_id = src.id;
            ws.active_runtime_mut().panes.push(src);
            let panes_before = ws.active_runtime().panes.len();

            // Switch to codex — opens a fresh pane (no project → cwd None → the
            // new pane parks in `Error`, so its lazy connect is skipped: offline).
            ws.open_agent_chat_pane_with_agent("codex".to_string(), window, cx);

            assert_eq!(
                ws.active_runtime().panes.len(),
                panes_before + 1,
                "the switch appends a new pane, it does not replace the source"
            );

            // The source pane and its session survive untouched.
            let src_view = agent_view(ws, src_id);
            let src_view = src_view.read(cx);
            assert_eq!(
                src_view.agent_id(),
                claude_id,
                "the source pane keeps chatting under its own agent"
            );
            assert!(
                src_view.any_handle().is_none(),
                "the source session is untouched"
            );

            // The new pane runs under the target agent.
            let new_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the switch pushed a pane")
                .id;
            assert_ne!(new_id, src_id, "a distinct pane was opened");
            assert_eq!(
                agent_view(ws, new_id).read(cx).agent_id(),
                "codex",
                "the new pane runs under the switched-to agent"
            );

            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );

            let split_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the split pushed a pane")
                .id;
            assert_ne!(split_id, new_id, "the split created a distinct pane");
            assert_eq!(
                agent_view(ws, split_id).read(cx).agent_id(),
                "codex",
                "the split inherits the source pane's agent, not the catalog default"
            );
        });
    })
    .unwrap();
}

/// A diff block's "open in file view" / "open externally" actions must
/// refuse a pane whose session is remote (`PaneCwd::Remote`) rather than
/// reading `Diff.path` — a remote-host path — off *this* machine's local
/// disk, which would either fail or silently show an unrelated local file at
/// the same absolute path.
#[gpui::test]
async fn diff_actions_on_a_remote_pane_report_an_error_instead_of_reading_local_disk(
    cx: &mut TestAppContext,
) {
    use crate::surface::strings as s;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Remote("host:/repo/lane".to_string())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                pane_id
            })
        })
        .unwrap();

    let errors_before = workspace.read_with(cx, |ws, _| ws.error_history().len());
    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_diff_in_file_view(
                pane_id,
                std::path::PathBuf::from("/repo/lane/src/main.rs"),
                window,
                cx,
            );
            ws.open_pane_file_externally(
                pane_id,
                std::path::PathBuf::from("/repo/lane/src/main.rs"),
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            tabs_before,
            "must not open a file-viewer tab for a remote pane's path"
        );
        assert_eq!(
            ws.error_history().len(),
            errors_before + 2,
            "both actions must report an error, one each"
        );
        for report in &ws.error_history()[..2] {
            assert_eq!(report.title, s::agent_chat::diff_remote_path_unsupported());
        }
    });
}

/// A remote session's link never reaches this machine's disk: the click
/// reports, and the menu gets no opener — even for a path (`/tmp`) that
/// happens to exist here, and even spelled as `file://`.
#[gpui::test]
async fn a_remote_panes_links_report_instead_of_opening_a_local_file(cx: &mut TestAppContext) {
    use crate::surface::strings as s;
    use daruda_content::link_target::LinkTarget;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let pane_id = push_agent_chat_pane(
        cx,
        window_handle,
        &workspace,
        PaneCwd::Remote("host:/repo/lane".to_string()),
    );
    let errors_before = workspace.read_with(cx, |ws, _| ws.error_history().len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            assert_eq!(
                ws.classify_pane_link(pane_id, "/tmp", cx),
                LinkTarget::Remote
            );
            assert!(ws.open_pane_link(pane_id, "/tmp", window, cx));
            assert!(ws.open_pane_link(pane_id, "file:///tmp", window, cx));
            let target = ws.classify_pane_resource(pane_id, "/tmp/shot.png", None, cx);
            assert!(ws.open_link_target(pane_id, target, window, cx));
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.error_history().len(), errors_before + 3);
        for report in &ws.error_history()[..3] {
            assert_eq!(report.title, s::agent_chat::diff_remote_path_unsupported());
        }
    });
}

/// A resource card's pane menu resolves the link as its left click does: by
/// resource rules and the declared type. Markdown rules would call the
/// extensionless image a text file, and a missing relative file a plain word.
#[gpui::test]
async fn a_resource_links_menu_resolves_it_as_its_click_does(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_menu::ResourceRightClick;
    use daruda_content::link_target::LocalKind;
    use gpui::{Point, px};

    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("artifact"), b"x").expect("write artifact");
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let pane_id = push_agent_chat_pane(
        cx,
        window_handle,
        &workspace,
        PaneCwd::Local(dir.path().to_path_buf()),
    );
    let at = Point::new(px(10.), px(20.));

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let mut menu_file = |uri: &str, mime: Option<&str>, position| {
                ws.record_resource_right_click(ResourceRightClick {
                    position: at,
                    uri: uri.to_string(),
                    mime: mime.map(str::to_string),
                });
                ws.pane_menu_file_for_test(pane_id, position, window, cx)
            };
            assert_eq!(
                menu_file("artifact", Some("image/png"), at),
                Some((dir.path().join("artifact"), LocalKind::Image))
            );
            assert_eq!(
                menu_file("gone.png", None, at),
                Some((dir.path().join("gone.png"), LocalKind::Missing))
            );
            // Another press's record is not this menu's.
            assert_eq!(
                menu_file("artifact", None, Point::new(px(0.), px(0.))),
                None
            );
        });
    })
    .unwrap();
}

/// A resource link whose file is gone reports, where it used to do nothing —
/// a relative URI included, which Markdown rules would read as a plain word.
#[gpui::test]
async fn a_missing_resource_link_reports_instead_of_doing_nothing(cx: &mut TestAppContext) {
    use crate::surface::strings as s;

    let dir = tempfile::tempdir().expect("temp dir");
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let pane_id = push_agent_chat_pane(
        cx,
        window_handle,
        &workspace,
        PaneCwd::Local(dir.path().to_path_buf()),
    );
    let errors_before = workspace.read_with(cx, |ws, _| ws.error_history().len());
    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let target = ws.classify_pane_resource(pane_id, "gone.png", None, cx);
            assert!(ws.open_link_target(pane_id, target, window, cx));
            let absolute = dir.path().join("gone.rs");
            assert!(ws.open_pane_link(pane_id, absolute.to_str().unwrap(), window, cx));
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), tabs_before, "no viewer tab");
        assert_eq!(ws.error_history().len(), errors_before + 2);
        for report in &ws.error_history()[..2] {
            assert_eq!(report.title, s::agent_chat::link_file_missing());
        }
    });
}

/// The workspace pulse is what reaches a pane whose run settled on the clock
/// alone: a trailing subagent stays busy until its quiescence window lapses,
/// and no event says so. The pulse must still pick that pane up and tick it,
/// or the working row outlives the run. (What the tick itself does is the
/// chat's own test.)
#[gpui::test]
async fn the_pulse_reaches_a_pane_that_settles_on_the_clock(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tool = |id: &str, parent: Option<&str>| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_string(),
            title: id.to_string(),
            kind: ToolKindView::Read,
            tool_name: None,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: parent.map(str::to_string),
            exit: None,
        })
    };

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(std::env::temp_dir())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            let view = agent_view(ws, id);
            view.update(cx, |v, cx| {
                v.set_subagent_activity_for_test("task-1", std::time::Instant::now());
                v.seed_items_for_test([tool("task-1", None), tool("child-1", Some("task-1"))], cx);
            });
            ws.pulse_agent_chats(cx);
            let working = view.read(cx).visible_row_count_for_test();
            assert!(
                view.read(cx).last_reconciled_busy(),
                "the live subagent keeps it busy"
            );

            view.update(cx, |v, _| {
                let lapsed = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(20))
                    .expect("the monotonic clock has 20s of history");
                v.set_subagent_activity_for_test("task-1", lapsed);
            });
            ws.pulse_agent_chats(cx);

            assert!(
                !view.read(cx).last_reconciled_busy(),
                "the pulse ticked the pane that settled with no event"
            );
            assert_eq!(
                view.read(cx).visible_row_count_for_test(),
                working - 1,
                "and the working row went with the run"
            );
        });
    })
    .expect("window is live");
}
