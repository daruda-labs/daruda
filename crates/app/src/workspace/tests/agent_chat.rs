//! AgentChat pane — open action and the pure parts of the prompt / permission
//! / mode ops (no live ACP session required). Tests build the view via
//! `create_agent_chat_pane`, which opens no connection, so `handle` stays
//! `None` while host-side state transitions still run.

use daruda_store::project::PaneCwd;
use gpui::{AppContext as _, Entity, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::{AgentChatView, AgentSessionStatus};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Fetch the `AgentChatView` entity for `pane_id` (panics if the pane is gone
/// or is not an AgentChat pane).
pub(super) fn agent_view(ws: &Workspace, pane_id: PaneId) -> Entity<AgentChatView> {
    ws.active_runtime()
        .panes
        .iter()
        .find(|p| p.id == pane_id)
        .and_then(|p| p.agent_chat_view())
        .cloned()
        .expect("agent chat pane present")
}

/// The queued-prompt texts in FIFO order — the queue holds `QueuedPrompt`
/// (id + text), so tests compare on the text projection.
fn queue_texts(v: &AgentChatView) -> Vec<String> {
    v.queue
        .pending_prompts
        .iter()
        .map(|q| q.text.clone())
        .collect()
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
                let AgentSessionStatus::Error(message) = &view.status else {
                    panic!(
                        "no lane cwd → error status, not a live connect, got {:?}",
                        view.status
                    );
                };
                assert_eq!(message.as_str(), s::agent_chat_no_lane_cwd());
                assert_ne!(
                    message.as_str(),
                    s::agent_chat_error_prefix(),
                    "payload must be the reason, not the prefix the banner re-adds"
                );
                assert!(view.items.is_empty(), "items start empty");
                assert!(view.handle.is_none(), "no session without a cwd");
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
                    PaneContent::AgentChat(ac) => ac.view.read(cx).status.clone(),
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
    let before = view.read_with(cx, |v, _| v.render_count.get());
    assert!(
        before >= 1,
        "the view should have rendered at least once after open, got {before}"
    );

    // Simulate the async mermaid-completion / re-parse notify.
    cx.update(|cx| view.update(cx, |_v, cx| cx.notify()));
    cx.run_until_parked();

    let after = view.read_with(cx, |v, _| v.render_count.get());
    assert!(
        after > before,
        "cx.notify() must re-render the cached view: before={before} after={after}"
    );
}

#[gpui::test]
async fn cancel_turn_ends_the_turn_locally_without_an_agent_reply(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                // A turn mid-flight: streaming text + a still-running tool call,
                // exactly the state a hung agent leaves (no stop reason ever
                // arrives, so `TurnEnded` would never clear this).
                view.update(cx, |v, _| {
                    v.set_turn_in_flight();
                    v.items = vec![
                        ChatItem::AssistantText {
                            text: "working".into(),
                            streaming: true,
                            message_id: None,
                        },
                        ChatItem::ToolCall(ToolCallItem {
                            id: "t1".into(),
                            title: "Read".into(),
                            kind: ToolKindView::Read,
                            tool_name: None,
                            status: ToolStatusView::InProgress,
                            diffs: Vec::new(),
                            output: Vec::new(),
                            raw_input: None,
                            parent_tool_id: None,
                            exit: None,
                        }),
                    ];
                });
                // Offline (no handle): `session/cancel` is a no-op, so only the
                // authoritative local teardown can end the turn.
                ws.cancel_agent_turn(id, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(
            view.turn_is_idle(),
            "Stop ends the turn without an agent reply"
        );
        assert!(view.turn_is_idle(), "the turn is settled to idle");
        let ChatItem::AssistantText { streaming, .. } = &view.items[0] else {
            panic!("expected the streamed assistant text");
        };
        assert!(!streaming, "streaming text settles on Stop");
        let ChatItem::ToolCall(tc) = &view.items[1] else {
            panic!("expected the tool call");
        };
        assert_eq!(
            tc.status,
            ToolStatusView::Cancelled,
            "the in-progress tool call is marked cancelled (stops the rollup pulse)"
        );
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            // Not busy yet → the Escape shim is a no-op and reports it did not
            // handle the key, so Escape can propagate normally.
            assert!(
                !ws.cancel_agent_turn_if_active(pane_id, cx),
                "no-op when the pane is idle"
            );

            // Simulate a turn in flight (as `send_prompt` would); this makes
            // `is_busy()` true.
            agent_view(ws, pane_id).update(cx, |v, _| v.set_turn_in_flight());
            assert!(
                ws.cancel_agent_turn_if_active(pane_id, cx),
                "cancels and reports handled when the pane is busy"
            );

            // An id that is not an agent chat pane reports not-handled, so
            // Escape keeps propagating to ancestors.
            let bogus: PaneId = pane_id + 999;
            assert!(
                !ws.cancel_agent_turn_if_active(bogus, cx),
                "no-op for an id that is not an agent chat pane"
            );
        });
    })
    .unwrap();
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
            super::fresh_test_data_dir(),
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
                v.status = AgentSessionStatus::Connected;
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

