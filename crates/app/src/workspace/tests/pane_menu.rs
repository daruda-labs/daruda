//! Regression guards for the pane context menu's focus contract.
//!
//! The menu acts on whatever `focused_pane_id` says (`split_focused_pane_kind`
//! / `toggle_zoom_pane` / `close_pane_by_id` all read it), so right-clicking a
//! non-focused pane has to move that field — and *only* that field.

use gpui::{Point, SharedString, px};

use super::*;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Split the starting pane so the tab holds two terminals, returning
/// `(first, second)`. The split leaves `second` focused.
fn two_terminals(
    window_handle: gpui::WindowHandle<gpui_component::Root>,
    workspace: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) -> (PaneId, PaneId) {
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane_kind(
                NewPaneKind::Terminal,
                SplitDirection::Horizontal,
                window,
                cx,
            );
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        let ids: Vec<PaneId> = ws.active_runtime().panes.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 2, "the split produced two panes");
        (ids[0], ids[1])
    })
}

#[gpui::test]
async fn menu_focus_tracks_right_clicked_pane_without_click_side_effects(cx: &mut TestAppContext) {
    // The terminal swallows the right-click (`cx.stop_propagation()` in
    // `view/mouse.rs`) before any ancestor handler runs, so the menu entry
    // point cannot lean on the click-to-focus path — it must move the model
    // focus itself. Without this, Split from a non-focused terminal's menu
    // splits whichever pane happened to be focused.
    let (window_handle, workspace) = build_workspace(cx);
    let (first, second) = two_terminals(window_handle, &workspace, cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.set_focused_pane(second, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, second);
            ws.terminal_input_visible = false;

            ws.open_pane_context_menu_at(first, Point::new(px(0.), px(0.)), window, cx);

            assert_eq!(
                ws.active_runtime().focused_pane_id,
                first,
                "the right-clicked terminal owns the model focus"
            );
            assert!(
                ws.main_area.popup_menu_deploy.is_some(),
                "terminal pane context menu should deploy"
            );
            assert!(
                !ws.terminal_input_visible,
                "opening a context menu must not surface the bottom input"
            );
            ws.open_pane_context_menu_at(first, Point::new(px(0.), px(0.)), window, cx);
            assert_eq!(
                ws.active_runtime().focused_pane_id,
                first,
                "opening the menu on the focused pane is idempotent"
            );

            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            let chat = ws
                .active_runtime()
                .panes
                .last()
                .expect("the split pushed a pane")
                .id;
            assert!(matches!(
                ws.active_runtime()
                    .panes
                    .iter()
                    .find(|p| p.id == chat)
                    .map(|p| &p.content),
                Some(PaneContent::AgentChat(_))
            ));

            ws.set_focused_pane(first, window, cx);
            ws.open_pane_context_menu_at(chat, Point::new(px(0.), px(0.)), window, cx);

            assert_eq!(
                ws.active_runtime().focused_pane_id,
                chat,
                "the right-clicked agent chat pane owns the model focus"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
async fn send_pane_selection_activates_the_target_tab_and_pane(cx: &mut TestAppContext) {
    // The delivery contract says `pane_id` is the destination, but AgentChat
    // delivery routes through the *focused* composer. The op therefore has to
    // bring the target tab and pane forward before handing over the text.
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let source = ws.active_runtime().focused_pane_id;

            ws.add_tab(window, cx);
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            let target = ws
                .active_runtime()
                .panes
                .last()
                .expect("the split pushed a pane")
                .id;

            // Go back to the source tab so the send has to switch tabs.
            ws.activate_tab(0, window, cx);
            ws.set_focused_pane(source, window, cx);
            assert_eq!(ws.active_runtime().active_tab_index, 0);

            let delivered =
                ws.send_pane_selection_to(target, SharedString::from("cargo test"), window, cx);

            assert!(delivered, "the target pane accepted the text");
            assert_eq!(
                ws.active_runtime().active_tab_index,
                1,
                "the target's tab is brought forward"
            );
            assert_eq!(ws.active_runtime().focused_pane_id, target);
            assert!(
                ws.terminal_input.read(cx).value().contains("cargo test"),
                "the captured text lands in the target's composer"
            );

            let gone = ws
                .active_runtime()
                .panes
                .iter()
                .map(|p| p.id)
                .max()
                .unwrap_or_default()
                + 1;
            assert!(
                !ws.send_pane_selection_to(gone, SharedString::from("text"), window, cx),
                "a target that no longer exists reports failure instead of panicking"
            );
        });
    })
    .unwrap();
}
