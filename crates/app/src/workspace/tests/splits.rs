use super::*;
use crate::workspace::main_area::pane::PaneContent;

// ---- Split tests ----

#[gpui::test]
async fn split_directions_and_close_restore_single_pane(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().panes.len(), 1);
    });

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
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                ..
            }
        ));
    });
    let (pane_a, pane_b) = workspace.read_with(cx, |ws, _| {
        let ids = ws.active_runtime().tabs[0].layout.pane_ids();
        (ids[0], ids[1])
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().focused_pane_id,
            pane_b,
            "after split, focus is on the new pane"
        );
    });
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.focus_next_pane(window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().focused_pane_id, pane_a);
    });
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.focus_prev_pane(window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().focused_pane_id, pane_b);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_focused_pane(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().panes.len(), 1);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Pane(_)
        ));
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane_kind(NewPaneKind::Terminal, SplitDirection::Vertical, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Vertical,
                ..
            }
        ));
    });

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
        assert_eq!(ws.active_runtime().panes.len(), 3);
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 3);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_focused_pane(window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 2);
    });
}

#[gpui::test]
async fn detach_pane_noops_for_single_leaf_and_succeeds_from_split(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    let only_pane = workspace.read_with(cx, |ws, _| ws.active_runtime().panes[0].id);
    let detached = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.detach_pane_to_new_tab(only_pane, 1, window, cx)
            })
        })
        .unwrap();

    assert!(
        !detached,
        "detaching the only pane in a tab must be a no-op"
    );
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().panes.len(), 1);
    });

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

    let (pane_a, pane_b) = workspace.read_with(cx, |ws, _| {
        let ids = ws.active_runtime().tabs[0].layout.pane_ids();
        (ids[0], ids[1])
    });
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 2);
    });

    let detached = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let at = ws.active_runtime().tabs.len();
                ws.detach_pane_to_new_tab(pane_b, at, window, cx)
            })
        })
        .unwrap();
    assert!(detached, "detaching a split pane should succeed");

    workspace.read_with(cx, |ws, _| {
        // Original tab shrank back to a single leaf holding only pane_a.
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 1);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Pane(id) if id == pane_a
        ));

        // A new tab was appended holding exactly the detached pane.
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert!(matches!(
            ws.active_runtime().tabs[1].layout,
            PaneLayout::Pane(id) if id == pane_b
        ));
        assert_eq!(ws.active_runtime().tabs[1].last_focused_pane, pane_b);

        // Both panes stayed alive — re-parented, not destroyed.
        assert_eq!(ws.active_runtime().panes.len(), 2);

        // The new tab was activated and holds keyboard focus.
        assert_eq!(ws.active_runtime().active_tab_index, 1);
        assert_eq!(ws.active_runtime().focused_pane_id, pane_b);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.activate_tab(0, window, cx);
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
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(
            ws.active_runtime().panes.len(),
            3,
            "split tab plus detached tab should hold three panes"
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(0, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(
            ws.active_runtime().panes.len(),
            1,
            "closing a split tab must remove all panes owned only by that tab"
        );
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Pane(id) if id == pane_b
        ));
    });
}

