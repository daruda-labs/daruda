//! AgentChat pane — open action and the pure parts of the prompt / permission
//! / mode ops (no live ACP session required).
//!
//! `open_agent_chat_pane` produces a `PaneContent::AgentChat` leaf wrapping a
//! self-owned `Entity<AgentChatView>`; the prompt-echo, permission-resolve,
//! cancel, and mode ops are tested against a view built with
//! `create_agent_chat_pane` (which does not itself open a connection), so no
//! `npx` adapter is ever spawned — the view's `handle` stays `None` and the
//! host-side state transitions still run.

use daruda_acp::{ModeStateView, SessionModeView};
use gpui::{AppContext as _, Entity, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::rows::RowKind;
use crate::workspace::main_area::agent_chat_pane::view::{AgentChatView, AgentSessionStatus};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Fetch the `AgentChatView` entity for `pane_id` (panics if the pane is gone
/// or is not an AgentChat pane).
fn agent_view(ws: &Workspace, pane_id: PaneId) -> Entity<AgentChatView> {
    ws.active_runtime()
        .panes
        .iter()
        .find(|p| p.id == pane_id)
        .and_then(|p| p.agent_chat_view())
        .cloned()
        .expect("agent chat pane present")
}

#[gpui::test]
async fn open_agent_chat_pane_creates_agent_chat_leaf(cx: &mut TestAppContext) {
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
                // The default test workspace has no resolvable lane cwd, so the
                // pane parks in `Error` rather than attempting a (subprocess)
                // connection — keeps the suite offline.
                assert!(
                    matches!(view.status, AgentSessionStatus::Error(_)),
                    "no lane cwd → error status, not a live connect, got {:?}",
                    view.status
                );
                assert!(view.items.is_empty(), "items start empty");
                assert!(view.handle.is_none(), "no session without a cwd");
            }
            _ => panic!("expected an AgentChat pane"),
        }
        assert_eq!(ws.active_runtime().focused_pane_id, pane.id);
    });
}

/// Task 2's core virtualization invariant: `sync_list_after` keeps the
/// `ListState` item count exactly in step with `items`. A desync would make the
/// virtualized `list` render the wrong rows (or index out of range), so this
/// pins the count after a sequence of appends driven through a public op.
#[gpui::test]
async fn list_state_count_tracks_items(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                // Each prompt echoes one `UserText` item (no live handle, so no
                // turn) and routes through `send_prompt_text` → `rebuild_rows`.
                for n in 0..3 {
                    ws.send_agent_prompt_text(id, format!("prompt {n}"), cx);
                }
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert_eq!(view.items.len(), 3, "three prompts were echoed");
        // The virtualized list indexes over projected rows, so its count must
        // track `rows`, not `items` (here they match: 3 bare user messages, no
        // agent responses → no synthetic headers).
        assert_eq!(view.list_state.item_count(), view.rows.len());
        assert_eq!(view.rows.len(), 3);
    });
}

/// Collapse-all hides every agent row of a response (the header stays visible);
/// expand-all shows them again. Exercises `set_all_folds` → `collect_foldable_keys`
/// → `rebuild_rows` across the response + tool-group levels.
#[gpui::test]
async fn fold_all_collapses_then_expands_the_response(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let tool = |id: &str| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        })
    };

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                view.update(cx, |v, cx| {
                    v.items = vec![
                        ChatItem::UserText("q".into()),
                        ChatItem::AssistantText {
                            text: "a".into(),
                            streaming: false,
                            message_id: None,
                        },
                        tool("c1"),
                        tool("c2"),
                    ];
                    v.set_all_folds(false, cx);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        // The run's only assistant text ("a", items index 1) is the turn's
        // conclusion: it projects as a ConclusionItem and stays visible even
        // under collapse-all. The process (other agent rows + tool group) hides.
        let process_all_hidden = view
            .rows
            .iter()
            .filter(|r| {
                matches!(
                    r.kind,
                    RowKind::AgentItem(_) | RowKind::ToolGroupHeader { .. }
                )
            })
            .all(|r| r.hidden);
        assert!(process_all_hidden, "collapse-all hides the process");
        assert!(
            view.rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::ConclusionItem(1)) && !r.hidden),
            "the conclusion (run's last assistant text) stays visible"
        );
        assert!(
            view.rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. }) && !r.hidden),
            "the response header itself stays visible (it is the toggle)"
        );
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            agent_view(ws, pane_id).update(cx, |v, cx| v.set_all_folds(true, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(
            view.rows.iter().all(|r| !r.hidden),
            "expand-all shows every row"
        );
    });
}

