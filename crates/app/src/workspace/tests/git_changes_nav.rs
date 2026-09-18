//! Keyboard skimming in the Git Changes dock.
//!
//! The panel's keybindings only fire under `key_context("GitChanges")`, which
//! needs `git_changes_panel_focus` to hold focus. Everything a panel row opens
//! therefore goes through `OpenIntent::Preview` or `Commit`, neither of which
//! enters the pane — one that did would end the keyboard session after a
//! single file. That, and which tab a later skim is allowed to take, are the
//! invariants these tests pin.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};

use super::Workspace;
use super::git_changes_virtualized::dock_showing_changes;

fn entry(path: &str) -> crate::lane::git::GitFileEntry {
    crate::lane::git::GitFileEntry {
        x: ' ',
        y: 'M',
        path: PathBuf::from(path),
        original_path: None,
    }
}

/// Paths of every file-viewer pane open in the active lane.
fn open_file_paths(ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext) -> Vec<PathBuf> {
    ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.file_view())
            .map(|fv| fv.path.clone())
            .collect()
    })
}

#[gpui::test]
async fn enter_opens_the_diff_and_keeps_the_panel_focused(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs"), entry("src/b.rs")]);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            let lane = ws.active.lane;
            ws.set_git_changes_cursor(lane, PathBuf::from("src/a.rs"), cx);
            ws.activate_git_changes_cursor(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    let open = open_file_paths(&ws, cx);
    assert_eq!(open.len(), 1, "Enter must open the cursor's file");
    assert!(
        open[0].ends_with("src/a.rs"),
        "opened {:?}, expected the cursor's file",
        open[0]
    );

    let focused = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        focused,
        "Enter handed the panel's focus to the file viewer, so the next arrow \
         key no longer reaches key_context(\"GitChanges\") — the click path \
         restores it and this one must too"
    );
}

#[gpui::test]
async fn arrow_navigation_previews_the_cursor_file_without_taking_focus(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs"), entry("src/b.rs")]);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            ws.move_git_changes_cursor(1, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    assert!(
        open_file_paths(&ws, cx).is_empty(),
        "the preview must wait out the debounce — opening on the keystroke \
         spends a file read, a `git diff` and a state write per arrow key"
    );

    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();

    let open = open_file_paths(&ws, cx);
    assert_eq!(open.len(), 1, "a settled cursor must preview its file");
    assert!(
        open[0].ends_with("src/a.rs"),
        "previewed {:?}, expected the first row",
        open[0]
    );

    let focused = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        focused,
        "the preview took the panel's focus, so the arrow key that opened it \
         cannot be followed by another"
    );
}

/// Skimming is the point: holding an arrow key walks rows faster than any of
/// them can load. Every row the cursor merely passes over must leave nothing
/// behind — no load, no tab, no state write.
#[gpui::test]
async fn a_row_the_cursor_only_passes_over_is_never_previewed(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs"), entry("src/b.rs")]);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            ws.move_git_changes_cursor(1, window, cx); // → src/a.rs
        });
    })
    .unwrap();
    // Short of a.rs's delay: the cursor moves on before it would have loaded.
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_git_changes_cursor(1, window, cx)); // → src/b.rs
    })
    .unwrap();
    // Now past where a.rs's timer would have fired, still short of b.rs's.
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert!(
        open_file_paths(&ws, cx).is_empty(),
        "src/a.rs's superseded timer still fired — re-arming must cancel the \
         row the cursor left, not queue it"
    );

    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let open = open_file_paths(&ws, cx);
    assert_eq!(open.len(), 1, "only the row the cursor settled on opens");
    assert!(
        open[0].ends_with("src/b.rs"),
        "previewed {:?}, expected the row the cursor settled on",
        open[0]
    );
}

/// The same rule on the other side of the dock: a Git Changes row click
/// previews and the panel keeps focus. Both panels now get this from
/// `OpenIntent::Preview` inside the opener rather than from each row handler
/// remembering to take focus back.
#[gpui::test]
async fn a_git_changes_row_click_previews_and_leaves_the_panel_focused(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs")]);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            let lane = ws.active.lane;
            ws.on_git_changes_row_click(lane, PathBuf::from("src/a.rs"), false, 1, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    let open = open_file_paths(&ws, cx);
    assert_eq!(open.len(), 1, "a click opens the diff");
    assert!(open[0].ends_with("src/a.rs"), "opened {:?}", open[0]);
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_scoped[&ws.active]
                .git
                .cursor
                .as_ref()
                .map(|c| c.path.as_path()),
            Some(Path::new("src/a.rs")),
            "the clicked row becomes the keyboard cursor, so arrows resume from it"
        );
    });

    let focused = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        focused,
        "the click handed the panel's focus to the file viewer"
    );
}

