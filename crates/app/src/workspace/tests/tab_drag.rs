//! Tests for the tab-bar drag-reorder ops (`tab_drag_ops.rs` +
//! `switch_tab_for_drag_preview` in `tab_ops.rs`).
//!
//! Two properties are load-bearing here and hard to see by inspection
//! alone:
//!   - the hover-preview switch must never persist (it runs on every tab
//!     the drag passes over, so persisting it would spam disk writes and
//!     could commit a reorder the user never dropped);
//!   - overwriting `tab_hover_switch` must actually cancel the previous
//!     timer, or a stale countdown started on tab A could fire after the
//!     drag has moved on to a different tab.

use std::time::Duration;

use daruda_store::project::load_project_state_in;
use gpui::Window;

use super::*;

fn make_workspace_with_project(
    cx: &mut TestAppContext,
    primary: &str,
) -> gpui::WindowHandle<Workspace> {
    let config = daruda_config::Config::default();
    std::fs::create_dir_all(primary).unwrap();
    let project = daruda_store::project::Project::from_path(primary);
    cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    })
}

/// `TestAppContext` doesn't implement `VisualContext`, so `Entity::update_in`
/// isn't available on it (unlike the `Root`-wrapped windows `build_workspace`
/// returns elsewhere in this test suite). Route window-needing calls through
/// `cx.update_window` instead — same pattern `lifecycle.rs` / `splits.rs` use.
fn in_window<R>(
    wh: gpui::WindowHandle<Workspace>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> R {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| f(ws, window, cx))
    })
    .unwrap()
}

// ---- Step 1: preview switch must skip persistence ----

#[gpui::test]
async fn switch_tab_for_drag_preview_skips_persistence(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_preview_persist");
    let ws = wh.root(cx).unwrap();

    // Land on a durable baseline: 2 tabs, active_tab_index = 0. `add_tab`
    // itself isn't durable-wrapped (matches production call sites), so
    // route back through `activate_tab` — the durable sibling of the
    // method under test — to get a real on-disk snapshot to diff against.
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
        ws.activate_tab(0, window, cx);
    });
    cx.run_until_parked();

    let (data_dir, project_uuid) = ws.read_with(cx, |ws, _| {
        (ws.data_dir.clone(), ws.active_project().unwrap().uuid)
    });
    let before = load_project_state_in(&data_dir, project_uuid).expect("baseline persisted");

    ws.update(cx, |ws, cx| {
        ws.switch_tab_for_drag_preview(1, cx);
    });
    cx.run_until_parked();

    let active_after = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(
        active_after, 1,
        "preview switch must still change the in-memory active tab"
    );

    let after = load_project_state_in(&data_dir, project_uuid)
        .expect("project state file must still exist");
    assert_eq!(
        before, after,
        "preview switch must not write a new project-state snapshot"
    );
}

#[gpui::test]
async fn switch_tab_for_drag_preview_noop_out_of_range_does_not_switch(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_preview_noop");
    let ws = wh.root(cx).unwrap();

    let before = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    ws.update(cx, |ws, cx| {
        // Single-tab workspace: index 5 is out of range.
        ws.switch_tab_for_drag_preview(5, cx);
    });
    let after = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(before, after, "out-of-range preview index must be a no-op");
}

// ---- Async lifecycle: drag-reorder end state ----