/// Task 1's core perf contract: a `cx.notify()` on the `AgentChatView` must
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
async fn send_agent_prompt_text_echoes_user_text(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                // `create_agent_chat_pane` builds the pane but does not open a
                // connection — that is the caller's job — so this never spawns
                // an adapter. Push it directly into the tree.
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                // The prompt arrives from the shared bottom-dock input via the
                // `send_agent_prompt_text` shim (the pane no longer owns an
                // input); it routes into the view.
                ws.send_agent_prompt_text(id, "hello agent".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert_eq!(
            view.items.len(),
            1,
            "the submitted prompt is echoed as one UserText item"
        );
        assert_eq!(
            view.items[0],
            daruda_acp::ChatItem::UserText("hello agent".to_string())
        );
        // No live handle → the turn is not marked in flight.
        assert!(!view.turn_in_flight);
    });
}

#[gpui::test]
async fn respond_permission_resolves_the_pending_card(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                // Inject a pending permission card + its pending id, as the
                // event pump would have on a `PermissionRequested` event, then
                // resolve it through the view op the permission button drives.
                view.update(cx, |v, cx| {
                    v.items.push(ChatItem::Permission(PermissionItem {
                        tool_title: Some("Write /tmp/x".to_string()),
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    v.pending_permission = Some(42);
                    v.respond_permission(
                        "allow_once".to_string(),
                        PermissionKindView::AllowOnce,
                        cx,
                    );
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        let ChatItem::Permission(card) = &view.items[0] else {
            panic!("expected a permission card");
        };
        assert_eq!(
            card.resolved,
            Some(PermissionResolution::Chosen("allow_once".to_string())),
            "the chosen option is recorded on the card"
        );
        assert!(
            view.pending_permission.is_none(),
            "the pending id is cleared once resolved"
        );
    });
}

/// A pending permission surfaces out of a collapsed response (it is
/// actionable), and resolving it folds it straight back into the process —
/// `respond_permission` reprojects the rows so the fold-back is immediate, not
/// deferred to the next agent event.
#[gpui::test]
async fn resolved_permission_folds_back_immediately(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem, ToolKindView,
        ToolStatusView,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let tool = |id: &str| {
        ChatItem::ToolCall(ToolCallItem {
            id: id.to_owned(),
            title: format!("Tool {id}"),
            kind: ToolKindView::Edit,
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: None,
        })
    };

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                agent_view(ws, id).update(cx, |v, cx| {
                    v.items = vec![
                        ChatItem::UserText("q".into()),
                        tool("a"),
                        tool("b"),
                        ChatItem::Permission(PermissionItem {
                            tool_title: Some("Write /tmp/x".to_string()),
                            options: vec![PermissionChoice {
                                option_id: "allow_once".to_string(),
                                name: "Allow".to_string(),
                                kind: PermissionKindView::AllowOnce,
                            }],
                            resolved: None,
                        }),
                    ];
                    v.pending_permission = Some(7);
                    // Collapse the response — the pending card must still surface.
                    v.set_all_folds(false, cx);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let perm_hidden = |ws: &Workspace, cx: &gpui::App| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        view.rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(3)))
            .expect("permission row present")
            .hidden
    };

    workspace.read_with(cx, |ws, cx| {
        assert!(
            !perm_hidden(ws, cx),
            "a pending permission stays visible under a collapsed response"
        );
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            agent_view(ws, pane_id).update(cx, |v, cx| {
                v.respond_permission("allow_once".to_string(), PermissionKindView::AllowOnce, cx);
            });
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert!(
            perm_hidden(ws, cx),
            "a resolved permission folds back into the collapsed response immediately"
        );
    });
}

#[gpui::test]
async fn agent_chat_pane_without_cwd_carries_reason_not_prefix(cx: &mut TestAppContext) {
    use crate::surface::strings as s;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    // No resolvable lane cwd → the pane parks in `Error`. The message must be
    // the bare reason: the status banner re-adds the error prefix, so storing
    // the prefix here would render it doubled.
    let status = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(None, None, None, window, cx);
                match &pane.content {
                    PaneContent::AgentChat(ac) => ac.view.read(cx).status.clone(),
                    _ => panic!("expected an AgentChat pane"),
                }
            })
        })
        .unwrap();

    match status {
        AgentSessionStatus::Error(message) => {
            assert_eq!(message, s::agent_chat_no_lane_cwd());
            assert_ne!(
                message,
                s::agent_chat_error_prefix(),
                "payload must be the reason, not the prefix the banner re-adds"
            );
        }
        other => panic!("expected an Error status, got {other:?}"),
    }
}

