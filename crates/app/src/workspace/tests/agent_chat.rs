//! AgentChat pane — open action and the pure parts of the prompt / permission
//! / mode ops (no live ACP session required). Tests build the view via
//! `create_agent_chat_pane`, which opens no connection, so `handle` stays
//! `None` while host-side state transitions still run.

use daruda_acp::{ModeStateView, SessionModeView};
use daruda_store::project::PaneCwd;
use gpui::{AppContext as _, Entity, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::rows::RowKind;
use crate::workspace::main_area::agent_chat_pane::view::{
    ActivityState, AgentChatView, AgentSessionStatus, TurnOutcome,
};
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

/// The queued-prompt texts in FIFO order — the queue holds `QueuedPrompt`
/// (id + text), so tests compare on the text projection.
fn queue_texts(v: &AgentChatView) -> Vec<String> {
    v.pending_prompts.iter().map(|q| q.text.clone()).collect()
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
                // No resolvable lane cwd → the pane parks in `Error` rather
                // than attempting a connection, keeping the suite offline.
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

/// `Pane::cwd()` is the read every local-only consumer trusts —
/// `resolve_default_cwd`'s `Cmd+T` inheritance tier chief among them (see
/// `pane.rs::default_cwd_for_new_pane`). A `PaneCwd::Remote` value must
/// never surface through it: `pane.cwd()` has to come back `None` so
/// `resolve_default_cwd` falls through to the next tier (the active lane's
/// local path) instead of handing a remote-host string to `spawn_pty`'s
/// local-existence check (which would silently fall back further, to
/// `$HOME`).
#[gpui::test]
async fn pane_cwd_returns_none_for_remote(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Remote("host:/repo/lane".to_string())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            assert_eq!(
                pane.cwd(),
                None,
                "PaneCwd::Remote must not surface through Pane::cwd()"
            );
            ws.active_runtime_mut().panes.push(pane);
        });
    })
    .unwrap();
}

/// Companion to the Remote case above: a `PaneCwd::Local` pane's cwd must
/// keep surfacing unchanged through `Pane::cwd()`.
#[gpui::test]
async fn pane_cwd_returns_path_for_local(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            assert_eq!(pane.cwd(), Some(tmp.as_path()));
            ws.active_runtime_mut().panes.push(pane);
        });
    })
    .unwrap();
}