#[gpui::test]
async fn bottom_dock_snapshot_reflects_active_agent_queue(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let first = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let first_id = first.id;
            ws.active_runtime_mut().panes.push(first);
            ws.send_agent_prompt_text(first_id, "inactive".to_string(), cx);

            let second = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let second_id = second.id;
            ws.active_runtime_mut().panes.push(second);
            ws.active_runtime_mut().focused_pane_id = second_id;
            ws.send_agent_prompt_text(second_id, "active first".to_string(), cx);
            ws.send_agent_prompt_text(second_id, "active second".to_string(), cx);

            let snap = ws.prepare_bottom_dock_snapshot(cx);
            let (pane_id, queued) = snap
                .queued_prompts
                .as_ref()
                .expect("active agent queue is projected into the bottom snapshot");
            assert_eq!(*pane_id, second_id);
            assert_eq!(
                queued.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
                vec!["active first".to_string(), "active second".to_string()],
                "only the focused agent pane's queue is projected"
            );

            ws.active_runtime_mut().focused_pane_id = first_id;
            let snap = ws.prepare_bottom_dock_snapshot(cx);
            let (pane_id, queued) = snap
                .queued_prompts
                .as_ref()
                .expect("newly active agent queue is projected");
            assert_eq!(*pane_id, first_id);
            assert_eq!(
                queued.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
                vec!["inactive".to_string()]
            );
        });
    })
    .unwrap();
}

/// `deliver_text_to_pane` dispatch table: the funnel routes by pane kind at one
/// place. An AgentChat submit with a non-empty body echoes/queues a prompt; a
/// whitespace-only submit is an accepted no-op (no blank ACP turn — the
/// "Enter-only" macro case); a non-text pane (TaskEdit) and a missing id both
/// return `false`.
#[gpui::test]
async fn deliver_text_to_pane_routes_by_kind(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // AgentChat pane: non-empty submit → accepted + queued (no live
            // handle here, so the prompt is queued rather than echoed).
            let chat = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let chat_id = chat.id;
            ws.active_runtime_mut().panes.push(chat);
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "  do the thing  ".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "AgentChat submit is accepted"
            );
            {
                let view = agent_view(ws, chat_id);
                let view = view.read(cx);
                assert!(view.items.is_empty(), "a queued prompt is not echoed");
                assert_eq!(
                    queue_texts(view),
                    vec!["do the thing".to_string()],
                    "the body is trimmed at the single dispatch point, then queued"
                );
            }

            // Whitespace-only submit → accepted no-op: no new queue entry, no turn.
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "   \n  ".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "whitespace-only submit is an accepted no-op (returns true)"
            );
            {
                let view = agent_view(ws, chat_id);
                let view = view.read(cx);
                assert_eq!(
                    queue_texts(view).len(),
                    1,
                    "a blank submit adds no queue entry and fires no ACP turn"
                );
                assert!(view.items.is_empty());
                assert!(view.turn_is_idle());
            }

            // Missing pane id → false.
            let bogus: PaneId = chat_id + 9999;
            assert!(
                !ws.deliver_text_to_pane(
                    bogus,
                    PaneTextInput {
                        body: "x".to_string(),
                        intent: PaneTextIntent::Command { submit: true },
                    },
                    window,
                    cx,
                ),
                "a missing pane id cannot receive text"
            );

            // TaskEdit pane → false (kind cannot receive delivered text).
            ws.open_task_edit_pane(None, window, cx);
            let te_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("open_task_edit_pane pushed a pane")
                .id;
            assert!(
                !ws.deliver_text_to_pane(
                    te_id,
                    PaneTextInput {
                        body: "x".to_string(),
                        intent: PaneTextIntent::Command { submit: false },
                    },
                    window,
                    cx,
                ),
                "a TaskEdit pane is not a text-delivery target"
            );
        });
    })
    .unwrap();
}