/// A pane with a working directory parks in `Idle`, not `Connecting`: the live
/// ACP session is started lazily on first focus, not at construction. This is
/// what keeps cold restore from spinning up an agent process per restored pane.
#[gpui::test]
async fn agent_chat_pane_with_cwd_is_idle_until_focus(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let status = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                match &pane.content {
                    PaneContent::AgentChat(ac) => ac.view.read(cx).status.clone(),
                    _ => panic!("expected an AgentChat pane"),
                }
            })
        })
        .unwrap();

    assert_eq!(
        status,
        AgentSessionStatus::Idle,
        "a pane with a cwd must stay dormant until first focus, got {status:?}"
    );
}

/// `AgentChatView::set_mode` immediately updates `modes.current` (optimistic
/// update) and is idempotent when the handle is absent (no live ACP session
/// required).
#[gpui::test]
async fn set_mode_updates_current_optimistically(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                view.update(cx, |v, cx| {
                    // Inject a ModeStateView with two modes so `set_mode` has
                    // something to flip. No live handle (handle stays `None`).
                    v.modes = Some(ModeStateView {
                        available: vec![
                            SessionModeView {
                                id: "auto".to_string(),
                                name: "Auto".to_string(),
                                description: None,
                            },
                            SessionModeView {
                                id: "plan".to_string(),
                                name: "Plan".to_string(),
                                description: Some("Plan mode".to_string()),
                            },
                        ],
                        current: "auto".to_string(),
                    });
                    v.set_mode("plan".to_string(), cx);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        let modes = view.modes.as_ref().expect("modes were injected");
        assert_eq!(
            modes.current, "plan",
            "set_mode flips current immediately (optimistic)"
        );
    });
}

#[gpui::test]
async fn cancel_agent_turn_cancels_the_pending_permission(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, PermissionChoice, PermissionItem, PermissionResolution};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                // Inject a pending permission card + its pending id, as the
                // event pump would have on a `PermissionRequested` event.
                view.update(cx, |v, _| {
                    v.items.push(ChatItem::Permission(PermissionItem {
                        tool_title: Some("Write /tmp/x".to_string()),
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: daruda_acp::PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    v.pending_permission = Some(7);
                });
                // No live handle (offline) — cancel still drains the pending
                // permission host-side via the bottom-dock shim: the card
                // resolves to `Cancelled` and the pending id clears.
                ws.cancel_agent_turn(id, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        let ChatItem::Permission(card) = &view.items[0] else {
            panic!("expected a permission card");
        };
        assert_eq!(
            card.resolved,
            Some(PermissionResolution::Cancelled),
            "cancelling the turn marks the pending card cancelled"
        );
        assert!(
            view.pending_permission.is_none(),
            "the pending id is cleared on cancel"
        );
    });
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                // A turn mid-flight: streaming text + a still-running tool call,
                // exactly the state a hung agent would leave (it never sends a
                // stop reason, so `TurnEnded` would never clear this).
                view.update(cx, |v, _| {
                    v.turn_in_flight = true;
                    v.turn_started_at = Some(std::time::Instant::now());
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
                            status: ToolStatusView::InProgress,
                            diffs: Vec::new(),
                            output: Vec::new(),
                            raw_input: None,
                            parent_tool_id: None,
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
            !view.turn_in_flight,
            "Stop ends the turn without an agent reply"
        );
        assert!(
            view.turn_started_at.is_none(),
            "the elapsed timer is cleared"
        );
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
}

#[gpui::test]
async fn cancel_if_in_flight_only_cancels_a_running_turn(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
            let id = pane.id;
            ws.active_runtime_mut().panes.push(pane);

            // No turn running yet → the Escape shim is a no-op and reports it
            // did not handle the key, so Escape can propagate normally.
            assert!(
                !ws.cancel_agent_turn_if_in_flight(id, cx),
                "no-op when no turn is in flight"
            );

            // Simulate a turn in flight, as `send_prompt` would.
            agent_view(ws, id).update(cx, |v, _| v.turn_in_flight = true);
            assert!(
                ws.cancel_agent_turn_if_in_flight(id, cx),
                "cancels and reports handled when a turn is in flight"
            );

            // An id that is not an agent chat pane reports not-handled, so
            // Escape keeps propagating to ancestors.
            let bogus: PaneId = id + 999;
            assert!(
                !ws.cancel_agent_turn_if_in_flight(bogus, cx),
                "no-op for an id that is not an agent chat pane"
            );
        });
    })
    .unwrap();
}

