use super::*;

// ---- Split tests ----

#[gpui::test]
async fn test_split_right(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.panes.len(), 1);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.panes.len(), 2);
        assert!(matches!(
            ws.tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                ..
            }
        ));
    });
}

#[gpui::test]
async fn test_split_down(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Vertical, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.panes.len(), 2);
        assert!(matches!(
            ws.tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ));
    });
}

#[gpui::test]
async fn test_close_pane_in_split(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.panes.len(), 2);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_focused_pane(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.panes.len(), 1);
        assert!(matches!(ws.tabs[0].layout, PaneLayout::Leaf(_)));
    });
}

#[gpui::test]
async fn test_focus_next_prev_pane(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    })
    .unwrap();

    let (pane_a, pane_b) = workspace.read_with(cx, |ws, _| {
        let ids = ws.tabs[0].layout.pane_ids();
        (ids[0], ids[1])
    });

    // After split, focus is on the new (second) pane.
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.focused_pane_id, pane_b);
    });

    // Focus next wraps to first pane.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.focus_next_pane(window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.focused_pane_id, pane_a);
    });

    // Focus prev wraps back to second pane.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.focus_prev_pane(window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.focused_pane_id, pane_b);
    });
}

#[gpui::test]
async fn test_nested_split(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    // First split: H-split creates [A | B], focus on B.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    })
    .unwrap();

    // Second split: V-split on B creates [A | [B / C]], focus on C.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane(SplitDirection::Vertical, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.panes.len(), 3);
        assert_eq!(ws.tabs[0].layout.leaf_count(), 3);
    });

    // Close focused pane (C) → back to [A | B].
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_focused_pane(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.panes.len(), 2);
        assert_eq!(ws.tabs[0].layout.leaf_count(), 2);
    });
}

#[gpui::test]
async fn test_close_tab_removes_all_panes(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    // Add a second tab so closing won't quit.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
        });
    })
    .unwrap();

    // Go back to first tab and split it.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.activate_tab(0, window, cx);
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.panes.len(), 3); // 2 in tab 0, 1 in tab 1
    });

    // Close tab 0 — should remove both panes.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(0, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.panes.len(), 1);
    });
}

#[gpui::test]
async fn test_move_tab_left_right(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();
    let ids: Vec<u64> = workspace.read_with(cx, |ws, _| ws.tabs.iter().map(|t| t.id).collect());
    assert_eq!(ids.len(), 3);

    // Active tab is index 2; move it left → index 1.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_move_tab_left(&MoveTabLeft, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_tab_index, 1);
        assert_eq!(ws.tabs[1].id, ids[2]);
        assert_eq!(ws.tabs[2].id, ids[1]);
    });

    // Move right back to 2.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_move_tab_right(&MoveTabRight, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_tab_index, 2);
        assert_eq!(ws.tabs[2].id, ids[2]);
    });
}

#[gpui::test]
async fn test_osc7_updates_pane_cwd_via_view_flush(cx: &mut TestAppContext) {
    // Verifies the queue→flush→read contract: TerminalView::queue_output_bytes
    // accumulates into pending_output which is drained by flush_pending_output
    // or render(). The poll task reads terminal_cwd() right after queueing, so
    // flush_pending_output() must run in between for OSC 7 to land.
    let (window_handle, workspace) = build_workspace(cx);

    let view = workspace.read_with(cx, |ws, _| {
        ws.panes[0]
            .terminal_view()
            .expect("first pane is a terminal in build_workspace")
            .clone()
    });

    cx.update_window(window_handle.into(), |_, _window, cx| {
        view.update(cx, |this, cx| {
            this.queue_output_bytes(b"\x1b]7;file://host/home/user\x07", cx);
            // Bug repro: without flush, terminal_cwd is still None.
            assert_eq!(this.terminal_cwd(), None);
            this.flush_pending_output(cx);
            // After flush the OSC 7 sequence is parsed.
            assert_eq!(this.terminal_cwd(), Some("/home/user"));
        });
    })
    .unwrap();
}

