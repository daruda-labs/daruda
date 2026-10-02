//! A shell that exits in a pane kept open says so, and can be started again
//! in place. A stub PTY never exits, so these drive the exit edge directly.

use super::*;
use crate::surface::strings;
use crate::workspace::main_area::pane::PaneContent;

fn exited(ws: &gpui::Entity<Workspace>, pane_id: u64, cx: &mut TestAppContext) -> bool {
    ws.read_with(cx, |ws, _| {
        let pane = ws.active_runtime().panes.iter().find(|p| p.id == pane_id);
        match pane.map(|p| &p.content) {
            Some(PaneContent::Terminal(t)) => t.has_exited(),
            _ => panic!("pane {pane_id} is not a terminal"),
        }
    })
}

#[gpui::test]
async fn an_exited_shell_is_marked_and_says_so(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    let pane_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    assert!(!exited(&ws, pane_id, cx));

    ws.update(cx, |ws, cx| ws.note_terminal_exited(pane_id, cx));
    cx.run_until_parked();
    assert!(exited(&ws, pane_id, cx));
    let screen = ws.read_with(cx, |ws, cx| {
        let view = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        view.terminal_view()
            .unwrap()
            .read(cx)
            .session()
            .dump_viewport()
            .unwrap()
    });
    assert!(
        screen.contains(&strings::terminal::process_exited()),
        "the pane tells the user the shell is gone: {screen:?}"
    );
}

#[gpui::test]
async fn restarting_an_exited_shell_keeps_the_pane(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    let pane_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    let old_view = ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap()
            .terminal_view()
            .unwrap()
            .entity_id()
    });
    ws.update(cx, |ws, cx| ws.note_terminal_exited(pane_id, cx));
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.restart_terminal_pane(pane_id, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        !exited(&ws, pane_id, cx),
        "a fresh shell runs in the same pane"
    );
    let new_view = ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap()
            .terminal_view()
            .unwrap()
            .entity_id()
    });
    assert_ne!(old_view, new_view, "on a new session");
}