/// Regression for "the agent status indicator disappears after a lane
/// switch": a parked lane's AgentChat session status must still reach the
/// **rendered** left-dock snapshot. Drives a *real* `activate_lane` switch
/// between two lanes and asserts on the output of the real
/// `prepare_left_dock_snapshot` — the same `agent_status_per_lane` map the
/// per-lane row keys its badge from. This is the production render path, not
/// a re-implementation.
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
            // exactly as `open_agent_chat_pane` does (pane + TabEntry) so it
            // appears in `pane_lane_index`; do it directly rather than via the
            // open action to skip `focus_pane`'s lazy `maybe_connect` (which
            // would spawn a real adapter). `create_agent_chat_pane` itself
            // opens no connection.
            let pane = ws.create_agent_chat_pane(Some(root_a.clone()), None, None, window, cx);
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
                v.turn_in_flight = true;
            });

            // Switch away → lane 0 is now parked, lane 1 active.
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1);

            // Build the actual left-dock snapshot the renderer consumes and
            // assert the parked lane's badge source is `Working`.
            let snap = ws.prepare_left_dock_snapshot(cx);
            assert_eq!(
                snap.agent_status_per_lane.get(&lane0),
                Some(&daruda_claude::SessionStatus::Working),
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
async fn agent_chat_view_finds_a_pane_parked_in_an_inactive_lane(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
            let id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            assert!(
                ws.agent_chat_view(id).is_some(),
                "found while live in the active lane"
            );

            // Simulate a lane switch: the pane moves into a *different*
            // lane's runtime in the single `runtimes` map while `self.active`
            // points elsewhere. (The key is a distinct parked lane — the
            // lookup scans every runtime, not just the active one.)
            let parked = ws
                .active_runtime_mut()
                .panes
                .pop()
                .expect("the pane we just pushed");
            let parked_key = daruda_store::project::LaneRef {
                project: ws.active.project,
                lane: ws.active.lane + 1,
            };
            ws.main_area.runtimes.insert(
                parked_key,
                crate::workspace::LaneRuntime {
                    tabs: Vec::new(),
                    panes: vec![parked],
                    active_tab_index: 0,
                    tab_history: Vec::new(),
                    focused_pane_id: id,
                },
            );

            // The event pump looks the view up by id on every ACP event; it must
            // still resolve while the lane is parked, or the pump breaks and the
            // session's responses are dropped after a lane switch.
            assert!(
                ws.agent_chat_view(id).is_some(),
                "agent_chat_view must find a pane parked in an inactive lane"
            );
        });
    })
    .unwrap();
}

/// A prompt submitted before the session connects (`handle` is still `None`,
/// the lazy-connect state) must be echoed locally *and* buffered — never
/// silently dropped — with no turn marked in flight (nothing is on the wire
/// yet). Buffering preserves submission order.
#[gpui::test]
async fn prompt_before_connect_is_buffered_not_dropped(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                // A pane with a cwd parks in `Idle` with `handle: None` — the
                // session connects lazily on first focus, which this test never
                // triggers, so no `npx` adapter is spawned.
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.send_agent_prompt_text(id, "first".to_string(), cx);
                ws.send_agent_prompt_text(id, "second".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(view.handle.is_none(), "not connected");
        // Both prompts echoed locally so the user sees them immediately.
        assert_eq!(view.items.len(), 2, "both prompts echoed as UserText");
        // Buffered in FIFO order rather than dropped.
        assert_eq!(
            view.pending_prompts,
            vec!["first".to_string(), "second".to_string()],
            "disconnected prompts are queued in submission order"
        );
        // Nothing is on the wire, so no turn is in flight.
        assert!(
            !view.turn_in_flight,
            "no turn until a handle carries a prompt"
        );
        assert!(view.turn_started_at.is_none());
    });
}