#[gpui::test]
async fn drop_tab_onto_bar_reorders_dragged_tab_before_first(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_reorder_lifecycle");
    let ws = wh.root(cx).unwrap();

    // Build up 3 tabs (indices 0, 1, 2).
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
        ws.add_tab(window, cx);
    });
    let ids: Vec<u64> = ws.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(ids.len(), 3, "expected 3 tabs after two add_tab calls");
    let dragged_id = ids[2];

    // Simulate the drag having landed with a "west half of tab 0" preview
    // (insert-before index 0) — this is what `update_tab_drag_from_move`
    // would have written into `tab_reorder_preview` while hovering the
    // left edge of the first tab; `DragMoveEvent` itself can't be
    // constructed outside `gpui` (its fields are private), so the drop
    // half of the flow is exercised directly here.
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.main_area.tab_reorder_preview = Some(0);
        ws.drop_tab_onto_bar(dragged_id, window, cx);
    });
    cx.run_until_parked();

    let (after_ids, after_panes): (Vec<u64>, Vec<u64>) = ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .tabs
            .iter()
            .map(|t| (t.id, t.last_focused_pane))
            .unzip()
    });
    assert_eq!(
        after_ids,
        vec![ids[2], ids[0], ids[1]],
        "tab dragged from index 2 to before index 0 must land at index 0"
    );

    // The drop must also settle transient drag state.
    ws.read_with(cx, |ws, _| {
        assert!(ws.main_area.tab_reorder_preview.is_none());
        assert!(ws.main_area.tab_hover_switch.is_none());
    });

    // And, unlike the preview switch, it must persist — `SerializedTab`
    // has no stable `id` field (tabs are positional on disk), so compare
    // by the pane each tab carries instead.
    let (data_dir, project_uuid) = ws.read_with(cx, |ws, _| {
        (ws.data_dir.clone(), ws.active_project().unwrap().uuid)
    });
    let persisted = load_project_state_in(&data_dir, project_uuid).expect("reorder must persist");
    let persisted_panes: Vec<u64> = persisted.lanes[0]
        .tabs
        .iter()
        .map(|t| t.last_focused_pane)
        .collect();
    assert_eq!(
        persisted_panes, after_panes,
        "the reorder must reach disk, unlike the preview switch"
    );
}

#[gpui::test]
async fn drop_tab_onto_bar_without_preview_is_noop(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_drop_no_preview");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let before: Vec<u64> = ws.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    let dragged_id = before[0];

    in_window(wh, &ws, cx, |ws, window, cx| {
        // No armed preview (`tab_reorder_preview` is `None`) — a drop must
        // not reorder anything.
        ws.drop_tab_onto_bar(dragged_id, window, cx);
    });

    let after: Vec<u64> = ws.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(before, after);
}

// ---- Hover-switch timer: stale-timer cancellation ----

#[gpui::test]
async fn arming_hover_switch_on_a_new_tab_cancels_the_previous_countdown(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_hover_cancel");
    let ws = wh.root(cx).unwrap();

    // 3 tabs: id_a at index 0, a filler at index 1, id_active (already the
    // active tab) at index 2.
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
        ws.add_tab(window, cx);
    });
    let (id_a, id_active) = ws.read_with(cx, |ws, _| {
        let tabs = &ws.active_runtime().tabs;
        (tabs[0].id, tabs[2].id)
    });
    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        2
    );

    // Arm the hover timer on tab A (would switch to index 0), then
    // immediately re-arm on the *already-active* tab (mirrors the drag
    // moving off A before A's delay elapses). Re-arming on the active tab
    // is deliberately a no-op target — `switch_active_tab_index` short-
    // circuits when the index already matches — so the only way
    // `active_tab_index` can move after the delay is if A's supposedly
    // superseded timer still fires. This isolates "did overwriting
    // `tab_hover_switch` actually cancel A" from "which of the two fired
    // last", which a same-target A/B re-arm can't distinguish (both would
    // land on the same index regardless of whether A also fired first).
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.arm_tab_hover_switch(id_a, window, cx);
        ws.arm_tab_hover_switch(id_active, window, cx);
    });

    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();

    let active = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(
        active, 2,
        "tab A's superseded hover timer must not fire once a different tab is armed"
    );
}

#[gpui::test]
async fn hover_switch_fires_after_the_delay_elapses(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_hover_fires");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let target_id = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[0].id);

    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.arm_tab_hover_switch(target_id, window, cx);
    });
    // Still within the delay — no switch yet.
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    let mid = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(
        mid, 1,
        "hover switch must not fire before the delay elapses"
    );

    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let after = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(
        after, 0,
        "hover switch must fire once the delay has elapsed"
    );
}

#[gpui::test]
async fn hover_switch_noops_when_the_armed_tab_no_longer_exists(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_hover_stale_tab");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let doomed_id = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[0].id);

    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.arm_tab_hover_switch(doomed_id, window, cx);
        // The armed tab closes mid-countdown.
        ws.close_tab_at(0, window, cx);
    });

    cx.executor().advance_clock(Duration::from_millis(700));
    cx.run_until_parked();

    // Must not panic on a stale index into a shrunk tab list, and must
    // leave the surviving tab active.
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
    });
}
