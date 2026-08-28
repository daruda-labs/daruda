//! What a press on the agent chat's Activity Bar is allowed to change.
//!
//! The bar is pane chrome: its controls say how the pane is *displayed*, not
//! that the user is engaging with what is in it. The pane wrapper reads any left
//! press inside a pane as "activate this pane" (`focus_pane_on_click` →
//! `focus_pane`, which swaps the bottom-dock draft, moves keyboard focus to the
//! bottom input and lazily connects an idle agent session), so the bar has to
//! keep its own presses to itself.

use gpui::{Modifiers, VisualTestContext};

use super::*;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

fn user(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::UserText(text.into())
}

fn assistant(text: &str) -> daruda_acp::ChatItem {
    daruda_acp::ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
    }
}

/// Split into `terminal | agent chat`, seed two turns, then focus the terminal.
///
/// Two turns so the first is settled history that `Auto` collapses — what
/// `Expand all` then has to undo. The items also enable the button: a *disabled*
/// `Button` stops propagation itself, which would pass the first test falsely.
async fn split_with_seeded_unfocused_chat(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    PaneId,
) {
    let (window_handle, workspace) = build_workspace(cx);
    let terminal = workspace.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);

    cx.update_window(window_handle.into(), |_, window, cx| {
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
            let view = ws
                .agent_chat_view(chat)
                .cloned()
                .expect("the chat pane owns a view");
            view.update(cx, |v, cx| {
                v.seed_items_for_test(
                    [
                        user("q1"),
                        assistant("process"),
                        assistant("first answer"),
                        user("q2"),
                        assistant("second answer"),
                    ],
                    cx,
                );
            });
            // The split focuses the new chat pane; hand focus back so the press
            // under test has an activation left to cause.
            ws.focus_pane_on_click(terminal, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().focused_pane_id,
            terminal,
            "the terminal is the focused pane before the press"
        );
    });

    (window_handle, workspace, terminal)
}

/// Locate the `Expand all` button. `refresh` is what makes it findable: the
/// chat pane is embedded as a `.cached()` view, and a reused paint records no
/// debug bounds — refresh sets `window.refreshing`, which bypasses the cache.
fn expand_all_bounds(vcx: &mut VisualTestContext) -> gpui::Bounds<gpui::Pixels> {
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.debug_bounds("agent-chat-expand-all")
        .expect("the expand-all button painted")
}

/// Visible rows across every agent chat pane — the projection's own answer to
/// "how much is on screen", which `Expand all` is supposed to raise.
fn visible_rows(ws: &Workspace, cx: &gpui::App) -> usize {
    ws.active_runtime()
        .panes
        .iter()
        .filter_map(|p| p.agent_chat_view())
        .map(|v| v.read(cx).rows.iter().filter(|r| !r.hidden).count())
        .sum()
}

/// Pressing `Expand all` must not activate the pane it belongs to — activation
/// swaps the bottom-dock draft, moves keyboard focus, and connects a dormant
/// session. A real workspace, because the press has to cross the pane's
/// `.cached()` view boundary to reach the wrapper at all.
#[gpui::test]
async fn an_activity_bar_press_does_not_activate_the_pane(cx: &mut TestAppContext) {
    let (window_handle, workspace, terminal) = split_with_seeded_unfocused_chat(cx).await;

    let mut vcx = VisualTestContext::from_window(window_handle.into(), cx);
    let button = expand_all_bounds(&mut vcx);
    vcx.simulate_click(button.center(), Modifiers::default());
    vcx.run_until_parked();

    workspace.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.active_runtime().focused_pane_id,
            terminal,
            "the Activity Bar's press activated the pane it sits on"
        );
    });
}

/// The control still works — the press is kept from the wrapper, not from the
/// button. Without this, consuming the press too early would satisfy the test
/// above by making the button do nothing at all.
#[gpui::test]
async fn an_activity_bar_press_still_reaches_its_own_control(cx: &mut TestAppContext) {
    let (window_handle, workspace, _terminal) = split_with_seeded_unfocused_chat(cx).await;

    let mut vcx = VisualTestContext::from_window(window_handle.into(), cx);
    let button = expand_all_bounds(&mut vcx);

    let before = workspace.read_with(&vcx, visible_rows);
    vcx.simulate_click(button.center(), Modifiers::default());
    vcx.run_until_parked();
    let after = workspace.read_with(&vcx, visible_rows);

    assert!(
        after > before,
        "expand-all put nothing more on screen: {before} -> {after}"
    );
}
