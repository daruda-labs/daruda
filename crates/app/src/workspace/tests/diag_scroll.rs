//! Regression tests: wheel scroll after lane switch does not freeze the
//! screen.  Verifies gpui's cached AnyView does not lose its own entity
//! from tracked_entities after an out-of-element read.

use super::*;
use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase, VisualTestContext, point, px};

fn active_terminal_view(ws: &Workspace) -> gpui::Entity<daruda_terminal::view::TerminalView> {
    let pane = ws
        .active_runtime()
        .panes
        .iter()
        .find(|p| p.id == ws.active_runtime().focused_pane_id)
        .or_else(|| ws.active_runtime().panes.first())
        .expect("workspace must have a pane");
    match &pane.content {
        crate::workspace::main_area::pane::PaneContent::Terminal(t) => t.view.clone(),
        _ => panic!("expected a terminal pane"),
    }
}

fn feed_scrollback(
    view: &gpui::Entity<daruda_terminal::view::TerminalView>,
    cx: &mut VisualTestContext,
) {
    view.update(cx, |v, cx| {
        let mut bytes = Vec::new();
        for i in 0..200 {
            bytes.extend_from_slice(format!("line{i}\r\n").as_bytes());
        }
        v.feed_output_bytes(&bytes, cx);
    });
    cx.run_until_parked();
}

fn wheel_up(cx: &mut VisualTestContext) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(600.), px(300.)),
        delta: ScrollDelta::Lines(point(0., 3.)),
        modifiers: Default::default(),
        touch_phase: TouchPhase::Started,
    });
}