/// Core virtualization invariant: `sync_list_after` keeps the `ListState`
/// item count exactly in step with `items`. A desync would make the
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
                // Seed three bare user messages directly and reproject rows via a
                // public op (`set_all_folds` calls `rebuild_rows`), then assert
                // the virtualized list count stays in step with the rows.
                // (Offline `send_agent_prompt_text` only queues, so seed directly.)
                let view = agent_view(ws, id);
                view.update(cx, |v, cx| {
                    v.items = vec![
                        daruda_acp::ChatItem::UserText("prompt 0".into()),
                        daruda_acp::ChatItem::UserText("prompt 1".into()),
                        daruda_acp::ChatItem::UserText("prompt 2".into()),
                    ];
                    v.set_all_folds(true, cx);
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert_eq!(view.items.len(), 3, "three user messages were seeded");
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

/// A prompt submitted while the pane cannot send now (here: no live handle) is
/// enqueued WITHOUT echoing into the transcript — it lives only in the queue
/// (surfaced by the bottom-dock strip) until it drains, so queued prompts don't
/// clutter the conversation.
#[gpui::test]
async fn send_agent_prompt_text_queues_without_echo_while_offline(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                // `create_agent_chat_pane` opens no connection, so this never
                // spawns an adapter. Push it directly into the tree.
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
                // Panes don't own input; prompts route through the shared
                // bottom-dock input's `send_agent_prompt_text` shim.
                ws.send_agent_prompt_text(id, "hello agent".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(
            view.items.is_empty(),
            "a queued prompt is NOT echoed into the transcript"
        );
        assert_eq!(
            queue_texts(view),
            vec!["hello agent".to_string()],
            "the prompt lives in the queue instead"
        );
        // No live handle → the turn is not marked in flight.
        assert!(view.turn_is_idle());
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
                // Inject a pending permission card + its pending id (as the pump
                // would on a `PermissionRequested` event), then resolve it
                // through the view op the permission button drives.
                view.update(cx, |v, cx| {
                    v.items.push(ChatItem::Permission(PermissionItem {
                        id: 42,
                        tool_title: Some("Write /tmp/x".to_string()),
                        raw_input_summary: None,
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    v.pending_permissions.insert(42);
                    v.respond_permission(
                        42,
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
            !view.has_pending_permission(),
            "the pending id is cleared once resolved"
        );
    });
}

/// Two permissions outstanding at once (parallel tool calls) each resolve to
/// their own card by id — answering the *first* must not clobber the second.
/// Guards against mis-routing the resolution to the newest card.
#[gpui::test]
async fn concurrent_permissions_resolve_independently_by_id(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let card = |id: u64| {
        ChatItem::Permission(PermissionItem {
            id,
            tool_title: Some(format!("Write /tmp/{id}")),
            raw_input_summary: None,
            options: vec![
                PermissionChoice {
                    option_id: "allow_once".to_string(),
                    name: "Allow".to_string(),
                    kind: PermissionKindView::AllowOnce,
                },
                PermissionChoice {
                    option_id: "reject_once".to_string(),
                    name: "Reject".to_string(),
                    kind: PermissionKindView::RejectOnce,
                },
            ],
            resolved: None,
        })
    };

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
                agent_view(ws, id).update(cx, |v, cx| {
                    // Two permissions arrive back-to-back, both outstanding.
                    v.items = vec![card(100), card(200)];
                    v.pending_permissions.insert(100);
                    v.pending_permissions.insert(200);
                    // Answer the FIRST one.
                    v.respond_permission(
                        100,
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
        let ChatItem::Permission(first) = &view.items[0] else {
            panic!("expected first permission card");
        };
        let ChatItem::Permission(second) = &view.items[1] else {
            panic!("expected second permission card");
        };
        assert_eq!(
            first.resolved,
            Some(PermissionResolution::Chosen("allow_once".to_string())),
            "the first card (the one answered) is resolved"
        );
        assert_eq!(
            second.resolved, None,
            "the second card stays live — answering the first must not clobber it"
        );
        assert!(
            view.is_permission_outstanding(200),
            "the second request is still outstanding"
        );
        assert!(
            !view.is_permission_outstanding(100),
            "the first request is no longer outstanding"
        );
        assert!(view.has_pending_permission());
    });

    // Now answer the SECOND one — the pane drains fully.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            agent_view(ws, pane_id).update(cx, |v, cx| {
                v.respond_permission(
                    200,
                    "reject_once".to_string(),
                    PermissionKindView::RejectOnce,
                    cx,
                );
            });
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        let ChatItem::Permission(second) = &view.items[1] else {
            panic!("expected second permission card");
        };
        assert_eq!(
            second.resolved,
            Some(PermissionResolution::Chosen("reject_once".to_string())),
            "the second card resolves to its own chosen option"
        );
        assert!(
            !view.has_pending_permission(),
            "no permissions remain outstanding"
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
                agent_view(ws, id).update(cx, |v, cx| {
                    v.items = vec![
                        ChatItem::UserText("q".into()),
                        tool("a"),
                        tool("b"),
                        ChatItem::Permission(PermissionItem {
                            id: 7,
                            tool_title: Some("Write /tmp/x".to_string()),
                            raw_input_summary: None,
                            options: vec![PermissionChoice {
                                option_id: "allow_once".to_string(),
                                name: "Allow".to_string(),
                                kind: PermissionKindView::AllowOnce,
                            }],
                            resolved: None,
                        }),
                    ];
                    v.pending_permissions.insert(7);
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
                v.respond_permission(
                    7,
                    "allow_once".to_string(),
                    PermissionKindView::AllowOnce,
                    cx,
                );
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
    // the bare reason: the banner re-adds the error prefix, so storing the
    // prefix here would render it doubled.
    let status = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    None,
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
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
/// ACP session starts lazily on first focus, not at construction — so cold
/// restore doesn't spin up an agent process per restored pane.
#[gpui::test]
async fn agent_chat_pane_with_cwd_is_idle_until_focus(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let status = cx
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
                // Inject a pending permission card + its pending id, as the
                // pump would on a `PermissionRequested` event.
                view.update(cx, |v, _| {
                    v.items.push(ChatItem::Permission(PermissionItem {
                        id: 7,
                        tool_title: Some("Write /tmp/x".to_string()),
                        raw_input_summary: None,
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: daruda_acp::PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    v.pending_permissions.insert(7);
                });
                // Offline (no handle): cancel still drains the pending
                // permission host-side — card resolves `Cancelled`, id clears.
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
            !view.has_pending_permission(),
            "the pending id is cleared on cancel"
        );
    });
}

/// Cancelling a turn with *several* permissions outstanding must drain every one
/// — ACP requires each parked request be resolved with a `Cancelled` outcome,
/// so no card is left with live buttons and no park hangs forever.
#[gpui::test]
async fn cancel_agent_turn_drains_all_outstanding_permissions(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let card = |id: u64| {
        ChatItem::Permission(PermissionItem {
            id,
            tool_title: Some(format!("Write /tmp/{id}")),
            raw_input_summary: None,
            options: vec![PermissionChoice {
                option_id: "allow_once".to_string(),
                name: "Allow".to_string(),
                kind: PermissionKindView::AllowOnce,
            }],
            resolved: None,
        })
    };

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
                agent_view(ws, id).update(cx, |v, _| {
                    v.items = vec![card(100), card(200), card(300)];
                    v.pending_permissions.insert(100);
                    v.pending_permissions.insert(200);
                    v.pending_permissions.insert(300);
                });
                ws.cancel_agent_turn(id, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        for item in &view.items {
            let ChatItem::Permission(card) = item else {
                panic!("expected a permission card");
            };
            assert_eq!(
                card.resolved,
                Some(PermissionResolution::Cancelled),
                "every outstanding card is cancelled on turn cancel"
            );
        }
        assert!(
            !view.has_pending_permission(),
            "no permission remains outstanding after cancel"
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
}

#[gpui::test]
async fn cancel_if_in_flight_only_cancels_a_running_turn(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
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

            // Not busy yet → the Escape shim is a no-op and reports it did not
            // handle the key, so Escape can propagate normally.
            assert!(
                !ws.cancel_agent_turn_if_active(id, cx),
                "no-op when the pane is idle"
            );

            // Simulate a turn in flight (as `send_prompt` would); this makes
            // `is_busy()` true.
            agent_view(ws, id).update(cx, |v, _| v.set_turn_in_flight());
            assert!(
                ws.cancel_agent_turn_if_active(id, cx),
                "cancels and reports handled when the pane is busy"
            );

            // An id that is not an agent chat pane reports not-handled, so
            // Escape keeps propagating to ancestors.
            let bogus: PaneId = id + 999;
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

/// `activity_state()` (and the lane badge it drives via `to_session_status`) is
/// derived from turn-in-flight OR a background subagent's child tool still
/// running — with a pending permission taking precedence. Covers the four
/// state combinations, including: turn ended but a background child tool is
/// still running, so the pane must still read `Working`.
#[gpui::test]
async fn activity_state_folds_background_tool_and_permission(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    // A child tool call (parent_tool_id = Some) with the given status.
    let child = |status| {
        ChatItem::ToolCall(ToolCallItem {
            id: "child".into(),
            title: "child".into(),
            kind: ToolKindView::Read,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: Some("parent".into()),
        })
    };

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
                // Connected so `to_session_status` maps activity.
                agent_view(ws, id).update(cx, |v, _| {
                    v.status = AgentSessionStatus::Connected;
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));

    // 1) Turn in flight, no permission, no background tool → Working.
    view.update(cx, |v, _| {
        v.set_turn_in_flight();
        v.items.clear();
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.activity_state(), ActivityState::Working);
        assert_eq!(
            v.to_session_status(),
            Some(daruda_claude::SessionStatus::Working)
        );
    });

    // 2) Turn idle, but a background subagent's child tool still running → the
    //    regression: the pane must stay Working past the turn boundary.
    view.update(cx, |v, _| {
        v.set_turn_idle();
        v.items = vec![child(ToolStatusView::InProgress)];
    });
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.activity_state(),
            ActivityState::Working,
            "a running background child tool keeps the pane Working after end_turn"
        );
        assert_eq!(
            v.to_session_status(),
            Some(daruda_claude::SessionStatus::Working)
        );
    });

    // 3) Pending permission takes precedence over everything else.
    view.update(cx, |v, _| {
        v.set_turn_idle();
        v.items.clear();
        v.pending_permissions.insert(7);
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.activity_state(), ActivityState::AwaitingPermission);
        assert_eq!(
            v.to_session_status(),
            Some(daruda_claude::SessionStatus::NeedsAttention)
        );
    });

    // 4) Idle: no turn, no permission, only a top-level or completed tool.
    view.update(cx, |v, _| {
        v.set_turn_idle();
        v.pending_permissions.clear();
        v.items = vec![
            child(ToolStatusView::Completed),
            ChatItem::ToolCall(ToolCallItem {
                id: "top".into(),
                title: "top".into(),
                kind: ToolKindView::Read,
                status: ToolStatusView::InProgress,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
                parent_tool_id: None,
            }),
        ];
    });
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.activity_state(),
            ActivityState::Idle,
            "a completed child and a top-level running tool are not background activity"
        );
        assert_eq!(
            v.to_session_status(),
            Some(daruda_claude::SessionStatus::Idle)
        );
    });
}

#[gpui::test]
async fn agent_chat_view_finds_a_pane_parked_in_an_inactive_lane(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
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
            assert!(
                ws.agent_chat_view(id).is_some(),
                "found while live in the active lane"
            );

            // Simulate a lane switch: move the pane into a different lane's
            // runtime while `self.active` points elsewhere, so the lookup must
            // scan every runtime, not just the active one.
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
/// the lazy-connect state) must be buffered — never silently dropped — without
/// being echoed into the transcript, and with no turn marked in flight (nothing
/// is on the wire yet). Buffering preserves submission order.
#[gpui::test]
async fn prompt_before_connect_is_buffered_not_dropped(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                // A pane with a cwd parks in `Idle` with `handle: None`; the
                // session connects lazily on first focus, which this test never
                // triggers, so no adapter is spawned.
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
        // Queued prompts are not echoed — they live only in the queue.
        assert!(view.items.is_empty(), "queued prompts are not echoed");
        // Buffered in FIFO order rather than dropped.
        assert_eq!(
            queue_texts(view),
            vec!["first".to_string(), "second".to_string()],
            "disconnected prompts are queued in submission order"
        );
        // Nothing is on the wire, so no turn is in flight.
        assert!(
            view.turn_is_idle(),
            "no turn until a handle carries a prompt"
        );
        assert!(view.turn_is_idle());
    });
}

/// Stop must PRESERVE the queue, not discard it. The first Escape (turn in
/// flight) cancels the turn and parks the buffered queue; a second Escape (now
/// idle, with a parked queue) clears it; a third Escape (idle, empty) is a
/// no-op that propagates. (Send is not gated on `turn.is_in_flight()`, so a
/// queue-behind-a-turn state is reachable in normal use.)
#[gpui::test]
async fn escape_parks_the_queue_then_a_second_escape_clears_it(cx: &mut TestAppContext) {
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
                // Disconnected (no handle): every prompt buffers, none is on the
                // wire. Buffer several so there is a real queue behind Stop.
                ws.send_agent_prompt_text(id, "a".to_string(), cx);
                ws.send_agent_prompt_text(id, "b".to_string(), cx);
                ws.send_agent_prompt_text(id, "c".to_string(), cx);
                // Simulate a live turn so the first Escape treats the pane as busy.
                agent_view(ws, id).update(cx, |v, _| v.set_turn_in_flight());
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    view.read_with(cx, |v, _| {
        assert_eq!(
            queue_texts(v),
            vec!["a", "b", "c"],
            "three queued behind the turn"
        );
    });

    // First Escape: busy → cancels the turn and parks the queue.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(
                ws.cancel_agent_turn_if_active(pane_id, cx),
                "first Escape handles the key (cancels the running turn)"
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(v.turn_is_idle(), "the turn is settled");
        assert!(
            v.pending_prompts.is_empty(),
            "the live queue is emptied by the park"
        );
        assert_eq!(
            v.paused_prompts
                .iter()
                .map(|q| q.text.clone())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "Stop parks the queue instead of dropping it"
        );
    });

    // Second Escape: idle with a parked queue → clears it, still handled.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(
                ws.cancel_agent_turn_if_active(pane_id, cx),
                "second Escape handles the key (clears the parked queue)"
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(v.paused_prompts.is_empty(), "the parked queue is cleared");
    });

    // Third Escape: idle, no queue → not handled, so Escape propagates.
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(
                !ws.cancel_agent_turn_if_active(pane_id, cx),
                "a third Escape with no queue is a no-op and propagates"
            );
        });
    })
    .unwrap();
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
            queue_texts(v),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "disconnected prompts buffer in submission (FIFO) order, none dropped"
        );
        assert!(v.items.is_empty(), "queued prompts are not echoed");
        assert!(
            v.turn_is_idle(),
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
            queue_texts(v),
            vec!["queued".to_string()],
            "no handle → pump leaves the buffer intact"
        );
        assert!(v.turn_is_idle(), "no handle → no turn started");
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
        assert!(v.turn_is_idle());
    });
}

