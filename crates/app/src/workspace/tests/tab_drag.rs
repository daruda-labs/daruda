//! Tests for the tab-bar drag-reorder ops (`tab_drag_ops.rs` +
//! `switch_tab_for_drag_preview` in `tab_ops.rs`).
//!
//! Properties that are load-bearing and hard to see by inspection alone:
//!   - the hover-preview switch must never persist (it runs on every tab
//!     the drag passes over, so persisting it would spam disk writes and
//!     could commit a reorder the user never dropped);
//!   - overwriting `tab_hover_switch` must actually cancel the previous
//!     timer, or a stale countdown fires after the drag has moved on;
//!   - a drag ending without a committed drop must unwind the preview to
//!     the pre-drag tab, and one that commits must not.

use std::time::Duration;

use daruda_store::project::load_project_state_in;
use gpui::Window;

use crate::workspace::main_area::pane_tree::{DropHalf, PaneId};

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

    ws.update(cx, |ws, cx| {
        ws.switch_tab_for_drag_preview(5, cx);
    });
    let after_noop = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(
        after_noop, active_after,
        "out-of-range preview index must be a no-op"
    );

    in_window(wh, &ws, cx, |ws, window, cx| {
        assert!(
            !ws.cancel_active_drag(window, cx),
            "no GPUI drag is active, so there is nothing to cancel"
        );
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().active_tab_index,
            active_after,
            "a no-op cancel must not unwind the preview"
        );
        assert!(ws.main_area.tab_preview_restore.is_some());
    });
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
    // (insert-before index 0), tagged with that cell's tab id as owner.
    // `DragMoveEvent` can't be constructed outside `gpui` (private fields),
    // so only the drop half of the flow is exercised here.
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.main_area.tab_reorder_preview = Some((ids[0], 0));
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

    let doomed_id = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[1].id);
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.arm_tab_hover_switch(doomed_id, window, cx);
        // The armed tab closes mid-countdown.
        ws.close_tab_at(1, window, cx);
    });

    cx.executor().advance_clock(Duration::from_millis(700));
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(
            ws.active_runtime().active_tab_index,
            0,
            "a stale hover switch must leave the surviving tab active"
        );
    });
}

// ---- Abandoned drag: the hover preview must not stick ----

/// The hover preview is a look-ahead, not a commit: abandoning restores the
/// pre-drag tab, while a committed drop keeps the previewed tab.
#[gpui::test]
async fn finish_tab_drag_restores_abandoned_and_keeps_committed(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_restore_abandoned");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
        ws.add_tab(window, cx);
    });
    // Three tabs, index 2 active (add_tab activates the new tab).
    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        2
    );

    in_window(wh, &ws, cx, |ws, _window, cx| {
        ws.switch_tab_for_drag_preview(0, cx);
        ws.switch_tab_for_drag_preview(1, cx);
    });
    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        1,
        "preview must move the active tab while the drag is live"
    );

    in_window(wh, &ws, cx, |ws, _window, cx| {
        ws.finish_tab_drag(false, cx);
    });

    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        2,
        "an abandoned drag must restore the pre-drag active tab, not the last preview hop"
    );

    in_window(wh, &ws, cx, |ws, _window, cx| {
        ws.switch_tab_for_drag_preview(0, cx);
        ws.finish_tab_drag(true, cx);
    });

    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        0,
        "a committed drop must keep the tab the drop landed on"
    );
}

/// `close_tab_at` picks its successor by popping `tab_history`, so a drag
/// preview that leaves entries there redirects a later close. With A/B/C and
/// the user having just come B→C, previewing A and abandoning the drag must
/// leave closing C going back to B.
#[gpui::test]
async fn abandoned_drag_leaves_no_trace_in_tab_history(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_history_clean");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
        ws.add_tab(window, cx);
        // Real navigation B → C, so history's top is B.
        ws.activate_tab(1, window, cx);
        ws.activate_tab(2, window, cx);
    });
    let tab_b = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[1].id);

    in_window(wh, &ws, cx, |ws, _window, cx| {
        ws.switch_tab_for_drag_preview(0, cx);
        ws.finish_tab_drag(false, cx);
    });
    cx.run_until_parked();

    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.close_tab_at(2, window, cx);
    });

    let active_id = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        rt.tabs[rt.active_tab_index].id
    });
    assert_eq!(
        active_id, tab_b,
        "closing C must fall back to B — the abandoned preview of A must not \
         have entered tab history"
    );
}

