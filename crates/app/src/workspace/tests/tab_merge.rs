//! Tests for `merge_tab_into_pane` (`tab_ops.rs`) — dropping a dragged tab
//! onto another tab's pane to fold both into a single tab with a 2-leaf
//! split. The drag-mechanics layer (`update_tab_merge_hover_from_move` /
//! `drop_tab_onto_pane` in `pane_drag_ops.rs`) reads a `DragMoveEvent`,
//! whose fields are private outside `gpui` (same constraint noted in
//! `tab_drag.rs`), so these tests call `merge_tab_into_pane` directly —
//! exactly what `drop_tab_onto_pane` does once it has taken
//! `pane_drop_hover`.

use super::*;

// ---- Async lifecycle: a real 2-tab merge lands a 2-leaf split ----

#[gpui::test]
async fn test_merge_tab_into_pane_lands_two_leaf_split(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    // build_workspace already yields tab 0 / pane_a. Add a second tab
    // (tab 1 / pane_b), which add_tab leaves active.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    let (tab0_id, pane_a) = workspace.read_with(cx, |ws, _| {
        let t0 = &ws.active_runtime().tabs[0];
        (t0.id, t0.layout.first_leaf())
    });
    let pane_b = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs[1].layout.first_leaf());
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1);
    });

    // Drag tab 0 (holding pane_a) onto tab 1's pane_b, East half: Horizontal
    // split, pane_a lands after pane_b — mirrors what
    // drop_tab_onto_pane(tab0_id, ...) would do with
    // pane_drop_hover == Some((pane_b, DropHalf::East)).
    let merged = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.merge_tab_into_pane(
                    tab0_id,
                    pane_b,
                    SplitDirection::Horizontal,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    assert!(merged, "merging tab 0 into tab 1's pane should succeed");

    workspace.read_with(cx, |ws, _| {
        // The source tab is consumed — only the merged tab remains.
        assert_eq!(ws.active_runtime().tabs.len(), 1, "source tab is consumed");
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 2);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                ..
            }
        ));
        assert!(ws.active_runtime().tabs[0].layout.contains(pane_a));
        assert!(ws.active_runtime().tabs[0].layout.contains(pane_b));

        // Both PTYs survived the merge — re-parented, not destroyed.
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert!(ws.active_runtime().panes.iter().any(|p| p.id == pane_a));
        assert!(ws.active_runtime().panes.iter().any(|p| p.id == pane_b));

        // The just-merged-in pane (source tab's last_focused_pane) is the
        // one that ends up focused, not the target's prior focus.
        assert_eq!(ws.active_runtime().tabs[0].last_focused_pane, pane_a);
        assert_eq!(ws.active_runtime().focused_pane_id, pane_a);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
    });
}

// ---- Self-merge no-op ----

#[gpui::test]
async fn test_merge_tab_self_merge_is_noop(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    // A single tab: dragging it back onto its own (already active) pane.
    let (tab_id, pane_id) = workspace.read_with(cx, |ws, _| {
        let idx = ws.active_runtime().active_tab_index;
        (ws.active_runtime().tabs[idx].id, ws.active_runtime().panes[0].id)
    });
    let tabs_before = workspace.read_with(cx, |ws, _| ws.active_runtime().tabs.len());

    let merged = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.merge_tab_into_pane(
                    tab_id,
                    pane_id,
                    SplitDirection::Horizontal,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap();

    assert!(
        !merged,
        "dragging the active tab onto its own content must be a no-op"
    );
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), tabs_before);
        assert_eq!(ws.active_runtime().tabs[0].id, tab_id);
    });
}

// ---- Vanished-target restore ----

#[gpui::test]
async fn test_merge_tab_vanished_target_restores_source(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    let (tab0_id, tab0_leaf, tab1_id) = workspace.read_with(cx, |ws, _| {
        let t0 = &ws.active_runtime().tabs[0];
        let t1 = &ws.active_runtime().tabs[1];
        (t0.id, t0.layout.first_leaf(), t1.id)
    });
    let (active_before, history_before) = workspace.read_with(cx, |ws, _| {
        (
            ws.active_runtime().active_tab_index,
            ws.active_runtime().tab_history.clone(),
        )
    });

    // A target pane id that exists nowhere in the active tab's layout —
    // simulates the race where the hovered pane closed between
    // update_tab_merge_hover_from_move computing the hover and the drop
    // committing it.
    let vanished_target: u64 = 999_999;
    let merged = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.merge_tab_into_pane(
                    tab0_id,
                    vanished_target,
                    SplitDirection::Horizontal,
                    false,
                    window,
                    cx,
                )
            })
        })
        .unwrap();

    assert!(
        !merged,
        "a target pane absent from the active tab must be a no-op"
    );
    workspace.read_with(cx, |ws, _| {
        // The source tab was never removed — its id, layout, and position
        // are all unchanged.
        assert_eq!(
            ws.active_runtime().tabs.len(),
            2,
            "source tab must still be present"
        );
        assert_eq!(ws.active_runtime().tabs[0].id, tab0_id);
        assert_eq!(ws.active_runtime().tabs[0].layout.first_leaf(), tab0_leaf);
        assert_eq!(ws.active_runtime().tabs[1].id, tab1_id);
        // Nothing was rebased either, because nothing was removed.
        assert_eq!(ws.active_runtime().active_tab_index, active_before);
        assert_eq!(ws.active_runtime().tab_history, history_before);
    });
}