/// The queue drains FIFO at send time: the front item leaves the queue, is
/// echoed into the transcript as `UserText`, and starts the tracked turn. The
/// live ACP handle is private to `daruda_acp`, so this exercises the shared
/// model transition that `pump_pending_prompt` runs immediately before sending
/// the returned text over the handle.
#[gpui::test]
async fn queued_prompt_drain_echoes_front_item_and_preserves_fifo(cx: &mut TestAppContext) {
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
                ws.send_agent_prompt_text(id, "first".to_string(), cx);
                ws.send_agent_prompt_text(id, "second".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let view = workspace.read_with(cx, |ws, _| agent_view(ws, pane_id));
    let would_send =
        cx.update(|cx| view.update(cx, |v, cx| v.drain_next_queued_prompt_for_test(cx)));

    assert_eq!(would_send, Some("first".to_string()));
    view.read_with(cx, |v, _| {
        assert_eq!(
            queue_texts(v),
            vec!["second".to_string()],
            "only the drained front prompt leaves the queue"
        );
        assert_eq!(
            v.items,
            vec![daruda_acp::ChatItem::UserText("first".to_string())],
            "the drained prompt is echoed at send time"
        );
        assert!(
            !v.turn_is_idle(),
            "draining a queued prompt starts a tracked turn"
        );
    });
}

#[gpui::test]
async fn queued_prompt_ops_remove_one_and_clear_all(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let (pane_id, remove_id) = cx
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
                ws.send_agent_prompt_text(id, "first".to_string(), cx);
                ws.send_agent_prompt_text(id, "second".to_string(), cx);
                ws.send_agent_prompt_text(id, "third".to_string(), cx);
                let remove_id = agent_view(ws, id).read(cx).pending_prompts[1].id;
                (id, remove_id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| ws.remove_queued_prompt(pane_id, remove_id, cx));
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert_eq!(
            queue_texts(view),
            vec!["first".to_string(), "third".to_string()],
            "remove_queued_prompt removes only the matching id"
        );
        assert!(
            view.items.is_empty(),
            "removing from the queue does not echo"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.clear_queued_prompts(pane_id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        let view = view.read(cx);
        assert!(
            view.pending_prompts.is_empty(),
            "clear removes every prompt"
        );
        assert!(view.items.is_empty(), "clearing the queue does not echo");
    });
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
    use crate::workspace::main_area::pane_input_ops::PaneTextInput;

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

/// A non-default `codex` id. Used by the restore-wiring tests below to prove the
/// persisted `agent_id` is threaded through `restore_from_disk` (not just the
/// pure `resolve_restored_agent` seam covered by unit tests).
fn codex_agent() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
    }
}