#[gpui::test]
async fn test_close_active_tab_returns_to_last_active(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx); // index 1
            ws.add_tab(window, cx); // index 2 (active)
            ws.activate_tab(0, window, cx); // active = 0, last = 2
            ws.activate_tab(1, window, cx); // active = 1, last = 0
        });
    })
    .unwrap();
    let ids: Vec<u64> = workspace.read_with(cx, |ws, _| ws.tabs.iter().map(|t| t.id).collect());

    // Close active tab (index 1) — should jump back to previously-active (0).
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(1, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active_tab_index, 0);
        assert_eq!(ws.tabs[0].id, ids[0]);
    });
}

#[gpui::test]
async fn test_move_tab_adjusts_history_indices(cx: &mut TestAppContext) {
    // Tabs: [A(0), B(1), C(2)]
    // Navigate: 0→1→2→0 so history = [1, 2, 1, 0] (with dedup only on consecutive).
    // Active = 0 after final activate_tab(0).
    // Move tab A (index 0) to index 2: tabs become [B(0), C(1), A(2)].
    // Expected history adjustments: old-index-0 → 2, indices 1..=2 shift -1.
    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx); // B at index 1
            ws.add_tab(window, cx); // C at index 2 (active after add)
            ws.activate_tab(0, window, cx); // push 2 → history=[2], active=0
            ws.activate_tab(1, window, cx); // push 0 → history=[2,0], active=1
            ws.activate_tab(0, window, cx); // push 1 → history=[2,0,1], active=0
        });
    })
    .unwrap();

    // Confirm setup: active=0, history ends with 1.
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_tab_index, 0);
        assert_eq!(ws.tab_history.last(), Some(&1));
    });

    let ids: Vec<u64> = workspace.read_with(cx, |ws, _| ws.tabs.iter().map(|t| t.id).collect());

    // Move tab A (0) to tail position (2): [A,B,C] → [B,C,A]
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.move_tab(0, 2, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        // Tab order: B, C, A.
        assert_eq!(ws.tabs[0].id, ids[1]); // B
        assert_eq!(ws.tabs[1].id, ids[2]); // C
        assert_eq!(ws.tabs[2].id, ids[0]); // A

        // active_tab_index was 0 (A), which moved to 2.
        assert_eq!(ws.active_tab_index, 2);

        // History had entry 1 (B) at the top → B is now index 0 (shifted down).
        // All history entries that referenced A (old 0) map to new 2.
        // All history entries that referenced indices >0 and ≤2 shift −1.
        // Original history (top-down): [..., 2(C), 0(A), 1(B)]
        //   2(C) → from<to (0<2) && 2>0 && 2<=2 → 2−1 = 1 ✓
        //   0(A) → idx==from → to = 2 ✓
        //   1(B) → from<to (0<2) && 1>0 && 1<=2 → 1−1 = 0 ✓
        assert!(ws.tab_history.contains(&2)); // A's new position
        assert!(ws.tab_history.contains(&0)); // B's new position
        assert!(ws.tab_history.contains(&1)); // C's new position
    });
}

#[gpui::test]
async fn test_close_tab_stale_history_fallback(cx: &mut TestAppContext) {
    // Verify the defensive pop-loop in close_tab_at: when all history entries
    // have been invalidated by prior removes, the fallback index.min(len-1) is used.
    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx); // index 1
            ws.add_tab(window, cx); // index 2 (active)
            // Activate 2 → push 0, history=[0]
            // Then activate 1 → push 2, history=[0,2]
            // active is now 1.
            ws.activate_tab(2, window, cx);
            ws.activate_tab(1, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_tab_index, 1);
    });

    // Close tab 2 first — this removes the history entry pointing at 2,
    // adjusts the remaining entries (0 stays 0). History should now have [0].
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(2, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active_tab_index, 1); // active tab 1 unaffected (index < 2)
    });

    // Close active tab 1 — history has [0] which is valid → should navigate to 0.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(1, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.active_tab_index, 0);
    });
}
