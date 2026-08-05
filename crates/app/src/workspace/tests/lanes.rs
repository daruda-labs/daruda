use super::*;

// ---- Lanes ----

#[gpui::test]
fn test_workspace_with_project_bootstraps_one_default_lane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_bootstrap_wt");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_lanes().len(), 1);
        assert_eq!(ws.active_lanes()[0].id, 0);
        assert_eq!(ws.active.lane, 0);
        assert_eq!(
            ws.active_lanes()[0].kind,
            daruda_store::project::LaneKind::Default
        );
        assert_eq!(
            ws.active_lanes()[0].path,
            std::path::PathBuf::from("/tmp/test_bootstrap_wt")
        );
        assert!(
            ws.git_repo_root().is_none(),
            "a non-git Default lane must not expose a repo root"
        );

        let proj = ws.active_ref().project;
        let unknown = daruda_store::project::LaneRef {
            project: proj,
            lane: 999,
        };
        let err = ws.validate_remove_lane(unknown).unwrap_err();
        assert!(err.contains("not found"));

        let default_lane = daruda_store::project::LaneRef {
            project: proj,
            lane: 0,
        };
        let err = ws.validate_remove_lane(default_lane).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Nonexistent id → no-op.
            let proj = ws.active_ref().project;
            let mk = |id| daruda_store::project::LaneRef {
                project: proj,
                lane: id,
            };
            ws.activate_lane(mk(99), window, cx);
            assert_eq!(ws.active.lane, 0);
            // Self id → no-op (already active).
            ws.activate_lane(mk(0), window, cx);
            assert_eq!(ws.active.lane, 0);
        });
    })
    .unwrap();
}

#[test]
fn lane_removable_respects_default_main_and_kind_updates() {
    let default_wt = crate::lane::Lane::default_for_project(0, std::path::PathBuf::from("/tmp"));
    assert!(!Workspace::lane_removable(&default_wt));

    let main_git = crate::lane::Lane::git(
        0,
        std::path::PathBuf::from("/tmp/repo"),
        Some("main".into()),
        std::path::PathBuf::from("/tmp/repo"),
        std::path::PathBuf::from("/tmp/repo"),
        0,
    );
    assert!(!Workspace::lane_removable(&main_git));

    let linked = crate::lane::Lane::git(
        1,
        std::path::PathBuf::from("/tmp/repo-feat"),
        Some("feat".into()),
        std::path::PathBuf::from("/tmp/repo"),
        std::path::PathBuf::from("/tmp/repo-feat"),
        1,
    );
    assert!(Workspace::lane_removable(&linked));

    // Regression: `path != repo_root` wrongly made a main lane removable when
    // the user opened a subdirectory. `worktree_root == repo_root` is the
    // correct signal.
    let subdir_main = crate::lane::Lane::git(
        0,
        std::path::PathBuf::from("/repo/sub"),
        Some("main".into()),
        std::path::PathBuf::from("/repo"),
        std::path::PathBuf::from("/repo"),
        0,
    );
    assert!(
        subdir_main.is_main,
        "precondition: subdir-anchored main lane must have is_main=true"
    );
    assert!(
        !Workspace::lane_removable(&subdir_main),
        "subdir-anchored main lane must NOT be removable"
    );

    // Regression: `set_kind` must recompute `is_main` atomically so a lane
    // promoted from `Default` to `Git` becomes non-removable immediately.
    let mut lane = crate::lane::Lane::default_for_project(0, std::path::PathBuf::from("/tmp/repo"));
    assert!(!lane.is_main, "precondition: Default lane is not main");
    assert!(
        !Workspace::lane_removable(&lane),
        "precondition: Default lane is not removable either"
    );

    lane.set_kind(daruda_store::project::LaneKind::Git {
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        branch: Some("main".into()),
        worktree_root: std::path::PathBuf::from("/tmp/repo"),
    });
    assert!(
        lane.is_main,
        "after set_kind with worktree_root == repo_root, is_main must be true"
    );
    assert!(
        !Workspace::lane_removable(&lane),
        "a freshly git-init'd main lane must NOT be removable"
    );
}