/// Persist an AgentChat pane owned by a non-default agent (`codex`) with a live
/// session id, then restore into a workspace whose catalog still lists `codex`.
/// End-to-end wiring guard: `restore_from_disk` must relaunch under `codex` AND
/// keep the session id (the owning agent is present, so resume is valid). The
/// pane sits in a non-active tab so the restore's focus never triggers a
/// connect — the suite stays offline.
#[gpui::test]
async fn agent_chat_agent_id_survives_restore_when_agent_present(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join("daruda_agent_chat_restore_agent_present");
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
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
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .cloned()
            .expect("restored agent chat pane present");
        let view = view.read(cx);
        assert_eq!(
            view.agent_id, "codex",
            "the owning agent is still in the catalog, so the pane relaunches under it"
        );
        assert_eq!(
            view.session_id.as_deref(),
            Some("sess-codex-1"),
            "session id is kept when the owning agent is present (resume valid)"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

/// Persist a pane owned by `codex` + a session id, then restore into a workspace
/// whose catalog no longer lists `codex` (the user removed it from config). The
/// restore must fall back to the default agent AND drop the session id — a
/// session created by an absent agent cannot resume against a different one.
#[gpui::test]
async fn agent_chat_removed_agent_drops_session_on_restore(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join("daruda_agent_chat_restore_agent_removed");
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    let (window_handle, workspace) = super::build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (workspace_state, project_states) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
                let pane = ws.create_agent_chat_pane(
                    cwd,
                    Some("sess-codex-2".to_string()),
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

    // Restore into a fresh workspace whose catalog is the default (claude only) —
    // `codex` was removed. `Workspace::new_with_project_for_test` seeds `agents`
    // from `config`, so no override is needed.
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

    restored.read_with(cx, |ws, cx| {
        let default_id = daruda_config::AgentDefinition::claude_default().id;
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.agent_chat_view())
            .cloned()
            .expect("restored agent chat pane present");
        let view = view.read(cx);
        assert_eq!(
            view.agent_id, default_id,
            "the owning agent is gone, so the pane falls back to the default agent"
        );
        assert_eq!(
            view.session_id, None,
            "session id is dropped when its owning agent is absent (resume invalid)"
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
                            capabilities: Default::default(),
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

/// A `session/prompt` that returns a JSON-RPC error (adapter usage / session
/// limit → `-32603`) arrives as `AcpEvent::TurnFailed`, NOT the terminal
/// `Error`: the ACP session stays alive. The failure shows inline, the turn
/// settles, but `status` stays `Connected` so the user can re-prompt on the same
/// session once the limit resets — the fix for the "-32603 then stuck" report.
#[gpui::test]
async fn turn_failed_keeps_session_connected_and_shows_error(cx: &mut TestAppContext) {
    use daruda_acp::{AcpEvent, ChatItem};

    let (window_handle, workspace) = build_workspace(cx);
    let tmp = std::env::temp_dir();

    cx.update_window(window_handle.into(), |_, window, cx| {
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

            view.update(cx, |v, cx| {
                // A live, working session: connected with a prompt turn in flight.
                v.status = AgentSessionStatus::Connected;
                v.set_turn_in_flight();

                v.apply_event(
                    AcpEvent::TurnFailed("session limit reached".into()),
                    "",
                    false,
                    cx,
                );

                // The session is NOT torn down — it stays Connected and usable.
                assert!(
                    matches!(v.status, AgentSessionStatus::Connected),
                    "a per-turn failure must not kill the session, got {:?}",
                    v.status
                );
                // The failed turn settles so the working indicator stops.
                assert!(v.turn_is_idle(), "the failed turn must settle to idle");
                // The failure shows inline in the conversation.
                assert_eq!(
                    v.items.last(),
                    Some(&ChatItem::Error("session limit reached".to_string())),
                    "the failure is surfaced as an inline Error item"
                );
                // Recorded as an errored outcome (fires at the busy→idle edge).
                assert_eq!(v.pending_completion, Some(TurnOutcome::Errored));
            });
        })
    })
    .unwrap();
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
                Some(PaneCwd::Local(tmp.clone())),
                Some("abc".to_string()),
                daruda_config::AgentDefinition::claude_default().id,
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
                v.set_turn_in_flight();
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
            assert!(v.turn_is_idle(), "reset clears the in-flight turn flag");
            assert!(v.plan.is_empty(), "reset clears the execution plan");
            assert!(
                v.fold.is_expanded(&FoldKey::Tool("call-1".into()), true),
                "reset drops fold overrides back to the natural default"
            );
        })
    })
    .unwrap();
}

