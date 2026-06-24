//! AgentChat pane — open action, persistence round-trip, and the pure
//! parts of the prompt / permission ops (no live ACP session required).
//!
//! `open_agent_chat_pane` produces a `PaneContent::AgentChat` leaf; the
//! layout serializer round-trips it back (the ACP session is intentionally
//! not persisted). The prompt-echo and permission-resolve ops are tested
//! against a pane built with `create_agent_chat_pane` (which does not
//! itself open a connection), so no `npx` adapter is ever spawned —
//! `handle` stays `None` and the host-side state transitions still run.

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
    use daruda_acp::{ChatItem, PermissionChoice, PermissionItem, PermissionKindView};

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
            card.resolved.as_deref(),
            Some("allow_once"),
            "the chosen option is recorded on the card"
        );
        assert!(
            ac.pending_permission.is_none(),
            "the pending id is cleared once resolved"
        );
    });
}
