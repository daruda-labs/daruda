//! Which panes the user can see — the question the "skip the pane you are
//! looking at" notification rule asks.

use super::*;
use crate::workspace::main_area::pane_tree::PaneId;

fn split(
    wh: gpui::WindowHandle<gpui_component::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) -> (PaneId, PaneId) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.split_focused_pane_kind(
                NewPaneKind::Terminal,
                SplitDirection::Horizontal,
                window,
                cx,
            );
        });
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        let ids: Vec<PaneId> = ws.active_runtime().panes.iter().map(|p| p.id).collect();
        (ids[0], ids[1])
    })
}

fn on_screen(ws: &gpui::Entity<Workspace>, pane: PaneId, cx: &mut TestAppContext) -> bool {
    ws.read_with(cx, |ws, _| ws.pane_on_screen(pane))
}

#[gpui::test]
async fn both_halves_of_a_split_are_on_screen(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    let (first, second) = split(wh, &ws, cx);
    assert!(
        on_screen(&ws, first, cx),
        "the unfocused half is in view too"
    );
    assert!(on_screen(&ws, second, cx));
}

#[gpui::test]
async fn a_pane_in_another_tab_is_not(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    let first = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.add_tab(window, cx));
    })
    .unwrap();
    assert!(!on_screen(&ws, first, cx));
}

#[gpui::test]
async fn zooming_one_half_hides_the_other(cx: &mut TestAppContext) {
    let (wh, ws) = build_workspace(cx);
    let (first, second) = split(wh, &ws, cx);
    ws.update(cx, |ws, cx| ws.toggle_zoom_pane(second, cx));
    assert!(!on_screen(&ws, first, cx));
    assert!(on_screen(&ws, second, cx));
}

#[gpui::test]
async fn a_page_over_the_center_hides_every_pane(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let pane = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    ws.update(cx, |ws, cx| {
        ws.show_page(crate::workspace::pages::Page::Tasks, cx)
    });
    assert!(!on_screen(&ws, pane, cx));
}

/// A zoom made in another lane zooms nothing in this one: its panes are all
/// in view, as the render draws them.
#[gpui::test]
async fn a_zoom_left_in_another_lane_hides_nothing_here(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path());
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.run_until_parked();
    let (first, second) = split(wh, &ws, cx);
    ws.update(cx, |ws, cx| ws.toggle_zoom_pane(second, cx));
    let other_dir = dir.path().join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other = ws.update(cx, |ws, _| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, other_dir.clone());
        lane.tab_order = 1;
        ws.project_for_mut(project).unwrap().lanes.push(lane);
        daruda_store::project::LaneRef {
            project,
            lane: lane_id,
        }
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate_lane(other, window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let here = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    assert!(
        on_screen(&ws, here, cx),
        "the zoom elsewhere does not hide it"
    );
    assert!(
        !on_screen(&ws, first, cx) && !on_screen(&ws, second, cx),
        "the parked lane's panes are out of view"
    );
}