/// A non-default `codex` id. Used by the restore-wiring tests below to prove the
/// persisted `agent_id` is threaded through `restore_from_disk` (not just the
/// pure `resolve_restored_agent` seam covered by unit tests).
fn codex_agent() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
        default_mode: None,
    }
}

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

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
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
                    v.session_title = Some("Investigate flaky test".to_string());
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
                ws.snapshot_for_disk(cx).expect("snapshot")
            })
        })
        .unwrap();

    // Restore into a fresh workspace whose catalog still includes `codex`.
    let restored_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test(
            &config,
            None,
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.agents = vec![
            daruda_config::AgentDefinition::claude_default(),
            codex_agent(),
        ];
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, cx| {
        let views = ws
            .active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.agent_chat_view())
            .map(|view| {
                let view = view.read(cx);
                (
                    view.agent_id.clone(),
                    view.session_id.clone(),
                    view.session_title.clone(),
                    view.restoring,
                    view.handle.is_none(),
                )
            })
            .collect::<Vec<_>>();
        let titled = views
            .iter()
            .find(|(_, session_id, _, _, _)| session_id.as_deref() == Some("sess-restore-123"))
            .expect("restored titled agent chat pane present");
        assert_eq!(
            titled.2.as_deref(),
            Some("Investigate flaky test"),
            "persisted title must round-trip (seeds the tab label before load)"
        );
        assert!(!titled.3, "a restored dormant pane is not yet loading");
        assert!(titled.4, "no live session until first focus connects");

        let codex = views
            .iter()
            .find(|(agent_id, _, _, _, _)| agent_id == "codex")
            .expect("restored codex-owned pane present");
        assert_eq!(
            codex.0, "codex",
            "the owning agent is still in the catalog, so the pane relaunches under it"
        );
        assert_eq!(
            codex.1.as_deref(),
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
            super::fresh_test_data_dir(),
            window,
            cx,
        );
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let fallback = fallback_handle.root(cx).unwrap();

    fallback.read_with(cx, |ws, cx| {
        let default_id = daruda_config::AgentDefinition::claude_default().id;
        let views = ws
            .active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.agent_chat_view())
            .map(|view| {
                let view = view.read(cx);
                (
                    view.agent_id.clone(),
                    view.session_id.clone(),
                    view.session_title.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            views.iter().all(|(agent_id, _, _)| *agent_id == default_id),
            "every restored agent-chat pane falls back to or keeps the default agent"
        );
        assert!(
            views.iter().any(|(_, session_id, title)| {
                session_id.as_deref() == Some("sess-restore-123")
                    && title.as_deref() == Some("Investigate flaky test")
            }),
            "a default-agent session keeps its persisted id and title"
        );
        assert!(
            views
                .iter()
                .any(|(_, session_id, title)| { session_id.is_none() && title.is_none() }),
            "session id is dropped when its owning agent is absent (resume invalid)"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// A second agent entry so open/switch/split tests have a non-default agent to
/// pick. `claude_default()` is catalog[0] (the session default); `codex` is the
/// distinct target.
fn codex() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
        default_mode: None,
    }
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
    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
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
            ws.agents = vec![daruda_config::AgentDefinition::claude_default(), codex()];
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
                src_view.agent_id, claude_id,
                "the source pane keeps chatting under its own agent"
            );
            assert!(src_view.handle.is_none(), "the source session is untouched");

            // The new pane runs under the target agent.
            let new_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the switch pushed a pane")
                .id;
            assert_ne!(new_id, src_id, "a distinct pane was opened");
            assert_eq!(
                agent_view(ws, new_id).read(cx).agent_id,
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
                agent_view(ws, split_id).read(cx).agent_id,
                "codex",
                "the split inherits the source pane's agent, not the catalog default"
            );
        });
    })
    .unwrap();
}

