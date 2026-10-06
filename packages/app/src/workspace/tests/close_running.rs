//! Closing a pane that is still running something — an agent turn, or a job
//! in a terminal — asks first. The terminal half reads the PTY's foreground
//! process group, which a stub PTY has none of; `daruda_terminal`'s real-shell
//! test covers that read, and these drive the shared prompt through an agent
//! turn.

use super::*;
use crate::surface::strings;
use crate::workspace::main_area::pane_tree::PaneId;

fn with_agent_tab(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    PaneId,
) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    // A second tab, so closing the agent's tab does not close the window.
    for _ in 0..2 {
        cx.update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
        })
        .unwrap();
        cx.run_until_parked();
    }
    let pane_id = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    (wh, ws, pane_id)
}

fn start_turn(ws: &gpui::Entity<Workspace>, pane_id: PaneId, cx: &mut TestAppContext) {
    ws.update(cx, |ws, cx| {
        super::agent_chat::agent_view(ws, pane_id).update(cx, |v, _| v.set_turn_in_flight());
    });
}

fn has_pane(ws: &gpui::Entity<Workspace>, pane_id: PaneId, cx: &mut TestAppContext) -> bool {
    ws.read_with(cx, |ws, _| {
        ws.active_runtime().panes.iter().any(|p| p.id == pane_id)
    })
}

fn close_active_tab(
    wh: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let index = ws.active_runtime().active_tab_index;
            ws.request_close_tab(index, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
async fn closing_a_tab_whose_agent_is_idle_does_not_ask(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    close_active_tab(wh, &ws, cx);
    assert!(!cx.has_pending_prompt());
    assert!(!has_pane(&ws, pane_id, cx));
}

#[gpui::test]
async fn closing_a_tab_whose_agent_is_working_asks_first(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    start_turn(&ws, pane_id, cx);
    close_active_tab(wh, &ws, cx);
    assert!(cx.has_pending_prompt(), "stopping a turn is asked first");
    assert!(
        has_pane(&ws, pane_id, cx),
        "nothing closes before the answer"
    );

    cx.simulate_prompt_answer(&strings::common::btn_cancel());
    cx.run_until_parked();
    assert!(has_pane(&ws, pane_id, cx), "Cancel keeps it");

    close_active_tab(wh, &ws, cx);
    cx.simulate_prompt_answer(&strings::modal::close_running_confirm());
    cx.run_until_parked();
    assert!(!has_pane(&ws, pane_id, cx), "confirming closes it");
}

#[gpui::test]
async fn closing_a_pane_whose_agent_is_working_asks_first(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    start_turn(&ws, pane_id, cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_focused_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer(&strings::modal::close_running_confirm());
    cx.run_until_parked();
    assert!(!has_pane(&ws, pane_id, cx));
}

#[gpui::test]
async fn closing_the_window_while_an_agent_works_asks_first(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    start_turn(&ws, pane_id, cx);
    let may_close = cx
        .update_window(wh.into(), |_, window, cx| {
            let weak = ws.downgrade();
            Workspace::may_close_window(&weak, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(!may_close, "the window is held while the prompt is up");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer(&strings::common::btn_cancel());
    cx.run_until_parked();
    assert!(
        cx.update_window(wh.into(), |_, _, _| ()).is_ok(),
        "Cancel keeps the window"
    );
}

/// ⌘Q, the menu and the palette all reach `Quit` inside the focused
/// window's own update, where gpui has that window checked out; the sweep
/// must still ask it.
#[gpui::test]
async fn quitting_from_inside_the_window_still_asks_it(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    start_turn(&ws, pane_id, cx);
    cx.update_window(wh.into(), |_, _window, cx| Workspace::request_quit(cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "the focused window is asked too");
    assert!(
        cx.update_window(wh.into(), |_, _, _| ()).is_ok(),
        "held open"
    );

    cx.simulate_prompt_answer(&strings::modal::close_running_confirm());
    cx.run_until_parked();
    assert!(
        cx.update_window(wh.into(), |_, _, _| ()).is_err(),
        "confirming closes the window on the way out"
    );
}

/// The app-drawn close button runs as a Workspace listener, inside the
/// entity's own update; the gate it calls reads and updates that entity.
#[gpui::test]
async fn the_close_button_asks_without_re_entering_the_workspace(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    start_turn(&ws, pane_id, cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.on_close_window(&crate::workspace::CloseWindow, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        cx.has_pending_prompt(),
        "the button asks like the OS close does"
    );
}

/// The prompt is up while another tab closes; the confirm still closes the
/// tab that was asked about, not whichever now sits at its old index.
#[gpui::test]
async fn a_tab_that_moved_under_the_prompt_is_still_the_one_closed(cx: &mut TestAppContext) {
    let (wh, ws, pane_id) = with_agent_tab(cx);
    start_turn(&ws, pane_id, cx);
    let (tabs_before, left_neighbour) = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        let left = rt.tabs[rt.active_tab_index - 1].layout.pane_ids();
        (rt.tabs.len(), left)
    });
    close_active_tab(wh, &ws, cx);
    assert!(cx.has_pending_prompt());
    // A tab to the left goes away while the question is up.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let index = ws.active_runtime().active_tab_index - 1;
            ws.close_tab_at(index, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_prompt_answer(&strings::modal::close_running_confirm());
    cx.run_until_parked();
    assert!(!has_pane(&ws, pane_id, cx), "the asked-about tab closed");
    let tabs_after = ws.read_with(cx, |ws, _| ws.active_runtime().tabs.len());
    assert_eq!(
        tabs_after,
        tabs_before - 2,
        "and only it, besides the one closed by hand"
    );
    assert!(!left_neighbour.is_empty());
}