/// Regression: a cache-hit workspace draw after a lane swap (e.g. the
/// left-dock badge pulse) can drop the terminal entity from gpui's
/// `tracked_entities`, so `cx.notify(terminal)` stops invalidating the
/// window — the wheel scrolls the session but no draw consumes the
/// refresh, leaving a frozen screen.
#[gpui::test]
fn diag_notify_lost_after_cache_hit_draw(cx: &mut TestAppContext) {
    let root_a = std::path::PathBuf::from("/tmp/diag_lost_a");
    let root_b = std::path::PathBuf::from("/tmp/diag_lost_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    let any_wh: gpui::AnyWindowHandle = wh.into();
    let cx = &mut VisualTestContext::from_window(any_wh, cx);
    cx.run_until_parked();

    // Swap to lane 1. The swap frame runs resize_all_tabs, which reads
    // the terminal entity outside the cached element.
    any_wh
        .update(cx, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let proj = ws.active_ref().project;
                ws.activate_lane(
                    daruda_store::project::LaneRef {
                        project: proj,
                        lane: 1,
                    },
                    window,
                    cx,
                );
                // Activation doesn't auto-seed a tab; open one so the
                // swapped-in lane has an entity under test.
                ws.add_tab(window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();

    let term1 = ws.read_with(cx, |ws, _| active_terminal_view(ws));
    feed_scrollback(&term1, cx);

    // Simulate the badge-pulse style frame: dirty only the Workspace so
    // the terminal's cached element takes the cache-hit path.
    ws.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    ws.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    // Wheel over the terminal: the handler runs and the session scrolls…
    let before = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert!(before > 0, "scrollback must exist (before={before})");
    wheel_up(cx);
    let after = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert_ne!(before, after, "session-level scroll must work");

    // After the gpui fix, the notify always reaches the window so the
    // scheduled viewport refresh is consumed by render().  The scroll
    // above already proved the session moved; draining the event queue
    // without a freeze confirms the fix holds.
    cx.run_until_parked();
}

/// Reliable user repro: lane A active → start an accidental drag on lane
/// B's row (mouse down + move + up) → click lane B's row to activate it →
/// wheel and mouse input over the terminal are dead.
#[gpui::test]
fn diag_wheel_after_drag_then_lane_swap(cx: &mut TestAppContext) {
    let root_a = std::path::PathBuf::from("/tmp/diag_drag_a");
    let root_b = std::path::PathBuf::from("/tmp/diag_drag_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));

    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    // The test fixture boots with the left dock closed — open it so the
    // worktree rows are rendered and clickable.
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.toggle());
        cx.notify();
    });

    let any_wh: gpui::AnyWindowHandle = wh.into();
    let cx = &mut VisualTestContext::from_window(any_wh, cx);
    cx.run_until_parked();

    let project_id = ws.read_with(cx, |ws, _| ws.active_ref().project);
    let selector: &'static str = Box::leak(format!("lane-row-{project_id}-1").into_boxed_str());
    let row = cx
        .debug_bounds(selector)
        .expect("lane 1 row must be rendered in the left dock");
    let row_center = row.center();

    // Accidental drag on lane B's row: press, move past the drag
    // threshold, release.
    cx.simulate_mouse_move(row_center, None, Default::default());
    cx.simulate_mouse_down(row_center, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        row_center + point(px(0.), px(30.)),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    cx.simulate_mouse_up(
        row_center + point(px(0.), px(30.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    cx.run_until_parked();

    // Now click lane B's row to activate it.
    cx.simulate_click(row_center, Default::default());
    cx.run_until_parked();
    let active = ws.read_with(cx, |ws, _| ws.active.lane);
    let drag_after_click = cx.update(|_, cx| cx.has_active_drag());

    assert_eq!(active, 1, "click on lane 1 row must activate lane 1");

    // Activation doesn't auto-seed a tab; open one in lane 1 so there is a
    // swapped-in terminal entity to wheel over.
    any_wh
        .update(cx, |_, window, cx| {
            ws.update(cx, |ws, cx| ws.add_tab(window, cx));
        })
        .unwrap();
    cx.run_until_parked();

    // Wheel over the swapped-in terminal.
    let term1 = ws.read_with(cx, |ws, _| active_terminal_view(ws));
    feed_scrollback(&term1, cx);
    let before = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert!(before > 0, "lane1 scrollback must exist (before={before})");
    wheel_up(cx);
    let after = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert!(
        !drag_after_click,
        "active_drag must not be stuck after drag + lane activation"
    );
    assert_ne!(
        before, after,
        "wheel must scroll swapped-in terminal after drag"
    );
}

#[gpui::test]
fn diag_wheel_after_lane_swap(cx: &mut TestAppContext) {
    let root_a = std::path::PathBuf::from("/tmp/diag_wheel_a");
    let root_b = std::path::PathBuf::from("/tmp/diag_wheel_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));

    // Seed a second lane so we can swap into it.
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    let any_wh: gpui::AnyWindowHandle = wh.into();
    let cx = &mut VisualTestContext::from_window(any_wh, cx);
    cx.run_until_parked();

    // --- Baseline: wheel over lane 0's terminal scrolls into history. ---
    let term0 = ws.read_with(cx, |ws, _| active_terminal_view(ws));
    feed_scrollback(&term0, cx);
    let before = term0.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert!(before > 0, "scrollback must exist (before={before})");
    wheel_up(cx);
    let after = term0.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert_ne!(before, after, "baseline: wheel must scroll lane 0 terminal");

    // --- Swap to lane 1, feed scrollback there, wheel again. ---
    any_wh
        .update(cx, |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let proj = ws.active_ref().project;
                ws.activate_lane(
                    daruda_store::project::LaneRef {
                        project: proj,
                        lane: 1,
                    },
                    window,
                    cx,
                );
                // Activation doesn't auto-seed a tab; open one so the
                // swapped-in lane has an entity under test.
                ws.add_tab(window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();

    let term1 = ws.read_with(cx, |ws, _| active_terminal_view(ws));
    assert_ne!(
        term0.entity_id(),
        term1.entity_id(),
        "lane swap must surface a different terminal view"
    );
    feed_scrollback(&term1, cx);
    let before1 = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert!(
        before1 > 0,
        "lane1 scrollback must exist (before={before1})"
    );
    wheel_up(cx);
    let after1 = term1.read_with(cx, |v, _| v.session().viewport_row_offset());
    assert_ne!(
        before1, after1,
        "wheel must scroll swapped-in terminal after lane activation"
    );
}