/// Keyboard skimming is unreachable unless the keyboard can get into the
/// panel first: Cmd+B opens the left dock without focusing it, and nothing
/// else focuses a left-dock panel. One binding has to switch the dock to the
/// view, open it, and take focus — then hand focus back to the pane when
/// pressed again, so the same key round-trips (zed's `toggle_panel_focus`).
#[gpui::test]
async fn toggling_git_changes_focus_switches_the_view_opens_the_dock_and_round_trips(
    cx: &mut TestAppContext,
) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs")]);
    // Start from the far side: dock shut, showing another view, focus on a pane.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_left_dock_view(daruda_store::project::LeftDockView::Lanes, cx);
            ws.left_dock.update(cx, |d, cx| {
                d.is_open = false;
                cx.notify();
            });
            // The fixture opens no pane, and a round-trip needs somewhere to
            // land — this is the "work" the binding must return focus to.
            ws.add_tab(window, cx);
            let pane = ws.active_runtime().focused_pane_id;
            ws.focus_pane(pane, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_git_changes_focus(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let focused = cx
        .update_window(w.into(), |_, window, cx| {
            let ws = ws.read(cx);
            (
                ws.left_dock.read(cx).is_open,
                ws.left_dock_view,
                ws.git_changes_panel_focus.is_focused(window),
            )
        })
        .unwrap();
    assert!(
        focused.0,
        "the dock must open — a focused but hidden panel is nothing"
    );
    assert_eq!(
        focused.1,
        daruda_store::project::LeftDockView::GitChanges,
        "the dock must switch to the view the binding names"
    );
    assert!(focused.2, "the panel must take keyboard focus");

    // Same binding again: back to the work.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_git_changes_focus(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let still_focused = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        !still_focused,
        "a second press must return focus to the pane, or the binding is a \
         one-way trip out of the editor"
    );
}

/// Enter keeps the panel focused so the arrows keep working, which leaves no
/// keyboard way into the diff itself. zed resolves this by making the second
/// Enter on an already-open row mean "go in" (`git_panel::open_diff` focuses
/// the ProjectDiff when it is already showing that entry). Same rule here.
#[gpui::test]
async fn a_second_enter_on_the_open_row_steps_into_the_viewer(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs"), entry("src/b.rs")]);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            let lane = ws.active.lane;
            ws.set_git_changes_cursor(lane, PathBuf::from("src/a.rs"), cx);
            ws.activate_git_changes_cursor(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    let after_first = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(after_first, "the first Enter opens and stays in the panel");

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_git_changes_cursor(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let after_second = cx
        .update_window(w.into(), |_, window, cx| {
            ws.read(cx).git_changes_panel_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        !after_second,
        "a second Enter on the row already open must step into the viewer — \
         otherwise the diff is unreachable without the mouse"
    );
    assert_eq!(
        open_file_paths(&ws, cx).len(),
        1,
        "stepping in must not open the file a second time"
    );
}

/// Previewing is not entering. `focus_pane` surfaces the bottom dock's Input
/// panel for the pane it focuses — right for a pane the user walked into,
/// wrong for a row the cursor merely rested on, which would swap the macro
/// grid out from under them on every arrow key.
#[gpui::test]
async fn previewing_a_row_does_not_surface_the_bottom_input(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("src/a.rs")]);
    assert!(
        !ws.read_with(cx, |ws, _| ws.terminal_input_visible),
        "fixture must start on the macro grid or this proves nothing"
    );

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            ws.move_git_changes_cursor(1, window, cx);
        });
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();

    assert_eq!(
        open_file_paths(&ws, cx).len(),
        1,
        "the preview must still have opened, or the assertion below is vacuous"
    );
    assert!(
        !ws.read_with(cx, |ws, _| ws.terminal_input_visible),
        "the preview surfaced the bottom Input panel, replacing the macro grid \
         for a pane the user never entered"
    );

    // Entering it deliberately still does surface the input.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_git_changes_cursor(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        ws.read_with(cx, |ws, _| ws.terminal_input_visible),
        "stepping into the viewer is entering a pane, and must surface its input"
    );
}