/// A second agent entry so open/switch/split tests have a non-default agent to
/// pick. `claude_default()` is catalog[0] (the session default); `codex` is the
/// distinct target.
fn codex() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
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
/// conversation, never a reuse of the existing session.
#[gpui::test]
async fn switch_agent_preserves_the_source_pane(cx: &mut TestAppContext) {
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
        });
    })
    .unwrap();
}

/// Fix regression guard: splitting an agent-chat pane inherits the *source*
/// pane's agent, so splitting a `codex` chat opens another `codex` pane — never
/// silently resets to the catalog default (`claude`).
#[gpui::test]
async fn split_agent_chat_inherits_source_agent(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::{PaneLayout, SplitDirection};
    use crate::workspace::main_area::tab_ops::NewPaneKind;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            // Two-entry catalog where the default (catalog[0]) is claude, so a
            // reset-to-default bug would produce `claude`, not `codex`.
            ws.agents = vec![daruda_config::AgentDefinition::claude_default(), codex()];

            // A source agent-chat pane under the non-default agent, wired into a
            // focused tab exactly as the split path expects (`insert_split_at`
            // matches the focused pane in a tab layout). Set the focused id
            // directly rather than calling `focus_pane`, which would lazily
            // connect the source (real cwd → Idle → adapter spawn).
            let src = ws.create_agent_chat_pane(
                Some(PaneCwd::Local(tmp.clone())),
                None,
                "codex".to_string(),
                None,
                window,
                cx,
            );
            let src_id = src.id;
            let tab_id = ws.alloc_id();
            ws.active_runtime_mut().panes.push(src);
            ws.active_runtime_mut().tabs.push(TabEntry {
                id: tab_id,
                layout: PaneLayout::Pane(src_id),
                last_focused_pane: src_id,
                user_label: None,
            });
            let ti = ws.active_runtime().tabs.len() - 1;
            ws.active_runtime_mut().active_tab_index = ti;
            ws.active_runtime_mut().focused_pane_id = src_id;

            // Drive the real split entry point the Cmd+D shortcut uses for an
            // agent-chat pane. (No active lane → the new pane's cwd is None → it
            // parks in `Error`, so its lazy connect is skipped: offline.)
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );

            let new_id = ws
                .active_runtime()
                .panes
                .last()
                .expect("the split pushed a pane")
                .id;
            assert_ne!(new_id, src_id, "the split created a distinct pane");
            assert_eq!(
                agent_view(ws, new_id).read(cx).agent_id,
                "codex",
                "the split inherits the source pane's agent, not the catalog default"
            );
        });
    })
    .unwrap();
}

/// Build an offline AgentChat view (a cwd so it parks `Idle` with `handle:
/// None`, no adapter spawned) and return its entity so the activity-span edge
/// logic can be driven directly.
fn make_activity_view(
    cx: &mut TestAppContext,
    window_handle: gpui::WindowHandle<gpui_component::Root>,
    workspace: &Entity<Workspace>,
) -> Entity<AgentChatView> {
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
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| agent_view(ws, pane_id))
}

