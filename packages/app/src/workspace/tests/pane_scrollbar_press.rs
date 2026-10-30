//! A press on a pane's scrollbar strip is still a press inside the pane.
//!
//! The strip blocks hover for everything behind it so content under it never
//! reacts, and the pane wrapper is behind it too — so the wrapper's ordinary
//! "a left press activates this pane" handler cannot see the press.

use gpui::{Modifiers, VisualTestContext};

use super::*;
use crate::workspace::main_area::pane::PaneContent;

fn assistant(text: String) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text,
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

#[gpui::test]
async fn a_scrollbar_press_activates_its_pane(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    let terminal = workspace.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    let chat = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.split_focused_pane_kind(
                    NewPaneKind::AgentChat,
                    SplitDirection::Horizontal,
                    window,
                    cx,
                );
                let chat = ws
                    .active_runtime()
                    .panes
                    .iter()
                    .find(|p| matches!(p.content, PaneContent::AgentChat(_)))
                    .map(|p| p.id)
                    .expect("the split produced an agent chat pane");
                let view = ws.agent_chat_view(chat).cloned().expect("chat view");
                // Enough replies to overflow the transcript and draw its thumb.
                view.update(cx, |v, cx| {
                    v.seed_items_for_test((0..80).map(|i| assistant(format!("reply {i}"))), cx);
                });
                ws.focus_pane_on_click(terminal, window, cx);
                chat
            })
        })
        .unwrap();
    cx.run_until_parked();

    let mut vcx = VisualTestContext::from_window(window_handle.into(), cx);
    // The chat pane is a `.cached()` view; a refresh records its debug bounds.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let selector: &'static str = format!("agent-chat-scrollbar-{chat}").leak();
    let strip = vcx
        .debug_bounds(selector)
        .expect("eighty replies overflow the transcript");
    vcx.simulate_click(strip.center(), Modifiers::default());
    vcx.run_until_parked();

    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.active_runtime().focused_pane_id,
            chat,
            "a press on the chat's scrollbar left the terminal active"
        );
    });
}