/// Preview-tab mode reuses a file tab so skimming does not bury the user in
/// tabs — but it reused *any* file tab, so a file opened deliberately was
/// taken over by the next arrow key. Only the tab a preview itself created is
/// the one a later preview may replace; anything the user committed to stays.
#[gpui::test]
async fn a_preview_replaces_only_the_tab_a_preview_opened(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(
        cx,
        vec![entry("src/a.rs"), entry("src/b.rs"), entry("src/c.rs")],
    );
    let file_tabs = |ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext| {
        ws.read_with(cx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .filter(|p| p.file_view().is_some())
                .count()
        })
    };

    // Enter is the deliberate open: the user picked this file.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            let lane = ws.active.lane;
            ws.set_git_changes_cursor(lane, PathBuf::from("src/a.rs"), cx);
            ws.activate_git_changes_cursor(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(file_tabs(&ws, cx), 1);

    // Skimming past two more rows must not land on top of it.
    for _ in 0..2 {
        cx.update_window(w.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| ws.move_git_changes_cursor(1, window, cx));
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
    }

    let open = open_file_paths(&ws, cx);
    assert!(
        open.iter().any(|p| p.ends_with("src/a.rs")),
        "the file the user opened with Enter was replaced by a preview; open: {open:?}"
    );
    assert_eq!(
        open.len(),
        2,
        "the two skimmed rows must share one preview tab beside it; open: {open:?}"
    );
}

/// Files leave the list while the user is reading it — an agent reverts one, a
/// discard lands, a group is collapsed. The cursor's file is then gone, and
/// navigation used to restart from the top of whatever remained, throwing away
/// the user's place in a list they were halfway down.
#[gpui::test]
async fn a_cursor_whose_file_vanished_resumes_in_place(cx: &mut TestAppContext) {
    let files = ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"];
    let (w, ws) = dock_showing_changes(cx, files.iter().map(|p| entry(p)).collect());
    let active = ws.read_with(cx, |ws, _| ws.active);

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
            let lane = ws.active.lane;
            ws.set_git_changes_cursor(lane, PathBuf::from("c.rs"), cx);
        });
    })
    .unwrap();

    // `c.rs` leaves the change set; the rows below it slide up one.
    ws.update(cx, |ws, _| {
        ws.lane_scoped_mut(active).git.worktree = Some(crate::lane::git::GitWorktreeStatus {
            unstaged: ["a.rs", "b.rs", "d.rs", "e.rs"]
                .iter()
                .map(|p| entry(p))
                .collect(),
            ..Default::default()
        });
    });
    cx.run_until_parked();

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_git_changes_cursor(1, window, cx));
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.lane_scoped[&active]
                .git
                .cursor
                .as_ref()
                .map(|c| c.path.clone()),
            Some(PathBuf::from("d.rs")),
            "the arrow must resume at the row the cursor held — `d.rs` slid \
             into `c.rs`'s place — not restart at the top of the list"
        );
    });
}

/// Re-focus the panel, as the user would before using the arrows again.
fn refocus_panel(
    w: gpui::WindowHandle<Workspace>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) {
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.git_changes_panel_focus.clone().focus(window, cx);
        });
    })
    .unwrap();
}

fn arrow(
    w: gpui::WindowHandle<Workspace>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    delta: isize,
) {
    refocus_panel(w, ws, cx);
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.move_git_changes_cursor(delta, window, cx));
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}

/// Only the open that *filled* a tab may claim the scratch slot. Re-activating
/// a tab that already holds the file must not, or skimming back over a row the
/// user opened with Enter quietly makes it replaceable again — and the next
/// arrow key takes it.
#[gpui::test]
async fn skimming_back_over_a_committed_row_does_not_make_it_replaceable(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("a.rs"), entry("b.rs"), entry("c.rs")]);

    // Enter on a.rs — the deliberate open.
    refocus_panel(w, &ws, cx);
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let lane = ws.active.lane;
            ws.set_git_changes_cursor(lane, PathBuf::from("a.rs"), cx);
            ws.activate_git_changes_cursor(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    arrow(w, &ws, cx, 1); // → b.rs, opens the scratch tab beside a.rs
    arrow(w, &ws, cx, -1); // → back onto a.rs, which is already open
    arrow(w, &ws, cx, 2); // → c.rs, which reuses whatever the scratch tab is

    let open = open_file_paths(&ws, cx);
    assert!(
        open.iter().any(|p| p.ends_with("a.rs")),
        "the row opened with Enter was taken over after the cursor passed back \
         across it; open now: {open:?}"
    );
}