/// Stop must halt everything queued, not just the live turn. With prompts
/// buffered behind the running turn, `cancel_turn` clears the whole queue so the
/// cancelled turn's later `TurnEnded` → `pump_pending_prompt` finds an empty
/// buffer and cannot silently auto-fire the next prompt (the bottom-dock input
/// does not gate Send on `turn_in_flight`, so this queue-behind-a-turn state is
/// reachable in normal use).
#[gpui::test]
async fn cancel_turn_clears_queued_prompts(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                // Disconnected (no handle): every prompt buffers, none is on the
                // wire. Buffer several so there is a real queue behind Stop.
                ws.send_agent_prompt_text(id, "a".to_string(), cx);
                ws.send_agent_prompt_text(id, "b".to_string(), cx);
                ws.send_agent_prompt_text(id, "c".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.pending_prompts.len(),
            3,
            "three prompts queued behind the turn"
        );
    });

    // Stop.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.cancel_agent_turn(pane_id, cx));
    })
    .unwrap();
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(
            v.pending_prompts.is_empty(),
            "Stop clears the queued prompts so nothing auto-resumes"
        );
        assert!(!v.turn_in_flight, "Stop ends the turn");
    });
}

/// Prompts submitted while disconnected buffer in FIFO order and never mark a
/// turn in flight — the one-per-turn model buffers everything until a handle
/// exists (drained one at a time by `pump_pending_prompt`, whose handle-send
/// side needs a live `AcpSessionHandle` and so is covered by types/compile).
#[gpui::test]
async fn disconnected_prompts_buffer_fifo_without_a_turn(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.send_agent_prompt_text(id, "a".to_string(), cx);
                ws.send_agent_prompt_text(id, "b".to_string(), cx);
                ws.send_agent_prompt_text(id, "c".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.pending_prompts,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "disconnected prompts buffer in submission (FIFO) order, none dropped"
        );
        assert_eq!(v.items.len(), 3, "all three echoed locally");
        assert!(
            !v.turn_in_flight,
            "nothing is on the wire while disconnected"
        );
    });
}

/// `pump_pending_prompt` is guarded: with no live handle it cannot send, so it
/// leaves the buffer intact and never marks a turn in flight (the send side is
/// only reachable once a real `AcpSessionHandle` is stored). An empty buffer is
/// likewise a no-op. This pins the guards that keep the one-per-turn drain from
/// firing prematurely.
#[gpui::test]
async fn pump_pending_prompt_is_a_noop_without_a_handle(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.send_agent_prompt_text(id, "queued".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    cx.update(|cx| view.update(cx, |v, cx| v.pump_pending_prompt(cx)));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.pending_prompts,
            vec!["queued".to_string()],
            "no handle → pump leaves the buffer intact"
        );
        assert!(!v.turn_in_flight, "no handle → no turn started");
    });

    // Empty buffer is also a no-op (does not panic / mark a turn).
    let empty_view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    cx.update(|cx| {
        empty_view.update(cx, |v, cx| {
            v.pending_prompts.clear();
            v.pump_pending_prompt(cx);
        })
    });
    empty_view.read_with(cx, |v, _| {
        assert!(v.pending_prompts.is_empty());
        assert!(!v.turn_in_flight);
    });
}