/// Regression: an external `git checkout` reaches the lane only via a
/// `git status` refresh, which must rewrite the recorded `kind.branch`
/// (the source the left-dock label reads). Drift updates the branch and
/// preserves the repo roots; a matching branch is a no-op.
#[gpui::test]
fn reconcile_lane_branch_updates_kind_on_drift(cx: &mut TestAppContext) {
    let project = daruda_store::project::Project::from_path("/tmp/test_reconcile_branch");
    let (_wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    ws.update(cx, |ws, cx| {
        let target = ws.active_ref();
        // Seed lane 0 as a git main lane currently on branch "old".
        ws.active_project_mut()
            .expect("active project")
            .lane_mut(target.lane)
            .expect("active lane")
            .set_kind(daruda_store::project::LaneKind::Git {
                branch: Some("old".into()),
                repo_root: std::path::PathBuf::from("/tmp/test_reconcile_branch"),
                worktree_root: std::path::PathBuf::from("/tmp/test_reconcile_branch"),
            });

        // Live status reports a different branch — it must propagate.
        ws.reconcile_lane_branch(target, Some("new"), cx);
        let branch_after = ws.lane_for(target).and_then(|l| match &l.kind {
            daruda_store::project::LaneKind::Git { branch, .. } => branch.clone(),
            _ => None,
        });
        assert_eq!(
            branch_after,
            Some("new".to_string()),
            "drifted branch must propagate into kind.branch"
        );

        // Same branch — idempotent no-op; roots stay intact.
        ws.reconcile_lane_branch(target, Some("new"), cx);
        match &ws.lane_for(target).expect("lane").kind {
            daruda_store::project::LaneKind::Git {
                branch,
                repo_root,
                worktree_root,
            } => {
                assert_eq!(branch.as_deref(), Some("new"));
                assert_eq!(
                    repo_root,
                    std::path::Path::new("/tmp/test_reconcile_branch")
                );
                assert_eq!(
                    worktree_root,
                    std::path::Path::new("/tmp/test_reconcile_branch")
                );
            }
            _ => panic!("expected git lane after reconcile"),
        }
    });
}

#[gpui::test]
fn test_activate_lane_swaps_tabs(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Both lane roots must exist on disk so activation classifies them as
    // `Present` and `add_tab` can spawn a pane rooted at the lane path.
    let root_a = std::path::PathBuf::from("/tmp/test_wt_swap");
    let root_b = std::path::PathBuf::from("/tmp/test_wt_swap_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Seed a second lane with empty runtime so we can swap into it.
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    // Swap to lane 1 → both lanes' runtimes coexist in the single
    // `runtimes` map (active just re-points to lane 1). Activation does
    // not auto-seed, so open one tab explicitly to give lane 1 content.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.active_runtime().tabs.len(), 1);
            assert_eq!(ws.active_runtime().panes.len(), 1);
            let proj = ws.active_ref().project;
            ws.activate_lane(
                daruda_store::project::LaneRef {
                    project: proj,
                    lane: 1,
                },
                window,
                cx,
            );
            assert_eq!(ws.active.lane, 1);
            // Freshly activated lane starts empty (no auto-seed).
            assert!(ws.active_runtime().tabs.is_empty());
            assert!(ws.active_runtime().panes.is_empty());
            assert!(!ws.has_focused_pane());
            ws.add_tab(window, cx);
            // Now exactly one tab/pane for lane 1.
            assert_eq!(ws.active_runtime().tabs.len(), 1);
            assert_eq!(ws.active_runtime().panes.len(), 1);
            // Both lanes registered in the single store.
            assert_eq!(ws.main_area.runtimes.len(), 2);
            let proj = ws.active_ref().project;
            let lane0 = daruda_store::project::LaneRef {
                project: proj,
                lane: 0,
            };
            let lane1 = daruda_store::project::LaneRef {
                project: proj,
                lane: 1,
            };
            let index = ws.pane_lane_index();
            assert!(
                index.iter().any(|(_, lane)| *lane == lane0),
                "parked lane 0's pane must remain in the index"
            );
            assert!(
                index.iter().any(|(_, lane)| *lane == lane1),
                "active lane 1's pane must be in the index"
            );
            // Lane 0 is now the parked entry; its runtime is untouched.
            let parked = ws
                .main_area
                .runtimes
                .get(&daruda_store::project::LaneRef {
                    project: proj,
                    lane: 0,
                })
                .unwrap();
            assert_eq!(parked.tabs.len(), 1);
            assert_eq!(parked.panes.len(), 1);
        });
    })
    .unwrap();

    // Swap back → lane 0's runtime is still there (same PTY tasks, same
    // pane ids); active just re-points to it. Lane 1's freshly-spawned
    // runtime stays in the map as the parked entry.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let proj = ws.active_ref().project;
            let lane1 = daruda_store::project::LaneRef {
                project: proj,
                lane: 1,
            };
            ws.activate_lane(
                daruda_store::project::LaneRef {
                    project: proj,
                    lane: 0,
                },
                window,
                cx,
            );
            assert_eq!(ws.active.lane, 0);
            assert_eq!(ws.active_runtime().tabs.len(), 1);
            assert_eq!(ws.active_runtime().panes.len(), 1);
            assert_eq!(ws.main_area.runtimes.len(), 2);
            let parked = ws.main_area.runtimes.get(&lane1).unwrap();
            // Lane 1 carries its lazy-spawned pane now.
            assert_eq!(parked.tabs.len(), 1);
            assert_eq!(parked.panes.len(), 1);

            // Closing the active lane's last tab must not close the window
            // while a sibling lane still holds content.
            ws.activate_lane(lane1, window, cx);
            ws.close_tab_at(0, window, cx);
            assert!(
                ws.active_runtime().tabs.is_empty(),
                "active lane must be emptied, not closed with the window"
            );
            assert!(ws.active_runtime().panes.is_empty());
            assert!(!ws.has_focused_pane());
            assert_eq!(
                ws.main_area
                    .runtimes
                    .get(&daruda_store::project::LaneRef {
                        project: proj,
                        lane: 0,
                    })
                    .map(|rt| rt.tabs.len()),
                Some(1),
                "the sibling lane's runtime must survive"
            );
            ws.add_tab(window, cx);
            assert_eq!(
                ws.active_runtime().tabs.len(),
                1,
                "a fresh tab is reopened in the emptied lane"
            );
            assert!(ws.has_focused_pane(), "the reopened tab is focused");
        });
    })
    .unwrap();
    assert_eq!(cx.windows().len(), 1, "the window must stay open");
}

