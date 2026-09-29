//! The tab strip's status dot: each tab shows the most urgent agent session
//! among its panes.

use super::*;
use daruda_agent::SessionStatus;

fn active_tab_status(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) -> Option<SessionStatus> {
    ws.read_with(cx, |ws, cx| {
        ws.tab_cells(cx)
            .into_iter()
            .find(|cell| cell.is_active)
            .and_then(|cell| cell.status)
    })
}

#[gpui::test]
async fn an_agent_tab_carries_its_session_status(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    // No lane cwd, so the pane parks in `Error`.
    assert_eq!(active_tab_status(&ws, cx), Some(SessionStatus::Failed));

    ws.update(cx, |ws, cx| {
        let pane_id = ws.active_runtime().focused_pane_id;
        super::agent_chat::agent_view(ws, pane_id).update(cx, |v, _| {
            v.status =
                crate::workspace::main_area::agent_chat_pane::view::AgentSessionStatus::Connected;
            v.set_turn_in_flight();
        });
    });
    assert_eq!(active_tab_status(&ws, cx), Some(SessionStatus::Working));
}

#[gpui::test]
async fn a_terminal_tab_has_no_status(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    assert_eq!(active_tab_status(&ws, cx), None);
}