/// The idle→busy edge stamps the activity-span start: `reconcile_activity`
/// returns `None` (no completion on the way *in*) and `activity_elapsed`
/// flips from `None` to `Some`.
#[gpui::test]
async fn reconcile_activity_idle_to_busy_stamps_span_start(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, _| {
        assert!(v.activity_elapsed().is_none(), "idle → no span");
        // Turn in flight makes `is_busy()` true without a live handle.
        v.set_turn_in_flight();
        let edge = v.reconcile_activity(std::time::Instant::now());
        assert_eq!(edge, None, "the busy edge fires no completion");
        assert!(v.was_busy, "reconcile records the busy level");
        assert!(
            v.activity_elapsed().is_some(),
            "the span start is stamped on idle→busy"
        );
    });
}

/// The busy→idle edge returns the stashed outcome once and clears the span
/// start (so `activity_elapsed` goes back to `None`).
#[gpui::test]
async fn reconcile_activity_busy_to_idle_returns_pending_once(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, _| {
        // Enter the span.
        v.set_turn_in_flight();
        assert_eq!(v.reconcile_activity(std::time::Instant::now()), None);
        // A completion was captured while busy (as `TurnEnded` would), and the
        // turn settled.
        v.pending_completion = Some(TurnOutcome::Completed);
        v.set_turn_idle();
        let edge = v.reconcile_activity(std::time::Instant::now());
        assert_eq!(
            edge,
            Some(TurnOutcome::Completed),
            "the busy→idle edge returns the stashed outcome"
        );
        assert!(
            v.activity_elapsed().is_none(),
            "the span start clears on settle"
        );
        // A second reconcile while still idle is a no-op — no re-fire.
        assert_eq!(
            v.reconcile_activity(std::time::Instant::now()),
            None,
            "the outcome fires exactly once"
        );
    });
}

/// A busy→idle settle with nothing captured returns `None` (no phantom
/// completion when the pane just goes quiet without a turn/session ending).
#[gpui::test]
async fn reconcile_activity_settle_without_pending_returns_none(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, _| {
        v.set_turn_in_flight();
        assert_eq!(v.reconcile_activity(std::time::Instant::now()), None);
        // Go idle with no captured outcome.
        v.set_turn_idle();
        assert_eq!(
            v.reconcile_activity(std::time::Instant::now()),
            None,
            "a settle with no pending outcome fires nothing"
        );
    });
}

/// Two turns that never let the pane fall idle between them are one activity
/// span: the outcome is overwritten to the latest and fires exactly once, at
/// the final settle.
#[gpui::test]
async fn reconcile_activity_two_turns_one_span_fires_latest_once(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, _| {
        // Turn 1 starts → span begins.
        v.set_turn_in_flight();
        assert_eq!(v.reconcile_activity(std::time::Instant::now()), None);

        // Turn 1 ends and turn 2 starts before any idle reconcile: the pane
        // never leaves the busy level, so no edge — the first outcome is
        // captured but not fired.
        v.pending_completion = Some(TurnOutcome::Stopped);
        v.set_turn_in_flight();
        assert_eq!(
            v.reconcile_activity(std::time::Instant::now()),
            None,
            "still busy across the turn boundary → no settle edge"
        );

        // Turn 2 ends → the latest outcome overwrites, then the pane settles.
        v.pending_completion = Some(TurnOutcome::Completed);
        v.set_turn_idle();
        assert_eq!(
            v.reconcile_activity(std::time::Instant::now()),
            Some(TurnOutcome::Completed),
            "the span fires the latest outcome exactly once at the final settle"
        );
    });
}

/// Regression: at connect the buffered first prompt is pumped
/// (turn Idle→InFlight) and the connect path now reconciles immediately,
/// stamping `was_busy`. So when the very first ACP event on the stream is
/// `TurnEnded` (turn → Idle, outcome stashed), the busy→idle edge is still
/// detected and the completion fires — instead of being stranded forever (task
/// stuck `Running`, no notification) because `was_busy` never became `true`.
/// Drives the view-level sequence the connect callback performs; the connect
/// path itself needs a live adapter subprocess, so it is not exercised here.
#[gpui::test]
async fn connect_time_pump_then_turn_end_fires_completion(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, _| {
        // Connect pumps the buffered prompt → turn in flight.
        v.set_turn_in_flight();
        // The connect path reconciles right after the pump, stamping the
        // busy level so a later settle edge is detectable.
        assert_eq!(v.reconcile_activity(std::time::Instant::now()), None);
        assert!(
            v.was_busy,
            "the connect-time reconcile stamps the busy level"
        );

        // The first ACP event is `TurnEnded`: settle + stash the outcome.
        v.pending_completion = Some(TurnOutcome::Completed);
        v.set_turn_idle();
        // The busy→idle edge is detected (not stranded) and fires exactly once.
        assert_eq!(
            v.reconcile_activity(std::time::Instant::now()),
            Some(TurnOutcome::Completed),
            "the completion fires at the settle edge instead of being stranded"
        );
    });
}

