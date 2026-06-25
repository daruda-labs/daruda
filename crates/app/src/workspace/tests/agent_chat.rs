//! AgentChat pane — open action, persistence round-trip, and the pure
//! parts of the prompt / permission ops (no live ACP session required).
//!
//! `open_agent_chat_pane` produces a `PaneContent::AgentChat` leaf; the
//! layout serializer round-trips it back (the ACP session is intentionally
//! not persisted). The prompt-echo and permission-resolve ops are tested
//! against a pane built with `create_agent_chat_pane` (which does not
//! itself open a connection), so no `npx` adapter is ever spawned —
//! `handle` stays `None` and the host-side state transitions still run.

use daruda_acp::{ModeStateView, SessionModeView};
use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::workspace::main_area::pane::{AgentSessionStatus, PaneContent};

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

    workspace.read_with(cx, |ws, _| {
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
                // The default test workspace has no resolvable lane cwd,
                // so the pane parks in `Error` rather than attempting a
                // (subprocess) connection — keeps the suite offline.
                assert!(
                    matches!(ac.status, AgentSessionStatus::Error(_)),
                    "no lane cwd → error status, not a live connect, got {:?}",
                    ac.status
                );
                assert!(ac.items.is_empty(), "items start empty");
                assert!(ac.handle.is_none(), "no session without a cwd");
            }
            _ => panic!("expected an AgentChat pane"),
        }
        assert_eq!(ws.main_area.focused_pane_id, pane.id);
    });
}

#[gpui::test]
async fn send_agent_prompt_text_echoes_user_text(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                // `create_agent_chat_pane` builds the pane but does not
                // open a connection — that is the caller's job — so this
                // never spawns an adapter. Push it directly into the tree.
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
                // The prompt arrives from the shared bottom-dock input via
                // `send_agent_prompt_text` (the pane no longer owns an input).
                ws.send_agent_prompt_text(id, "hello agent".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let ac = ws
            .main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.agent_chat_content())
            .expect("agent chat pane present");
        assert_eq!(
            ac.items.len(),
            1,
            "the submitted prompt is echoed as one UserText item"
        );
        assert_eq!(
            ac.items[0],
            daruda_acp::ChatItem::UserText("hello agent".to_string())
        );
        // No live handle → the turn is not marked in flight.
        assert!(!ac.turn_in_flight);
    });
}

#[gpui::test]
async fn respond_agent_permission_resolves_the_pending_card(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
                // Inject a pending permission card + its pending id, as the
                // event pump would have on a `PermissionRequested` event.
                if let Some(ac) = ws
                    .main_area
                    .panes
                    .iter_mut()
                    .find(|p| p.id == id)
                    .and_then(|p| p.agent_chat_content_mut())
                {
                    ac.items.push(ChatItem::Permission(PermissionItem {
                        tool_title: Some("Write /tmp/x".to_string()),
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    ac.pending_permission = Some(42);
                }
                ws.respond_agent_permission(
                    id,
                    "allow_once".to_string(),
                    PermissionKindView::AllowOnce,
                    cx,
                );
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let ac = ws
            .main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.agent_chat_content())
            .expect("agent chat pane present");
        let ChatItem::Permission(card) = &ac.items[0] else {
            panic!("expected a permission card");
        };
        assert_eq!(
            card.resolved,
            Some(PermissionResolution::Chosen("allow_once".to_string())),
            "the chosen option is recorded on the card"
        );
        assert!(
            ac.pending_permission.is_none(),
            "the pending id is cleared once resolved"
        );
    });
}

#[gpui::test]
async fn agent_chat_pane_without_cwd_carries_reason_not_prefix(cx: &mut TestAppContext) {
    use crate::surface::strings as s;
    use crate::workspace::main_area::pane::AgentSessionStatus;

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    // No resolvable lane cwd → the pane parks in `Error`. The message must be
    // the bare reason: the status banner re-adds the error prefix, so storing
    // the prefix here would render it doubled.
    let status = cx
        .update_window(window_handle.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(None, cx);
                match &pane.content {
                    PaneContent::AgentChat(ac) => ac.status.clone(),
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

/// `set_agent_mode` immediately updates `modes.current` (optimistic update) and
/// is idempotent when the handle is absent (no live ACP session required).
#[gpui::test]
async fn set_agent_mode_updates_current_optimistically(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                let mut pane = ws.create_agent_chat_pane(Some(tmp.clone()), cx);
                // Inject a ModeStateView with two modes so `set_agent_mode` has
                // something to flip. No live handle (handle stays `None`).
                if let PaneContent::AgentChat(ref mut ac) = pane.content {
                    ac.modes = Some(ModeStateView {
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
                }
                let id = pane.id;
                ws.main_area.panes.push(pane);
                ws.set_agent_mode(id, "plan".to_string(), cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let ac = ws
            .main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.agent_chat_content())
            .expect("agent chat pane present");
        let modes = ac.modes.as_ref().expect("modes were injected");
        assert_eq!(
            modes.current, "plan",
            "set_agent_mode flips current immediately (optimistic)"
        );
    });
}

#[gpui::test]
async fn cancel_agent_turn_cancels_the_pending_permission(cx: &mut TestAppContext) {
    use daruda_acp::{
        ChatItem, PermissionChoice, PermissionItem, PermissionKindView, PermissionResolution,
    };

    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let tmp = std::env::temp_dir();
    let pane_id = cx
        .update_window(window_handle.into(), |_, _window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(Some(tmp.clone()), cx);
                let id = pane.id;
                ws.main_area.panes.push(pane);
                // Inject a pending permission card + its pending id, as the
                // event pump would have on a `PermissionRequested` event.
                if let Some(ac) = ws
                    .main_area
                    .panes
                    .iter_mut()
                    .find(|p| p.id == id)
                    .and_then(|p| p.agent_chat_content_mut())
                {
                    ac.items.push(ChatItem::Permission(PermissionItem {
                        tool_title: Some("Write /tmp/x".to_string()),
                        options: vec![PermissionChoice {
                            option_id: "allow_once".to_string(),
                            name: "Allow".to_string(),
                            kind: PermissionKindView::AllowOnce,
                        }],
                        resolved: None,
                    }));
                    ac.pending_permission = Some(7);
                }
                // No live handle (offline) — cancel still drains the pending
                // permission host-side: the card resolves to `Cancelled` and
                // the pending id clears, so no card is left with live buttons.
                ws.cancel_agent_turn(id, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let ac = ws
            .main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.agent_chat_content())
            .expect("agent chat pane present");
        let ChatItem::Permission(card) = &ac.items[0] else {
            panic!("expected a permission card");
        };
        assert_eq!(
            card.resolved,
            Some(PermissionResolution::Cancelled),
            "cancelling the turn marks the pending card cancelled"
        );
        assert!(
            ac.pending_permission.is_none(),
            "the pending id is cleared on cancel"
        );
    });
}