#[gpui::test]
fn welcome_workspace_has_seeded_active_runtime(cx: &mut TestAppContext) {
    // Regression: with the single `runtimes` store, the Welcome state
    // (no project, `active == LaneRef::default()`, no `add_tab`) must
    // still carry a seeded runtime so `render`'s unconditional
    // `active_runtime()` read (and every accessor it drives) resolves
    // instead of panicking on a missing map entry. Construct via the
    // bare test constructor — it returns before `add_tab`, so the only
    // entry in `runtimes` is the constructor's unconditional seed.
    let config = daruda_config::Config::default();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.projects.is_empty());
        assert!(ws.active_lanes().is_empty());
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
        // The accessor would `expect`-panic if the default ref had no
        // entry; a clean empty read proves the constructor seed ran.
        assert!(ws.active_runtime().tabs.is_empty());
        assert!(ws.active_runtime().panes.is_empty());
        assert!(ws.main_area.runtimes.contains_key(&ws.active));
    });
}

#[gpui::test]
fn finalize_remove_active_lane_keeps_main_area_filled(cx: &mut TestAppContext) {
    // Removing the *active* lane must re-point to a sibling and refill
    // the viewport — never leave the main area blank.
    let config = daruda_config::Config::default();
    let repo = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo");
    let feature = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo-feature");
    let _ = std::fs::create_dir_all(&repo);
    let _ = std::fs::create_dir_all(&feature);
    let project = daruda_store::project::Project::from_path(&repo);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Replace the bootstrapped Default lane with a git main lane (id 0,
    // path == repo_root → non-removable) plus a removable linked lane
    // (id 1, a distinct path).
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes = vec![
                crate::lane::Lane::git(
                    0,
                    repo.clone(),
                    Some("main".into()),
                    repo.clone(),
                    repo.clone(),
                    0,
                ),
                crate::lane::Lane::git(
                    1,
                    feature.clone(),
                    Some("feature".into()),
                    repo.clone(),
                    feature.clone(),
                    1,
                ),
            ];
        }
    });

    let proj = ws.read_with(cx, |ws, _| ws.active_ref().project);
    let lane0 = daruda_store::project::LaneRef {
        project: proj,
        lane: 0,
    };
    let lane1 = daruda_store::project::LaneRef {
        project: proj,
        lane: 1,
    };

    let pane_ids = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.activate_lane(lane1, window, cx);
                assert_eq!(ws.active, lane1, "removable lane is active before removal");
                ws.add_tab(window, cx);
                let pane_ids: Vec<_> = ws
                    .active_runtime()
                    .tabs
                    .iter()
                    .flat_map(|t| t.layout.pane_ids())
                    .collect();
                assert!(!pane_ids.is_empty(), "opened tab spawns panes");
                for id in &pane_ids {
                    ws.claude.pty_tracker.register(*id, 4242);
                    ws.claude.pty_claude_bindings.insert(
                        *id,
                        crate::hooks::pty_tracker::PtyBinding {
                            claude_pid: 4242,
                            session_id: format!("sess-{id}"),
                        },
                    );
                }
                ws.finalize_remove_lane(lane1, window, cx);
                pane_ids
            })
        })
        .unwrap();

    ws.read_with(cx, |ws, _| {
        // Fell back to the surviving main lane.
        assert_eq!(ws.active, lane0);
        let p = ws.active_project().unwrap();
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(p.lanes[0].id, 0);
        // The removed active lane must not blank the viewport.
        assert!(
            !ws.active_runtime().tabs.is_empty(),
            "main area tabs must survive removing the active lane"
        );
        assert!(!ws.active_runtime().panes.is_empty());
        // The removed lane's runtime is dropped from the single store.
        assert!(
            !ws.main_area.runtimes.contains_key(&lane1),
            "removed lane's runtime must be cleared"
        );
        let tracked = ws.claude.pty_tracker.tracked_pane_ids();
        for id in &pane_ids {
            assert!(
                !ws.claude.pty_claude_bindings.contains_key(id),
                "removed lane's pane binding must be dropped"
            );
            assert!(
                !tracked.contains(id),
                "removed lane's pane must be unregistered from the tracker"
            );
        }
    });

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&feature);
}