/// `deliver_text_to_pane` dispatch table: the funnel routes by pane kind at one
/// place. An AgentChat submit with a non-empty body echoes/queues a prompt; a
/// whitespace-only submit is an accepted no-op (no blank ACP turn — the
/// "Enter-only" macro case); a non-text pane (TaskEdit) and a missing id both
/// return `false`.
#[gpui::test]
async fn deliver_text_to_pane_routes_by_kind(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::PaneTextInput;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // AgentChat pane: non-empty submit → accepted + echoed/queued.
            let chat = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
            let chat_id = chat.id;
            ws.active_runtime_mut().panes.push(chat);
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "  do the thing  ".to_string(),
                        submit: true,
                    },
                    window,
                    cx,
                ),
                "AgentChat submit is accepted"
            );
            {
                let view = agent_view(ws, chat_id);
                let view = view.read(cx);
                assert_eq!(view.items.len(), 1, "non-empty submit echoes one prompt");
                assert_eq!(
                    view.items[0],
                    daruda_acp::ChatItem::UserText("do the thing".to_string()),
                    "the body is trimmed at the single dispatch point"
                );
            }

            // Whitespace-only submit → accepted no-op: no new echo, no turn.
            assert!(
                ws.deliver_text_to_pane(
                    chat_id,
                    PaneTextInput {
                        body: "   \n  ".to_string(),
                        submit: true,
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
                    view.items.len(),
                    1,
                    "a blank submit adds no echo and fires no ACP turn"
                );
                assert!(!view.turn_in_flight);
            }

            // Missing pane id → false.
            let bogus: PaneId = chat_id + 9999;
            assert!(
                !ws.deliver_text_to_pane(
                    bogus,
                    PaneTextInput {
                        body: "x".to_string(),
                        submit: true,
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
                        submit: false,
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

/// Save → restore round-trip: an AgentChat pane's persisted `session_id` and
/// `title` survive `snapshot_for_disk` → `restore_from_disk`, so a later launch
/// can resume the prior conversation via `session/load` and show its label
/// before loading. The pane is placed in a *non-active* tab so the restore's
/// focus-pane never triggers the lazy connect — the suite stays offline (no
/// adapter subprocess).
#[gpui::test]
async fn agent_chat_session_id_and_title_survive_save_restore(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join("daruda_agent_chat_restore_roundtrip");
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    // 1) Build a workspace with a real project (so the agent chat pane carries a
    //    resolvable lane cwd), inject an AgentChat pane into a second tab, and
    //    stamp a live session id + title on its view as a connect would.
    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| w.path.clone());
                let pane = ws.create_agent_chat_pane(cwd, None, None, window, cx);
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                // Append as a new, *non-active* tab (active_tab_index stays on
                // the boot terminal tab) so restore focuses the terminal, not
                // this pane — its lazy connect never fires.
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });

                // Stamp the session identity as a connected session would.
                let view = ws
                    .agent_chat_view(pane_id)
                    .cloned()
                    .expect("agent chat view present");
                view.update(cx, |v, cx| {
                    v.session_id = Some("sess-restore-123".to_string());
                    v.session_title = Some("Investigate flaky test".to_string());
                    cx.notify();
                });

                ws.snapshot_for_disk(cx).expect("snapshot")
            })
        })
        .unwrap();

    // 2) Restore into a fresh, empty workspace.
    let restored_handle = cx.add_window(|window, cx| {
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
    let restored = restored_handle.root(cx).unwrap();

    // 3) The restored AgentChat pane's view carries the persisted id + title,
    //    and stays dormant (no live session, not yet loading).
    restored.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .cloned()
            .expect("restored agent chat pane present");
        let view = view.read(cx);
        assert_eq!(
            view.session_id.as_deref(),
            Some("sess-restore-123"),
            "persisted session id must round-trip"
        );
        assert_eq!(
            view.session_title.as_deref(),
            Some("Investigate flaky test"),
            "persisted title must round-trip (seeds the tab label before load)"
        );
        assert!(
            !view.restoring,
            "a restored dormant pane is not yet loading"
        );
        assert!(
            view.handle.is_none(),
            "no live session until first focus connects"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// While `restoring` (a `session/load` replay in flight), `apply_event`
/// accumulates model state but coalesces the row rebuild: the projection is not
/// rebuilt per replayed event, only once when the finishing `Connected` clears
/// the gate.
#[gpui::test]
async fn restoring_gate_defers_rebuild_until_connected(cx: &mut TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let (window_handle, workspace) = build_workspace(cx);
    let tmp = std::env::temp_dir();

    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let view = agent_view(ws, id);
                view.update(cx, |v, cx| {
                    // Simulate a resume mid-replay: gate set, items populated by
                    // the replayed updates, rows not yet projected.
                    v.restoring = true;
                    v.items = vec![
                        ChatItem::UserText("q".into()),
                        ChatItem::AssistantText {
                            text: "a".into(),
                            streaming: false,
                            message_id: None,
                        },
                    ];
                    v.rows.clear();

                    // A non-terminal event during the replay must NOT rebuild the
                    // rows (the gate defers it).
                    v.apply_event(AcpEvent::Notice("still loading".into()), "", false, cx);
                    assert!(v.restoring, "gate stays set until Connected/Error");
                    assert!(
                        v.rows.is_empty(),
                        "row rebuild is coalesced while restoring"
                    );
                    assert_eq!(v.items.len(), 2, "items still accumulate during replay");

                    // The finishing Connected reply clears the gate and runs the
                    // single catch-up rebuild, keeping the replayed items.
                    v.apply_event(
                        AcpEvent::Connected {
                            session_id: "sess-1".into(),
                            modes: None,
                            config_options: Vec::new(),
                        },
                        "",
                        false,
                        cx,
                    );
                    assert!(!v.restoring, "Connected releases the gate");
                    assert_eq!(v.session_id.as_deref(), Some("sess-1"));
                    assert!(
                        !v.rows.is_empty(),
                        "the catch-up rebuild projects the replayed items"
                    );
                    assert_eq!(v.items.len(), 2, "resume keeps the replayed conversation");
                });
                id
            })
        })
        .unwrap();
    let _ = pane_id;
}