/// Regression: `cancel_turn` must not clobber an already-captured
/// completion. Stop is offered (gated on `is_busy()`) during the trailing-
/// subagent window even after the foreground turn ended normally and stashed
/// `Completed`; stopping then must leave that outcome intact rather than
/// overwriting it with `Stopped` (which would drop the completion signal). A
/// real in-flight turn still stashes `Stopped`.
#[gpui::test]
async fn cancel_turn_preserves_completion_when_no_turn_in_flight(cx: &mut TestAppContext) {
    use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    let running_child = || {
        ChatItem::ToolCall(ToolCallItem {
            id: "child".into(),
            title: "child".into(),
            kind: ToolKindView::Read,
            status: ToolStatusView::InProgress,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: Some("parent".into()),
        })
    };

    // Case 1: the foreground turn already ended normally (Completed stashed) but
    // a background subagent's child tool is still running, so the pane is busy
    // and the dock still offers Stop. Stopping keeps the captured Completed.
    view.update(cx, |v, cx| {
        v.set_turn_idle();
        v.items = vec![running_child()];
        v.pending_completion = Some(TurnOutcome::Completed);
        assert!(v.is_busy(), "a running child subagent keeps the pane busy");

        v.cancel_turn(cx);
        assert_eq!(
            v.pending_completion,
            Some(TurnOutcome::Completed),
            "Stop with no foreground turn keeps the already-captured completion"
        );
    });

    // Case 2: a real in-flight foreground turn — Stop settles it locally and
    // stashes `Stopped` (fired at the settle edge), overwriting any prior capture.
    view.update(cx, |v, cx| {
        v.pending_completion = None;
        v.set_turn_in_flight();
        v.cancel_turn(cx);
        assert!(v.turn_is_idle(), "Stop settles the live turn locally");
        assert_eq!(
            v.pending_completion,
            Some(TurnOutcome::Stopped),
            "Stop of a live turn stashes Stopped"
        );
    });
}

/// Regression for the Stop-then-reprompt(-then-Stop) race. Stop settles the turn
/// locally (responsive + hung-safe) and opens the cancel window; a re-prompt then
/// buffers **client-side** (not raced onto the wire), so it can't be
/// misattributed to the cancelled turn's ack. A *second* Stop then parks that
/// re-prompt (queue-preserving Stop) instead of dropping it — still client-side,
/// so the ack still can't misattribute it. The cancel's `TurnEnded` closes the
/// window without draining (the parked queue does not auto-resume).
#[gpui::test]
async fn stop_buffers_reprompt_and_second_stop_parks_it(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, cx| {
        // Turn-1 in flight, then Stop: settle locally + open the cancel window.
        v.set_turn_in_flight();
        v.cancel_turn(cx);
        assert!(
            v.turn_is_idle(),
            "Stop settles the turn locally (hung-safe)"
        );
        assert!(
            v.cancel_in_flight,
            "the cancel window stays open until the ack"
        );
        assert_eq!(
            v.pending_completion,
            Some(TurnOutcome::Stopped),
            "the cancelled turn's Stopped is stashed to fire at the settle edge"
        );

        // A re-prompt during the cancel window buffers client-side — NOT put on
        // the wire (turn stays idle), so it can't race the cancel's ack.
        v.send_prompt_text("again".into(), cx);
        assert!(
            v.turn_is_idle(),
            "the re-prompt is buffered, not raced onto the wire, during cancel"
        );
        assert_eq!(
            queue_texts(v),
            vec!["again".to_string()],
            "the re-prompt buffers client-side"
        );

        // A SECOND Stop parks the buffered re-prompt (queue-preserving Stop):
        // moved out of the live queue into the parked queue, still client-side
        // (never raced onto the wire), so a stale ack still can't misattribute
        // it. It stays resumable rather than being silently dropped.
        v.cancel_turn(cx);
        assert!(
            v.pending_prompts.is_empty(),
            "the second Stop empties the live queue"
        );
        assert_eq!(
            v.paused_prompts
                .iter()
                .map(|q| q.text.clone())
                .collect::<Vec<_>>(),
            vec!["again".to_string()],
            "the re-prompt is parked (preserved), not dropped"
        );

        // The cancel's `TurnEnded` ack closes the window. The parked queue does
        // not auto-drain, so nothing runs; it neither re-settles nor re-completes.
        v.apply_event(
            daruda_acp::AcpEvent::TurnEnded {
                completed_normally: false,
                stop_reason: "Cancelled".into(),
            },
            "",
            false,
            cx,
        );
        assert!(!v.cancel_in_flight, "the ack closes the cancel window");
        assert!(v.turn_is_idle(), "nothing left to run after both Stops");
    });
}

/// Editing a queued prompt replaces that slot's text in place — order preserved,
/// the editing flag cleared, and nothing echoed into the transcript (an edit is
/// not a new turn).
#[gpui::test]
async fn send_prompt_text_editing_replaces_slot_in_place(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, cx| {
        v.send_prompt_text("first".into(), cx);
        v.send_prompt_text("second".into(), cx);
        v.send_prompt_text("third".into(), cx);
        let middle = v.pending_prompts[1].id;
        v.begin_edit(middle, cx);
        assert_eq!(
            v.editing_prompt,
            Some(middle),
            "begin_edit records the target"
        );

        v.send_prompt_text("edited".into(), cx);
        assert_eq!(
            queue_texts(v),
            vec![
                "first".to_string(),
                "edited".to_string(),
                "third".to_string()
            ],
            "editing replaces the slot in place, order preserved"
        );
        assert!(v.editing_prompt.is_none(), "send clears the editing flag");
        assert!(v.items.is_empty(), "an in-place edit does not echo");
    });
}

/// When the edit target is no longer queued (drained onto the wire while the
/// user was editing), a send falls through to a brand-new queued prompt rather
/// than dropping the typed text.
#[gpui::test]
async fn send_prompt_text_editing_target_gone_falls_through_to_new(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let view = make_activity_view(cx, window_handle, &workspace);

    view.update(cx, |v, cx| {
        v.send_prompt_text("only".into(), cx);
        let id = v.pending_prompts[0].id;
        v.begin_edit(id, cx);
        // Model the target leaving the queue while the composer still targets it.
        v.remove_queued(id, cx);
        assert_eq!(
            v.editing_prompt,
            Some(id),
            "removing a row does not by itself clear the editing flag"
        );
        assert!(v.pending_prompts.is_empty());

        v.send_prompt_text("typed".into(), cx);
        assert_eq!(
            queue_texts(v),
            vec!["typed".to_string()],
            "a drained edit target falls through to a new queued prompt"
        );
        assert!(
            v.editing_prompt.is_none(),
            "the stale editing flag is cleared on send"
        );
        assert!(
            v.items.is_empty(),
            "no handle → the new prompt is queued, not echoed"
        );
    });
}