/// Verify per-lane input draft separation:
/// - Typing in lane A and switching to lane B clears the input (B has no draft).
/// - Typing in lane B and switching back to lane A restores A's draft.
/// - Submitting (`send_terminal_input`) drops the lane's saved draft.
#[gpui::test]
fn input_draft_is_per_pane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root_a = std::path::PathBuf::from("/tmp/test_input_draft_a");
    let root_b = std::path::PathBuf::from("/tmp/test_input_draft_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Seed a second lane.
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    let proj = ws.read_with(cx, |ws, _| ws.active_ref().project);
    let lane0 = daruda_store::project::LaneRef {
        project: proj,
        lane: 0,
    };
    let lane1 = daruda_store::project::LaneRef {
        project: proj,
        lane: 1,
    };

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // The test constructor skips the construction `add_tab`, so seed
            // lane 0's terminal pane explicitly; that pane becomes the draft
            // owner (drafts are keyed per input pane, not per lane).
            ws.add_tab(window, cx);
            let pane0 = ws.active_runtime().focused_pane_id;

            // Lane 0's terminal pane is focused; type a draft into the input.
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("draft-a", window, cx_state));

            // Switch to lane 1 and open its own pane (activation doesn't
            // auto-seed one); that pane takes focus, so draft-a is saved for
            // pane 0 and the input is cleared (lane 1's pane has no draft).
            ws.activate_lane(lane1, window, cx);
            ws.add_tab(window, cx);
            assert_eq!(ws.active, lane1);
            let pane1 = ws.active_runtime().focused_pane_id;
            assert_ne!(pane1, pane0, "each lane has its own input pane");
            let after_switch = ws.terminal_input.read(cx).value().to_string();
            assert_eq!(
                after_switch, "",
                "input must be empty after focusing a pane with no draft"
            );
            // The saved draft for pane 0 must be in the map.
            assert_eq!(
                ws.input_drafts.get(&pane0).map(String::as_str),
                Some("draft-a"),
                "pane 0 draft must be persisted on focus away"
            );

            // Type a draft for lane 1's pane.
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("draft-b", window, cx_state));

            // Switch back to lane 0 — pane 1's draft must be saved and
            // pane 0's draft ("draft-a") must be restored.
            ws.activate_lane(lane0, window, cx);
            assert_eq!(ws.active, lane0);
            let restored = ws.terminal_input.read(cx).value().to_string();
            assert_eq!(
                restored, "draft-a",
                "pane 0 draft must be restored on focus back"
            );
            assert_eq!(
                ws.input_drafts.get(&pane1).map(String::as_str),
                Some("draft-b"),
                "pane 1 draft must be persisted on focus away"
            );

            // And forward to lane 1 again shows its own draft.
            ws.activate_lane(lane1, window, cx);
            assert_eq!(
                ws.terminal_input.read(cx).value().to_string(),
                "draft-b",
                "pane 1 draft must be restored on focus back"
            );

            ws.activate_lane(lane0, window, cx);
            assert_eq!(
                ws.terminal_input.read(cx).value().to_string(),
                "draft-a",
                "pane 0 draft must still survive the lane round-trip"
            );

            // Submit pane 0's draft — the saved entry must be cleared.
            ws.send_terminal_input(window, cx);
            assert_eq!(
                ws.input_drafts.get(&pane0),
                None,
                "submitting must drop the owner pane's saved draft"
            );
            // The input widget itself must also be empty after send.
            let after_send = ws.terminal_input.read(cx).value().to_string();
            assert_eq!(after_send, "", "input must be empty after send");
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// Same-lane input panes keep independent drafts, non-input panes leave the
/// active draft owner alone, and closing an input pane drops its saved draft.
#[gpui::test]
fn input_draft_round_trips_across_panes_and_cleans_up(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::FileViewMode;

    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/test_input_draft_roundtrip");
    let _ = std::fs::create_dir_all(&root);
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Two terminal panes in the same lane, one per tab: the
            // constructor opens tab 0, and a second `add_tab` opens tab 1.
            let pane_a = ws.active_runtime().focused_pane_id;
            ws.add_tab(window, cx);
            let pane_b = ws.active_runtime().focused_pane_id;
            assert_ne!(pane_a, pane_b, "each tab seeds its own input pane");

            // Focus A and type a draft.
            ws.activate_tab(0, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, pane_a);
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("foo", window, cx_state));

            // Focus B — "foo" is saved for A; B has no draft, input clears.
            ws.activate_tab(1, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, pane_b);
            assert_eq!(
                ws.terminal_input.read(cx).value().to_string(),
                "",
                "pane B shows its own (empty) draft"
            );
            assert_eq!(
                ws.input_drafts.get(&pane_a).map(String::as_str),
                Some("foo"),
                "pane A draft persisted on focus away"
            );

            // Focus A again — "foo" restored; B never gained a draft.
            ws.activate_tab(0, window, cx);
            assert_eq!(
                ws.terminal_input.read(cx).value().to_string(),
                "foo",
                "pane A draft restored on focus back"
            );
            assert_eq!(
                ws.input_drafts.get(&pane_b),
                None,
                "pane B has no stored draft"
            );

            // A File pane is not input-capable. Focusing it must not touch
            // the visible draft or the owner pointer.
            let lane_id = ws.active.lane;
            let file_pane = ws.create_file_pane(
                lane_id,
                root.join("note.txt"),
                false,
                None,
                FileViewMode::Preview,
                window,
                cx,
            );
            let file_id = file_pane.id;
            ws.active_runtime_mut().panes.push(file_pane);
            ws.set_focused_pane(file_id, window, cx);

            assert_eq!(
                ws.terminal_input.read(cx).value().to_string(),
                "foo",
                "non-input focus leaves the visible draft untouched"
            );
            assert_eq!(
                ws.input_owner,
                Some(pane_a),
                "non-input focus leaves the owner pointer at pane A"
            );

            // Edit the still-visible text, then focus a fresh input pane —
            // the edit must be saved to pane A (the owner), never the file.
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("foobar", window, cx_state));
            ws.add_tab(window, cx);
            let pane_b = ws.active_runtime().focused_pane_id;
            assert_ne!(pane_b, file_id);
            assert_eq!(
                ws.input_drafts.get(&pane_a).map(String::as_str),
                Some("foobar"),
                "edit made while a non-input pane held focus saves to owner A"
            );
            assert_eq!(
                ws.input_drafts.get(&file_id),
                None,
                "the file pane never stores a draft"
            );

            // Closing an input pane drops its stored draft and clears the owner
            // if it pointed at that pane.
            ws.activate_tab(0, window, cx);
            assert_eq!(ws.input_owner, Some(pane_a));
            assert_eq!(
                ws.input_drafts.get(&pane_a).map(String::as_str),
                Some("foobar"),
                "pane A has a stored draft before close"
            );

            ws.close_pane_by_id(pane_a, window, cx);
            assert_eq!(
                ws.input_drafts.get(&pane_a),
                None,
                "closing pane A drops its draft"
            );
            assert_ne!(
                ws.input_owner,
                Some(pane_a),
                "owner no longer points at the closed pane"
            );
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn test_restore_inaccessible_active_lane_leaves_no_tab(cx: &mut TestAppContext) {
    // When the active lane's root is missing on disk, restore must not
    // seed a stray $HOME terminal tab — the main area renders the
    // inaccessible empty-state instead, which requires zero tabs so the
    // empty-tab branch in `render` is reached.
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    // Intentionally a path that does not exist → classified Missing.
    let missing_root = std::env::temp_dir().join(format!(
        "daruda_restore_missing_{}_{}",
        std::process::id(),
        "nope"
    ));
    let _ = std::fs::remove_dir_all(&missing_root);
    let project = daruda_store::project::Project::from_path(&missing_root);
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let lane = daruda_store::project::SerializedLane {
            id: 0,
            kind: daruda_store::project::LaneKind::Default,
            path: missing_root.clone(),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: vec![daruda_store::project::SerializedTab {
                layout: daruda_store::project::SerializedLayout::Leaf {
                    pane_id: 1,
                    cwd: Some(missing_root.clone()),
                    file: None,
                    agent_chat: None,
                    account_id: None,
                },
                last_focused_pane: 1,
                user_label: None,
            }],
            active_tab_index: 0,
            base_ref: None,
            description: None,
            remote_cwd: None,
            session_host: None,
        };
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: missing_root.clone(),
            name: None,
            lanes: vec![lane],
            last_active_lane_id: 0,
            next_lane_id: 1,
            default_branch: None,
            base_branch: None,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: Some(0),
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 1,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().tabs.len(),
            0,
            "inaccessible active lane must not seed a fallback tab"
        );
        assert_eq!(
            ws.active_lane().map(|l| l.availability),
            Some(crate::lane::availability::LaneAvailability::Missing),
        );
    });
}