/// The ↑ consume-predicate `history_navigate_possible`: a non-empty queue on the
/// focused agent pane makes ↑ consumable even with no lane input history (so the
/// key reaches `do_history_navigate` and begins the edit); an empty queue with no
/// history does not consume ↑, and Down is never affected by the queue.
#[gpui::test]
async fn up_arrow_consumes_queue_and_edits_last_prompt(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.active_runtime_mut().focused_pane_id = id;
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    // Empty queue, no history → ↑ falls through to cursor movement.
    workspace.read_with(cx, |ws, cx| {
        assert!(
            !ws.history_navigate_possible(crate::ui::HistoryDir::Up, cx),
            "no queue and no history → ↑ is not consumed"
        );
        assert!(!ws.history_navigate_possible(crate::ui::HistoryDir::Down, cx));
    });

    // Enqueue offline (no handle → buffered, not sent); no input history is
    // recorded by this path.
    let last_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.send_agent_prompt_text(pane_id, "q1".into(), cx);
                ws.send_agent_prompt_text(pane_id, "q2".into(), cx);
                let last = agent_view(ws, pane_id)
                    .read(cx)
                    .queue
                    .pending_prompts
                    .last()
                    .expect("queue non-empty")
                    .id;
                ws.terminal_input
                    .update(cx, |s, cx_state| s.set_value("typing", window, cx_state));
                last
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.do_history_navigate(crate::ui::HistoryDir::Up, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "typing",
            "a non-empty composer is left for history recall, not a queue edit"
        );
        assert!(
            agent_view(ws, pane_id)
                .read(cx)
                .queue
                .editing_prompt
                .is_none(),
            "no queue edit begins when the composer is non-empty"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("", window, cx_state));
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, cx| {
        assert!(
            ws.history_navigate_possible(crate::ui::HistoryDir::Up, cx),
            "a queued prompt makes Up consumable even without input history"
        );
        assert!(
            !ws.history_navigate_possible(crate::ui::HistoryDir::Down, cx),
            "Down is unaffected by the queue"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.do_history_navigate(crate::ui::HistoryDir::Up, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "q2",
            "the composer receives the last queued prompt's text"
        );
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(last_id),
            "the editing flag targets the last queued prompt"
        );
    });
}

/// Queue edit exit paths clear the editing flag and composer. Cancel leaves the
/// queued prompt intact; clear-all deletes it; whitespace submit cancels the
/// edit rather than sending an empty prompt.
#[gpui::test]
async fn queued_prompt_edit_exit_paths_clear_state(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::{PaneTextInput, PaneTextIntent};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let (pane_id, id) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    Some(PaneCwd::Local(tmp.clone())),
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.active_runtime_mut().focused_pane_id = id;
                ws.send_agent_prompt_text(id, "editable".into(), cx);
                let prompt_id = agent_view(ws, id).read(cx).queue.pending_prompts[0].id;
                ws.begin_edit_queued_prompt(id, prompt_id, window, cx);
                (id, prompt_id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    // begin pulled the text into the composer and set the editing flag.
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.terminal_input.read(cx).value(), "editable");
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(id)
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.cancel_edit_queued_prompt(pane_id, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "cancel empties the composer"
        );
        assert!(
            agent_view(ws, pane_id)
                .read(cx)
                .queue
                .editing_prompt
                .is_none(),
            "cancel clears the editing flag; the prompt stays queued"
        );
        assert_eq!(
            queue_texts(agent_view(ws, pane_id).read(cx)),
            vec!["editable".to_string()],
            "cancel leaves the queued prompt intact"
        );
    });

    let clear_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.send_agent_prompt_text(pane_id, "q2".into(), cx);
                let last = agent_view(ws, pane_id)
                    .read(cx)
                    .queue
                    .pending_prompts
                    .last()
                    .unwrap()
                    .id;
                ws.begin_edit_queued_prompt(pane_id, last, window, cx);
                last
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.terminal_input.read(cx).value(), "q2");
        assert_eq!(
            agent_view(ws, pane_id).read(cx).queue.editing_prompt,
            Some(clear_id)
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.clear_queued_prompts(pane_id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        assert!(
            view.read(cx).queue.pending_prompts.is_empty(),
            "queue cleared"
        );
        assert!(
            view.read(cx).queue.editing_prompt.is_none(),
            "editing flag cleared"
        );
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "the orphaned edit text is cleared from the composer"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.send_agent_prompt_text(pane_id, "editable".into(), cx);
            let pid = agent_view(ws, pane_id).read(cx).queue.pending_prompts[0].id;
            ws.begin_edit_queued_prompt(pane_id, pid, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    // Submit a whitespace-only body while the edit is in progress.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.deliver_text_to_pane(
                pane_id,
                PaneTextInput {
                    body: "   ".into(),
                    intent: PaneTextIntent::Command { submit: true },
                },
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        assert!(
            view.read(cx).queue.editing_prompt.is_none(),
            "whitespace submit cancels the edit"
        );
        assert_eq!(
            queue_texts(view.read(cx)),
            vec!["editable".to_string()],
            "the queued prompt is left intact (edit cancelled, not sent)"
        );
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "the composer is emptied"
        );
    });
}