/// Walking into a pane is a commit. A row the arrow preview opened and the
/// user then entered with Enter must stop being replaceable — otherwise the
/// file they are reading is taken by the next arrow key.
#[gpui::test]
async fn entering_a_previewed_row_commits_its_tab(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("a.rs"), entry("b.rs")]);

    arrow(w, &ws, cx, 1); // → a.rs, previewed into the scratch tab
    assert_eq!(open_file_paths(&ws, cx).len(), 1);

    // Enter steps into the viewer, since the preview already opened it.
    refocus_panel(w, &ws, cx);
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_git_changes_cursor(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    arrow(w, &ws, cx, 1); // → b.rs

    let open = open_file_paths(&ws, cx);
    assert!(
        open.iter().any(|p| p.ends_with("a.rs")),
        "the row the user entered was still the scratch tab, so the next arrow \
         key replaced the file they had walked into; open now: {open:?}"
    );
}

/// GPUI does not blur a focus handle when its element unmounts, so a panel
/// whose dock the user shut with Cmd+B still reports itself focused. Reading
/// only that made the binding conclude "you are already here" and hand focus
/// to the pane, leaving the dock shut — a dead keypress the user has to repeat.
#[gpui::test]
async fn the_focus_binding_reopens_a_dock_the_user_closed(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("a.rs")]);
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.toggle_git_changes_focus(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    // Cmd+B: the dock closes while its panel still holds keyboard focus.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.on_toggle_left_dock(&crate::workspace::ToggleLeftDock, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.toggle_git_changes_focus(window, cx));
    })
    .unwrap();
    cx.run_until_parked();

    let (open, focused) = cx
        .update_window(w.into(), |_, window, cx| {
            let r = ws.read(cx);
            (
                r.left_dock.read(cx).is_open,
                r.git_changes_panel_focus.is_focused(window),
            )
        })
        .unwrap();
    assert!(open, "the binding must reopen the dock it was pressed for");
    assert!(focused, "and put the user back in the panel");
}

/// The preview fires 150 ms after the key. Leave the panel inside that window
/// — step into the viewer, go back to the pane — and the timer would still
/// switch the active tab, moving the user somewhere they had already left.
#[gpui::test]
async fn a_preview_armed_before_leaving_the_panel_does_not_fire(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("a.rs"), entry("b.rs")]);

    refocus_panel(w, &ws, cx);
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            ws.git_changes_panel_focus.clone().focus(window, cx);
            ws.move_git_changes_cursor(1, window, cx);
            // The user leaves the panel before the timer elapses.
            let pane = ws.active_runtime().focused_pane_id;
            ws.focus_pane(pane, window, cx);
        });
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();

    assert!(
        open_file_paths(&ws, cx).is_empty(),
        "a preview armed for a panel the user has left must not fire — it \
         switches the active tab out from under them"
    );
}

/// Closing the scratch tab leaves `preview_tab_id` pointing at a tab that no
/// longer exists. Tab ids are never reused, so the stale id can only fail to
/// resolve — but that is a property of `alloc_id`, not of this code, and the
/// next preview must open cleanly rather than land on whatever now sits at
/// that index.
#[gpui::test]
async fn closing_the_scratch_tab_leaves_the_next_preview_intact(cx: &mut TestAppContext) {
    let (w, ws) = dock_showing_changes(cx, vec![entry("a.rs"), entry("b.rs")]);
    // A second tab, so closing the scratch one is a real close —
    // `close_tab_at` closes the *window* when it would empty the last tab.
    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.add_tab(window, cx));
    })
    .unwrap();

    arrow(w, &ws, cx, 1); // → a.rs into the scratch tab
    let scratch = ws.read_with(cx, |ws, cx| ws.preview_tab_index(cx));
    assert_eq!(
        scratch,
        Some(ws.read_with(cx, |ws, _| ws.active_runtime().active_tab_index))
    );

    cx.update_window(w.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let idx = ws.active_runtime().active_tab_index;
            ws.close_tab_at(idx, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.preview_tab_index(cx),
            None,
            "a closed tab must not still read as the scratch slot"
        );
    });

    arrow(w, &ws, cx, 1); // → b.rs
    let open = open_file_paths(&ws, cx);
    assert_eq!(open.len(), 1, "the next preview opens its own tab");
    assert!(open[0].ends_with("b.rs"), "opened {:?}", open[0]);
}