#[gpui::test]
async fn tab_move_actions_and_history_indices(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.add_tab(window, cx);
        });
    })
    .unwrap();
    let ids: Vec<u64> = workspace.read_with(cx, |ws, _| {
        ws.active_runtime().tabs.iter().map(|t| t.id).collect()
    });
    assert_eq!(ids.len(), 3);

    // Active tab is index 2; move it left → index 1.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_move_tab_left(&MoveTabLeft, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 1);
        assert_eq!(ws.active_runtime().tabs[1].id, ids[2]);
        assert_eq!(ws.active_runtime().tabs[2].id, ids[1]);
    });

    // Move right back to 2.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_move_tab_right(&MoveTabRight, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 2);
        assert_eq!(ws.active_runtime().tabs[2].id, ids[2]);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.activate_tab(0, window, cx);
            ws.activate_tab(1, window, cx);
            ws.activate_tab(0, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 0);
        assert_eq!(ws.active_runtime().tab_history.last(), Some(&1));
    });

    // Move tab A (0) to tail position (2): [A,B,C] → [B,C,A]
    cx.update_window(window_handle.into(), |_, _window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.move_tab(0, 2, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        // Tab order: B, C, A.
        assert_eq!(ws.active_runtime().tabs[0].id, ids[1]); // B
        assert_eq!(ws.active_runtime().tabs[1].id, ids[2]); // C
        assert_eq!(ws.active_runtime().tabs[2].id, ids[0]); // A

        // active_tab_index was 0 (A), which moved to 2.
        assert_eq!(ws.active_runtime().active_tab_index, 2);

        // History had entry 1 (B) at the top → B is now index 0 (shifted down).
        // All history entries that referenced A (old 0) map to new 2.
        // All history entries that referenced indices >0 and ≤2 shift −1.
        // Original history (top-down): [..., 2(C), 0(A), 1(B)]
        //   2(C) → from<to (0<2) && 2>0 && 2<=2 → 2−1 = 1 ✓
        //   0(A) → idx==from → to = 2 ✓
        //   1(B) → from<to (0<2) && 1>0 && 1<=2 → 1−1 = 0 ✓
        assert!(ws.active_runtime().tab_history.contains(&2)); // A's new position
        assert!(ws.active_runtime().tab_history.contains(&0)); // B's new position
        assert!(ws.active_runtime().tab_history.contains(&1)); // C's new position
    });

    // Same three-tab fixture: closing a non-active tab first prunes the history
    // entry pointing at it; closing the active tab then falls back to the
    // remaining valid history entry.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.activate_tab(2, window, cx);
            ws.activate_tab(1, window, cx);
        });
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().active_tab_index, 1);
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
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1); // active tab 1 unaffected (index < 2)
    });

    // Close active tab 1 — history has [0] which is valid → should navigate to 0.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.close_tab_at(1, window, cx);
        });
    })
    .unwrap();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().active_tab_index, 0);
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
        ws.active_runtime().panes[0]
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

/// Splitting with `NewPaneKind::AgentChat` fills the new leaf with an agent
/// chat pane, preserves the source terminal, and makes later splits inherit
/// the focused agent-chat kind.
#[gpui::test]
async fn agent_chat_splits_preserve_source_and_chain_by_focused_kind(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.focused_pane_split_kind(), NewPaneKind::Terminal);
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        // One tab, two panes, split horizontally.
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_runtime().panes.len(), 2);
        assert!(matches!(
            ws.active_runtime().tabs[0].layout,
            PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                ..
            }
        ));
        // The newly focused pane is the agent chat leaf; the original stays a
        // terminal. The default test workspace has no lane cwd, so the chat
        // pane parks offline (no subprocess) — see tests/agent_chat.rs.
        let focused = ws.active_runtime().focused_pane_id;
        let new_pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused)
            .expect("focused pane present after split");
        assert!(
            matches!(new_pane.content, PaneContent::AgentChat(_)),
            "split-off pane is an agent chat pane"
        );
        // The original pane must remain a terminal — splitting with an
        // AgentChat kind converts only the new leaf, never the existing one.
        let original = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id != focused)
            .expect("original pane present after split");
        assert!(
            matches!(original.content, PaneContent::Terminal(_)),
            "original pane stays a terminal after an agent-chat split"
        );
        assert_eq!(ws.focused_pane_split_kind(), NewPaneKind::AgentChat);
    });

    // Focus is now on the first agent-chat pane; split it again.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Vertical,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        // Original terminal + two agent-chat panes.
        assert_eq!(ws.active_runtime().panes.len(), 3);
        assert_eq!(ws.active_runtime().tabs[0].layout.leaf_count(), 3);
        let nested_vertical = match &ws.active_runtime().tabs[0].layout {
            PaneLayout::Split {
                direction: SplitDirection::Horizontal,
                children,
                ..
            } => matches!(
                children.get(1),
                Some(PaneLayout::Split {
                    direction: SplitDirection::Vertical,
                    ..
                })
            ),
            _ => false,
        };
        assert!(
            nested_vertical,
            "second agent-chat split should preserve the requested vertical direction"
        );
        let agent_chat_count = ws
            .active_runtime()
            .panes
            .iter()
            .filter(|p| matches!(p.content, PaneContent::AgentChat(_)))
            .count();
        assert_eq!(
            agent_chat_count, 2,
            "two agent-chat panes coexist in one tab"
        );
    });
}