/// The reported scenario end to end: preview another tab, then drop onto a
/// pane half to merge. That commits, so the previewed tab must stay — the
/// user dropped while looking at it.
#[gpui::test]
async fn drop_tab_onto_pane_keeps_committed_preview_and_restores_refused_merge(
    cx: &mut TestAppContext,
) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_merge_keeps_preview");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let (dragged_id, target_pane) = ws.read_with(cx, |ws, _| {
        let rt = ws.active_runtime();
        (rt.tabs[1].id, rt.tabs[0].last_focused_pane)
    });

    in_window(wh, &ws, cx, |ws, window, cx| {
        // Hover-preview tab 0, then drop the dragged tab onto tab 0's pane.
        ws.switch_tab_for_drag_preview(0, cx);
        ws.main_area.pane_drop_hover = Some((target_pane, DropHalf::East));
        ws.drop_tab_onto_pane(dragged_id, window, cx);
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            1,
            "the dragged tab merged into the previewed tab"
        );
        assert!(ws.main_area.tab_preview_restore.is_none());
        assert!(ws.main_area.tab_hover_switch.is_none());
    });

    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_merge_refused");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let dragged_id = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[1].id);

    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.switch_tab_for_drag_preview(0, cx);
        // A pane id that exists in no tab — `merge_tab_into_pane` bails.
        ws.main_area.pane_drop_hover = Some((PaneId::MAX, DropHalf::East));
        ws.drop_tab_onto_pane(dragged_id, window, cx);
    });
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2, "nothing merged");
        assert_eq!(
            ws.active_runtime().active_tab_index,
            1,
            "a refused merge must unwind the preview"
        );
    });
}

// ---- Leaving a tab cell: release only what that cell owns ----

/// Hovering tab 0 to preview it, then dragging down into the pane area to
/// pick a split half: every cell's `on_drag_move` keeps firing there (GPUI
/// has no hover guard on it), so tab 0's cell resolves to `Release`. Its
/// armed countdown must die with it, or it fires mid-drop.
#[gpui::test]
async fn releasing_a_cell_cancels_the_countdown_it_armed(cx: &mut TestAppContext) {
    let wh = make_workspace_with_project(cx, "/tmp/daruda_tab_drag_release_cancels");
    let ws = wh.root(cx).unwrap();
    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.add_tab(window, cx);
    });
    let hovered_id = ws.read_with(cx, |ws, _| ws.active_runtime().tabs[0].id);
    let active_before = ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index);
    assert_eq!(active_before, 1);

    in_window(wh, &ws, cx, |ws, window, cx| {
        ws.arm_tab_hover_switch(hovered_id, window, cx);
        ws.main_area.tab_reorder_preview = Some((hovered_id, 0));
        // Cursor moves off the cell (down into the pane area).
        ws.release_tab_drag_state_owned_by(hovered_id, cx);
    });

    ws.read_with(cx, |ws, _| {
        assert!(ws.main_area.tab_hover_switch.is_none());
        assert!(ws.main_area.tab_reorder_preview.is_none());
    });

    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();

    assert_eq!(
        ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index),
        active_before,
        "a released cell's countdown must not switch tabs after the cursor left it"
    );

    let (left_id, entered_id) = ws.read_with(cx, |ws, _| {
        let tabs = &ws.active_runtime().tabs;
        (tabs[0].id, tabs[1].id)
    });

    in_window(wh, &ws, cx, |ws, window, cx| {
        // The entered cell claims the drag...
        ws.arm_tab_hover_switch(entered_id, window, cx);
        ws.main_area.tab_reorder_preview = Some((entered_id, 1));
        // ...and the cell just left runs afterwards in the same frame.
        ws.release_tab_drag_state_owned_by(left_id, cx);
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.main_area.tab_hover_switch.as_ref().map(|(id, _)| *id),
            Some(entered_id),
            "the entered cell's countdown must survive its neighbour's release"
        );
        assert_eq!(ws.main_area.tab_reorder_preview, Some((entered_id, 1)));
    });
}