#[gpui::test]
fn test_restore_state_clamps_stale_active_lane_id(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_clamp_id");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let lane = daruda_store::project::SerializedLane::default_for_path(
            3,
            std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
            name: None,
            lanes: vec![lane],
            last_active_lane_id: 3,
            next_lane_id: 4,
            default_branch: None,
            base_branch: None,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: Some(999), // stale — only id 3 exists
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active.lane, 3);
    });
}

#[gpui::test]
fn activate_inaccessible_lane_persists_active_ref(cx: &mut TestAppContext) {
    // Two lanes: lane 0 present (a hermetic tempdir), lane 1 pointing at
    // a directory that does not exist. Activating lane 1 must flip
    // `self.active` (the persisted selection) and reach the early-return's
    // `mutate_durable`, without seeding a tab on the dead path.
    //
    // `root_a` must be a non-git directory so `bootstrap_from_project`
    // discovers exactly one lane (id 0). Using the live process cwd here is
    // a trap: when daruda's own checkout has git worktrees, bootstrap
    // assigns them lane ids 1.., colliding with the manually-pushed id-1
    // lane below — `lane_for(1)` would then resolve to a real, present
    // worktree and the inaccessible-lane assertion would never hold.
    let config = daruda_config::Config::default();
    let _root_a_dir = tempfile::tempdir().unwrap();
    let root_a = _root_a_dir.path().to_path_buf();
    let root_b = std::path::PathBuf::from("/tmp/daruda_activate_inaccessible_missing_dir");
    let _ = std::fs::remove_dir_all(&root_b);
    let project = daruda_store::project::Project::from_path(&root_a);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });
    cx.update_window(wh.into(), |_, window, cx| {
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
            // active-ref flipped (this is the value mutate_durable persists)
            assert_eq!(ws.active.lane, 1);
            assert_eq!(
                ws.active_lane().map(|l| l.availability),
                Some(crate::lane::availability::LaneAvailability::Missing),
            );
            // No tab seeded on the dead path.
            assert_eq!(
                ws.active_runtime().tabs.len(),
                0,
                "inaccessible lane must not seed a fallback tab"
            );

            ws.add_tab(window, cx);
            assert_eq!(
                ws.active_runtime().tabs.len(),
                0,
                "add_tab on a non-Present lane must not spawn a tab/PTY"
            );
            assert_eq!(ws.active_runtime().panes.len(), 0);

            ws.split_focused_pane_kind(
                NewPaneKind::Terminal,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            assert_eq!(
                ws.active_runtime().panes.len(),
                0,
                "split on a non-Present / paneless lane must not push an orphan PTY"
            );
            assert_eq!(ws.active_runtime().tabs.len(), 0);

            ws.on_toggle_zoom_pane(&ToggleZoomPane, window, cx);
            assert_eq!(
                ws.main_area.zoomed_pane_id, None,
                "zoom must stay None when focused_pane_id matches no real pane"
            );
        });
    })
    .unwrap();

    // The user recreates the missing directory (or grants access), then clicks
    // the same lane again. The active ref stays put, but availability must be
    // re-probed and recovered without auto-seeding a shell.
    std::fs::create_dir_all(&root_b).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            ws.activate_lane(active, window, cx);
            assert_eq!(
                ws.active_lane().map(|l| l.availability),
                Some(crate::lane::availability::LaneAvailability::Present),
                "same-lane click must re-probe and recover a recreated dir"
            );
            assert!(
                ws.active_runtime().tabs.is_empty(),
                "a recovered empty lane must land on the empty-state, not a seeded shell"
            );
            assert!(ws.active_runtime().panes.is_empty());
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_b);
}

