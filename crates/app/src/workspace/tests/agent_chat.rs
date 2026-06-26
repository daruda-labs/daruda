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
use crate::workspace::main_area::agent_chat_pane::view::{AgentChatView, AgentSessionStatus};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Fetch the `AgentChatView` entity for `pane_id` (panics if the pane is gone
/// or is not an AgentChat pane).
fn agent_view(ws: &Workspace, pane_id: PaneId) -> Entity<AgentChatView> {
    ws.main_area
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

    let tabs_before = workspace.read_with(cx, |ws, _| ws.main_area.tabs.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.main_area.tabs.len(),
            tabs_before + 1,
            "opening an agent chat pane appends a tab"
        );
        let pane = ws
            .main_area
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
        assert_eq!(ws.main_area.focused_pane_id, pane.id);
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
                // Each prompt echoes one `UserText` item (no live handle, so no
                // turn) and routes through `send_prompt_text` → `sync_list_after`.
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
        assert_eq!(
            view.list_state.item_count(),
            view.items.len(),
            "the virtualized list count must track items exactly"
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
        ws.main_area
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
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
                let pane = ws.create_agent_chat_pane(None, window, cx);
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
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
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), window, cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
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
