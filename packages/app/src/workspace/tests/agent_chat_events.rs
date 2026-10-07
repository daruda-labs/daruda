//! The chat → host channel. An event a chat emits reaches the Workspace that
//! built it exactly once, whether the chat is a tab's or the orchestrator's —
//! the orchestrator keeps the view but drops the pane it was built in, and a
//! revealed orchestrator tab wraps that same view again.

use daruda_store::accounts::AccountSelection;
use daruda_store::observability::error_report::ErrorReport;
use gpui::{AppContext as _, Context, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatEvent;
use crate::workspace::main_area::pane_tree::PaneId;

fn emit_report(ws: &Workspace, pane: PaneId, title: &'static str, cx: &mut Context<Workspace>) {
    let view = ws.agent_chat_view(pane).expect("a chat pane").clone();
    view.update(cx, |_, cx| {
        cx.emit(AgentChatEvent::ReportError(ErrorReport::new(title).build()));
    });
}

fn times_reported(ws: &Workspace, title: &str) -> usize {
    ws.error_history()
        .iter()
        .filter(|r| r.title == title)
        .count()
}

#[gpui::test]
fn a_tab_chat_reaches_its_host_once(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            let pane = ws.active_runtime().panes.last().expect("pane").id;
            emit_report(ws, pane, "from a tab chat", cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(times_reported(ws, "from a tab chat"), 1)
    });
}

#[gpui::test]
fn the_orchestrator_chat_reaches_its_host_once_however_often_it_is_shown(cx: &mut TestAppContext) {
    let (window, workspace) = build_workspace(cx);
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws
                .seed_orchestrator_chat_pane_unrevealed_for_test(
                    ws.agents[0].id.clone(),
                    std::env::temp_dir(),
                    AccountSelection::SystemDefault,
                    None,
                    window,
                    cx,
                )
                .expect("slot seeded");
            emit_report(ws, pane, "unrevealed", cx);
            // Reveal, hide and reveal again: each reveal wraps the same view
            // in a new tab, which must not add a second listener.
            ws.toggle_orchestrator_tab(window, cx);
            ws.toggle_orchestrator_tab(window, cx);
            ws.toggle_orchestrator_tab(window, cx);
            emit_report(ws, pane, "revealed", cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(times_reported(ws, "unrevealed"), 1);
        assert_eq!(times_reported(ws, "revealed"), 1);
    });
}