/// The replay gate is also released on a terminal `Error` and by the pump's
/// end-of-stream `abort_restore` guard, so a load that never reaches `Connected`
/// can't freeze the pane mid-restore.
#[gpui::test]
async fn restoring_cleared_on_error_and_abort(cx: &mut TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let (window_handle, workspace) = build_workspace(cx);
    let tmp = std::env::temp_dir();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(Some(tmp.clone()), None, None, window, cx);
            let id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            let view = agent_view(ws, id);

            // A terminal error mid-restore clears the gate.
            view.update(cx, |v, cx| {
                v.restoring = true;
                v.apply_event(AcpEvent::Error("load rejected".into()), "", false, cx);
                assert!(!v.restoring, "Error releases the restore gate");
            });

            // The end-of-stream guard releases a gate stuck with no
            // Connected/Error, projecting whatever accumulated.
            view.update(cx, |v, cx| {
                v.restoring = true;
                v.items = vec![ChatItem::UserText("q".into())];
                v.rows.clear();
                v.abort_restore(cx);
                assert!(!v.restoring, "abort_restore releases the gate");
                assert!(
                    !v.rows.is_empty(),
                    "abort_restore projects the accumulated items"
                );
            });
        })
    })
    .unwrap();
}

/// `reset_for_new_session` (the `/clear` teardown) wipes the conversation
/// model, the fold overrides, and the persisted session id, and parks the view
/// back in `Connecting` so a fresh `session/new` can supersede it. Only the
/// local reset is exercised here — `Workspace::reset_agent_chat_session`'s
/// `connect_agent_chat` call is async and needs a real adapter, so it is not
/// covered by this test.
#[gpui::test]
async fn reset_for_new_session_clears_conversation_state(cx: &mut TestAppContext) {
    use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
    use daruda_acp::{ChatItem, PlanEntryView, PlanPriority, PlanStatus};

    let (window_handle, workspace) = build_workspace(cx);
    let tmp = std::env::temp_dir();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(tmp.clone()),
                Some("abc".to_string()),
                None,
                window,
                cx,
            );
            let id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            let view = agent_view(ws, id);

            // Seed mutable state a live session would have accumulated.
            view.update(cx, |v, _cx| {
                v.items.push(ChatItem::UserText("hi".into()));
                v.items.push(ChatItem::UserText("again".into()));
                v.turn_in_flight = true;
                v.plan.push(PlanEntryView {
                    content: "step 1".into(),
                    priority: PlanPriority::Medium,
                    status: PlanStatus::Pending,
                });
                // Explicit fold override — the default is `ExpandedWhileActive`
                // (expanded while active=true); collapse it explicitly so the
                // reset's `FoldState::default()` is observably different.
                v.fold.toggle(FoldKey::Tool("call-1".into()), true);
                assert!(
                    !v.fold.is_expanded(&FoldKey::Tool("call-1".into()), true),
                    "sanity: override collapsed the block while active"
                );
            });

            view.update(cx, |v, cx| v.reset_for_new_session(cx));

            let v = view.read(cx);
            assert!(v.items.is_empty(), "reset clears the conversation items");
            assert!(v.rows.is_empty(), "reset splices the projected rows to 0");
            assert_eq!(v.session_id, None, "reset clears the persisted session id");
            assert_eq!(
                v.status,
                AgentSessionStatus::Connecting,
                "reset parks the view in Connecting for the fresh session/new"
            );
            assert!(!v.turn_in_flight, "reset clears the in-flight turn flag");
            assert!(v.plan.is_empty(), "reset clears the execution plan");
            assert!(
                v.fold.is_expanded(&FoldKey::Tool("call-1".into()), true),
                "reset drops fold overrides back to the natural default"
            );
        })
    })
    .unwrap();
}