#[gpui::test]
fn apply_discovered_lanes_rekeys_active_ref_and_preserves_non_git_noop(cx: &mut TestAppContext) {
    // Construction now seeds a pure placeholder (single Default lane,
    // id 0). Discovery assigns ids before sorting the project-root
    // lane first, so the incoming active lane may carry a non-zero id;
    // the swap must repair `self.active` and the snap target.
    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/test_apply_discovered");
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| {
            // Precondition: the construction placeholder.
            assert_eq!(ws.active_lanes().len(), 1);
            assert!(!ws.active_lanes()[0].is_git());
            assert_eq!(ws.active, daruda_store::project::LaneRef::default());

            let repo_root = std::path::PathBuf::from("/tmp/test_apply_discovered_repo");
            let discovered = vec![
                // Sorted-first (target) lane with a non-zero id.
                crate::lane::Lane::git(
                    2,
                    root.clone(),
                    Some("feat".into()),
                    repo_root.clone(),
                    root.clone(),
                    0,
                ),
                crate::lane::Lane::git(
                    0,
                    repo_root.clone(),
                    Some("main".into()),
                    repo_root.clone(),
                    repo_root.clone(),
                    1,
                ),
            ];
            ws.apply_discovered_lanes(0, discovered, cx);

            let p = ws.active_project().expect("project survives the swap");
            assert_eq!(p.lanes.len(), 2);
            assert!(p.lanes[0].is_git());
            assert_eq!(
                ws.active,
                daruda_store::project::LaneRef {
                    project: 0,
                    lane: 2
                },
                "active ref must follow the placeholder onto the sorted-first lane id"
            );
            assert_eq!(p.last_active_lane_id, 2);
        });
    })
    .unwrap();

    // A genuinely non-git root probes back the same single-Default
    // shape — the placeholder must stay untouched (no churn, no
    // active-ref movement).
    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/test_apply_discovered_nongit");
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| {
            let discovered = vec![crate::lane::Lane::default_for_project(0, root.clone())];
            ws.apply_discovered_lanes(0, discovered, cx);
            assert_eq!(ws.active_lanes().len(), 1);
            assert!(!ws.active_lanes()[0].is_git());
            assert_eq!(ws.active, daruda_store::project::LaneRef::default());
        });
    })
    .unwrap();
}

/// The single-lane convention is preserved: when the active lane's last
/// tab is also the window's last tab across every lane, closing it removes
/// the window (B1). Nothing else is open, so there is nothing to keep.
#[gpui::test]
fn closing_last_tab_of_only_lane_closes_window(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/daruda_close_last_tab_only_lane");
    let _ = std::fs::create_dir_all(&root);
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_runtime().tabs.len(), 1, "one bootstrapped tab");
    });
    assert_eq!(cx.windows().len(), 1);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.close_tab_at(0, window, cx);
        });
    })
    .unwrap();

    assert!(
        cx.windows().is_empty(),
        "closing the only lane's last tab removes the window"
    );

    let _ = std::fs::remove_dir_all(&root);
}