/// ↑ in an EMPTY composer, with a non-empty queue on the focused agent pane,
/// pulls the most-recent queued prompt into the composer for editing.
#[gpui::test]
async fn up_arrow_in_empty_composer_edits_last_queued_prompt(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();
    let tmp = std::env::temp_dir();

    let (pane_id, last_id) = cx
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
                ws.send_agent_prompt_text(id, "q1".into(), cx);
                ws.send_agent_prompt_text(id, "q2".into(), cx);
                let last = agent_view(ws, id)
                    .read(cx)
                    .pending_prompts
                    .last()
                    .expect("queue non-empty")
                    .id;
                ws.terminal_input
                    .update(cx, |s, cx_state| s.set_value("", window, cx_state));
                (id, last)
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
            "q2",
            "the composer receives the last queued prompt's text"
        );
        assert_eq!(
            agent_view(ws, pane_id).read(cx).editing_prompt,
            Some(last_id),
            "the editing flag targets the last queued prompt"
        );
    });
}

/// The ↑ consume-predicate `history_navigate_possible`: a non-empty queue on the
/// focused agent pane makes ↑ consumable even with no lane input history (so the
/// key reaches `do_history_navigate` and begins the edit); an empty queue with no
/// history does not consume ↑, and Down is never affected by the queue.
#[gpui::test]
async fn history_navigate_possible_consumes_up_for_queue_without_history(cx: &mut TestAppContext) {
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
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.send_agent_prompt_text(pane_id, "q1".into(), cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert!(
            ws.history_navigate_possible(crate::ui::HistoryDir::Up, cx),
            "a queued prompt makes ↑ consumable even without input history"
        );
        assert!(
            !ws.history_navigate_possible(crate::ui::HistoryDir::Down, cx),
            "Down is unaffected by the queue"
        );
    });
}

/// ↑ with a NON-empty composer does the ordinary history recall (no queue edit)
/// — with no history entries here, the composer is left untouched and no edit
/// begins.
#[gpui::test]
async fn up_arrow_with_nonempty_composer_does_not_edit_queue(cx: &mut TestAppContext) {
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
                ws.send_agent_prompt_text(id, "q1".into(), cx);
                ws.terminal_input
                    .update(cx, |s, cx_state| s.set_value("typing", window, cx_state));
                id
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
            agent_view(ws, pane_id).read(cx).editing_prompt.is_none(),
            "no queue edit begins when the composer is non-empty"
        );
    });
}

/// Cancelling a queued-prompt edit clears the editing flag and empties the
/// composer.
#[gpui::test]
async fn cancel_edit_queued_prompt_clears_flag_and_empties_composer(cx: &mut TestAppContext) {
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
                let prompt_id = agent_view(ws, id).read(cx).pending_prompts[0].id;
                ws.begin_edit_queued_prompt(id, prompt_id, window, cx);
                (id, prompt_id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    // begin pulled the text into the composer and set the editing flag.
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.terminal_input.read(cx).value(), "editable");
        assert_eq!(agent_view(ws, pane_id).read(cx).editing_prompt, Some(id));
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
            agent_view(ws, pane_id).read(cx).editing_prompt.is_none(),
            "cancel clears the editing flag; the prompt stays queued"
        );
        assert_eq!(
            queue_texts(agent_view(ws, pane_id).read(cx)),
            vec!["editable".to_string()],
            "cancel leaves the queued prompt intact"
        );
    });
}

/// Clearing the whole queue while a prompt is being edited also empties the
/// composer — otherwise the deleted slot's text lingers as a phantom draft.
#[gpui::test]
async fn clear_queue_while_editing_empties_composer(cx: &mut TestAppContext) {
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
                ws.send_agent_prompt_text(id, "q1".into(), cx);
                ws.send_agent_prompt_text(id, "q2".into(), cx);
                let last = agent_view(ws, id)
                    .read(cx)
                    .pending_prompts
                    .last()
                    .unwrap()
                    .id;
                ws.begin_edit_queued_prompt(id, last, window, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| ws.clear_queued_prompts(pane_id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        let view = agent_view(ws, pane_id);
        assert!(view.read(cx).pending_prompts.is_empty(), "queue cleared");
        assert!(
            view.read(cx).editing_prompt.is_none(),
            "editing flag cleared"
        );
        assert_eq!(
            ws.terminal_input.read(cx).value(),
            "",
            "the orphaned edit text is cleared from the composer"
        );
    });
}

/// A whitespace-only submit while editing cancels the edit rather than stranding
/// the "Editing…" strip row against an empty body.
#[gpui::test]
async fn whitespace_submit_while_editing_cancels_edit(cx: &mut TestAppContext) {
    use crate::workspace::main_area::pane_input_ops::PaneTextInput;

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
                ws.send_agent_prompt_text(id, "editable".into(), cx);
                let pid = agent_view(ws, id).read(cx).pending_prompts[0].id;
                ws.begin_edit_queued_prompt(id, pid, window, cx);
                id
            })
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
                    submit: true,
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
            view.read(cx).editing_prompt.is_none(),
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
