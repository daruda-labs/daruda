use super::*;

// ---- Lanes (W-2) ----

#[gpui::test]
fn test_workspace_without_project_has_no_lanes(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert!(ws.active_lanes().is_empty());
    });
}

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
    });
}

#[gpui::test]
fn test_activate_lane_requires_existing_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_activate");
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

#[gpui::test]
fn test_lane_removable_excludes_main_and_default(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |_ws, _| {
        let default_wt =
            crate::lane::Lane::default_for_project(0, std::path::PathBuf::from("/tmp"));
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
    });
}

/// Regression: `path != repo_root` wrongly made a main lane removable when
/// the user opened a subdirectory and `Lane.path` was anchored to that
/// subdirectory. `worktree_root == repo_root` is the correct signal.
#[gpui::test]
fn test_lane_removable_subdir_anchored_main_not_removable(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |_ws, _| {
        // path is anchored to a subdirectory; worktree_root stays at the
        // git toplevel which equals repo_root — this IS the main worktree.
        let lane = crate::lane::Lane::git(
            0,
            std::path::PathBuf::from("/repo/sub"), // subdir anchor
            Some("main".into()),
            std::path::PathBuf::from("/repo"), // repo_root
            std::path::PathBuf::from("/repo"), // worktree_root == repo_root
            0,
        );
        assert!(
            lane.is_main,
            "precondition: subdir-anchored main lane must have is_main=true"
        );
        assert!(
            !Workspace::lane_removable(&lane),
            "subdir-anchored main lane must NOT be removable"
        );
    });
}

/// Regression: `set_kind` must recompute `is_main` atomically so that a lane
/// promoted from `Default` to `Git` (via `git init`) with `worktree_root ==
/// repo_root` becomes non-removable immediately, without going through a
/// constructor.
#[gpui::test]
fn test_set_kind_recomputes_is_main_and_blocks_removal(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |_ws, _| {
        // Start from a Default lane — is_main is false.
        let mut lane =
            crate::lane::Lane::default_for_project(0, std::path::PathBuf::from("/tmp/repo"));
        assert!(!lane.is_main, "precondition: Default lane is not main");
        assert!(
            !Workspace::lane_removable(&lane),
            "precondition: Default lane is not removable either"
        );

        // Promote to Git where worktree_root == repo_root (freshly init'd repo).
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
    });
}

/// Regression: an external `git checkout` is surfaced only through a
/// `git status` refresh, which must rewrite the lane's recorded
/// `kind.branch` (the source the left-dock label reads). Drift updates
/// the branch and preserves the repo roots; a matching branch is a
/// no-op.
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
fn test_git_repo_root_returns_none_for_non_git_workspace(cx: &mut TestAppContext) {
    // Non-git Default lane → git_repo_root() is the gate the
    // `[+]` button uses to short-circuit before opening the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_modal_non_git");
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
        assert!(ws.git_repo_root().is_none());
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_unknown_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_unknown");
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
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 999,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("not found"));
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_default_kind(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_default");
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
        // Bootstrapped default lane at id 0 — not removable.
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 0,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_default(cx: &mut TestAppContext) {
    // Default-kind lane → not removable. The `×` click handler in
    // dock/lanes/list.rs uses validate_remove_lane() to
    // decide whether to even open the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_main_noop");
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
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 0,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_activate_lane_swaps_tabs(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Both lane roots must exist on disk so activation classifies them
    // as `Present` and the lazy-seed spawns a pane — the behavior under
    // test. A missing root would (correctly) skip the seed.
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

    // Swap to lane 1 → both lanes' runtimes now coexist in the single
    // `runtimes` map (active just re-points to lane 1), and activate_lane
    // lazy-spawns a new pane rooted at the target lane's path (so the
    // viewport isn't empty).
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
            // Lazy seed: exactly one tab/pane materialized for lane 1.
            assert_eq!(ws.active_runtime().tabs.len(), 1);
            assert_eq!(ws.active_runtime().panes.len(), 1);
            // Both lanes registered — no separate active/inactive store.
            assert_eq!(ws.main_area.runtimes.len(), 2);
            let proj = ws.active_ref().project;
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
            let parked = ws
                .main_area
                .runtimes
                .get(&daruda_store::project::LaneRef {
                    project: proj,
                    lane: 1,
                })
                .unwrap();
            // Lane 1 carries its lazy-spawned pane now.
            assert_eq!(parked.tabs.len(), 1);
            assert_eq!(parked.panes.len(), 1);
        });
    })
    .unwrap();
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
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
        // The accessor would `expect`-panic if the default ref had no
        // entry; a clean empty read proves the constructor seed ran.
        assert!(ws.active_runtime().tabs.is_empty());
        assert!(ws.active_runtime().panes.is_empty());
        assert!(ws.main_area.runtimes.contains_key(&ws.active));
    });
}

#[gpui::test]
fn pane_lane_index_spans_active_and_parked_lanes(cx: &mut TestAppContext) {
    // Regression: a parked lane's panes must stay visible to the
    // workspace-wide aggregators (here `pane_lane_index`, which feeds the
    // ACP/PTY status indicators) after a lane switch — the bug the
    // single-store unification fixes at its root.
    let config = daruda_config::Config::default();
    let root_a = std::path::PathBuf::from("/tmp/test_pane_index_a");
    let root_b = std::path::PathBuf::from("/tmp/test_pane_index_b");
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
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root_b.clone()));
        }
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let proj = ws.active_ref().project;
            let lane0 = daruda_store::project::LaneRef {
                project: proj,
                lane: 0,
            };
            let lane1 = daruda_store::project::LaneRef {
                project: proj,
                lane: 1,
            };
            ws.activate_lane(lane1, window, cx);
            // After the switch, lane 0 is parked and lane 1 is active.
            // Both must appear in the index — a parked lane's panes can no
            // longer be dropped from the workspace-wide scan.
            let index = ws.pane_lane_index();
            assert!(
                index.iter().any(|(_, lane)| *lane == lane0),
                "parked lane 0's pane must remain in the index"
            );
            assert!(
                index.iter().any(|(_, lane)| *lane == lane1),
                "active lane 1's pane must be in the index"
            );
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[gpui::test]
fn finalize_remove_active_lane_keeps_main_area_filled(cx: &mut TestAppContext) {
    // Removing the *active* lane must re-point to a sibling and refill
    // the viewport — never leave the main area blank.
    let config = daruda_config::Config::default();
    let repo = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo");
    let feature = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo-feature");
    let _ = std::fs::create_dir_all(&repo);
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

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1, "removable lane is active before removal");
            ws.finalize_remove_lane(lane1, window, cx);
        });
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
    });

    let _ = std::fs::remove_dir_all(&repo);
}

#[gpui::test]
fn finalize_remove_lane_releases_pane_tracking(cx: &mut TestAppContext) {
    // Dropping a lane's runtime must also release its panes from PTY
    // tracking — otherwise the tracker's poll loop walks the dead
    // shell PIDs forever (its idle guard never re-arms) and the
    // panes' claude bindings linger until the next poll.
    let config = daruda_config::Config::default();
    let repo = std::path::PathBuf::from("/tmp/daruda_release_tracking_repo");
    let feature = std::path::PathBuf::from("/tmp/daruda_release_tracking_repo-feature");
    let _ = std::fs::create_dir_all(&repo);
    // The feature lane root must exist so activation classifies it as
    // `Present` and the lazy-seed spawns the panes this test removes.
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
    let lane1 = daruda_store::project::LaneRef {
        project: proj,
        lane: 1,
    };

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate_lane(lane1, window, cx);
            // Seed tracker registrations + bindings for the lane's
            // panes the way pane spawn and the tracker pump would.
            let pane_ids: Vec<_> = ws
                .active_runtime()
                .tabs
                .iter()
                .flat_map(|t| t.layout.pane_ids())
                .collect();
            assert!(!pane_ids.is_empty(), "activated lane spawns panes");
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
    })
    .unwrap();

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

            // Switch to lane 1 — its seeded pane takes focus, so draft-a is
            // saved for pane 0 and the input is cleared (lane 1's pane has no
            // draft).
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1);
            let pane1 = ws.active_runtime().focused_pane_id;
            assert_ne!(pane1, pane0, "each lane seeds its own input pane");
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

/// Two input-capable panes in the *same* lane keep independent drafts:
/// text typed for pane A is saved on focus-away and restored when A is
/// focused again, while pane B shows only its own (empty) draft.
#[gpui::test]
fn input_draft_round_trips_between_panes(cx: &mut TestAppContext) {
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
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root);
}

/// Focusing a non-input pane (File / TaskEdit) is a no-op for the shared
/// draft: the visible text and its `input_owner` are left in place, and a
/// later edit still saves to the original owner when an input pane is
/// focused again.
#[gpui::test]
fn focusing_non_input_pane_keeps_draft(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::FileViewMode;

    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/test_input_draft_noninput");
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
            ws.add_tab(window, cx);
            let pane_a = ws.active_runtime().focused_pane_id;
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("foo", window, cx_state));
            assert_eq!(ws.input_owner, Some(pane_a));

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
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root);
}

/// Closing a pane drops its stored draft and clears `input_owner` if it
/// pointed at that pane — no lingering entry until the lane is deleted.
#[gpui::test]
fn closing_pane_drops_its_draft(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root = std::path::PathBuf::from("/tmp/test_input_draft_close");
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
            // Two single-leaf tabs so closing pane A does not close the
            // window (last-tab close removes the window): the constructor
            // opens tab 0 (pane A), a second `add_tab` opens tab 1.
            let pane_a = ws.active_runtime().focused_pane_id;
            ws.add_tab(window, cx);

            // Give pane A a stored draft: type on A, focus B (saves), back to A.
            ws.activate_tab(0, window, cx);
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("foo", window, cx_state));
            ws.activate_tab(1, window, cx);
            ws.activate_tab(0, window, cx);
            assert_eq!(ws.input_owner, Some(pane_a));
            assert_eq!(
                ws.input_drafts.get(&pane_a).map(String::as_str),
                Some("foo"),
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

/// Per-pane drafts survive a lane switch (they ride the focus chokepoint,
/// not any lane-scoped store): distinct drafts on two lanes' panes are
/// each preserved across activating the other lane and back.
#[gpui::test]
fn input_drafts_survive_lane_switch(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root_a = std::path::PathBuf::from("/tmp/test_input_draft_switch_a");
    let root_b = std::path::PathBuf::from("/tmp/test_input_draft_switch_b");
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
            ws.add_tab(window, cx);
            let pane0 = ws.active_runtime().focused_pane_id;
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("draft-a", window, cx_state));

            ws.activate_lane(lane1, window, cx);
            let pane1 = ws.active_runtime().focused_pane_id;
            assert_ne!(pane1, pane0);
            ws.terminal_input
                .update(cx, |s, cx_state| s.set_value("draft-b", window, cx_state));

            // Round-trip back to lane 0 — its draft is restored and both
            // panes' drafts are still in the map.
            ws.activate_lane(lane0, window, cx);
            assert_eq!(ws.terminal_input.read(cx).value().to_string(), "draft-a");
            assert_eq!(
                ws.input_drafts.get(&pane0).map(String::as_str),
                Some("draft-a"),
                "lane 0 pane draft survives the switch"
            );
            assert_eq!(
                ws.input_drafts.get(&pane1).map(String::as_str),
                Some("draft-b"),
                "lane 1 pane draft survives the switch"
            );

            // And forward to lane 1 again shows its own draft.
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.terminal_input.read(cx).value().to_string(), "draft-b");
        });
    })
    .unwrap();

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

// TODO Task 11: rewrite for new schema (uses deleted `save_state`).
#[cfg(any())]
#[gpui::test]
fn test_save_state_serializes_inactive_lane_runtime(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_save_inactive");
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

    // Add a second lane, push into it, swap back — now inactive
    // lane 1 has meta only (no tabs). Verify save captures both
    // without duplication.
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes.push(crate::lane::Lane::default_for_project(
                1,
                std::path::PathBuf::from("/tmp/test_wt_save_inactive_b"),
            ));
        }
    });

    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    let primary = state.primary_project().unwrap();
    assert_eq!(primary.lanes.len(), 2);
    // Active lane 0 has its tab.
    let wt0 = primary.lanes.iter().find(|w| w.id == 0).unwrap();
    assert!(!wt0.tabs.is_empty());
    // Inactive lane 1 has no runtime → serialized tabs empty.
    let wt1 = primary.lanes.iter().find(|w| w.id == 1).unwrap();
    assert!(wt1.tabs.is_empty());
}

// TODO Task 11: rewrite for new schema (uses deleted `restore_state` /
// legacy `ProjectState`).
#[cfg(any())]
#[gpui::test]
fn test_restore_state_rebuilds_all_lanes(cx: &mut TestAppContext) {
    // Create the lane directories so the path-exists check in
    // restore_state doesn't skip the inactive lane layout rebuild.
    std::fs::create_dir_all("/tmp/test_wt_restore_all/main").unwrap();
    std::fs::create_dir_all("/tmp/test_wt_restore_all/side").unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_restore_all");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let make_tab = |pane_id: u64, cwd: &str| daruda_store::project::SerializedTab {
            layout: daruda_store::project::SerializedLayout::Leaf {
                pane_id,
                cwd: Some(std::path::PathBuf::from(cwd)),
                file: None,
                agent_chat: None,
            },
            last_focused_pane: pane_id,
            user_label: None,
        };
        let state = daruda_store::project::legacy::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_wt_restore_all"),
            lanes: vec![
                daruda_store::project::SerializedLane {
                    id: 0,
                    kind: daruda_store::project::LaneKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/main"),
                    name: None,
                    tab_order: 0,
                    is_unread: false,
                    last_activity: 0,
                    tabs: vec![make_tab(100, "/tmp/test_wt_restore_all/main")],
                    active_tab_index: 0,
                    base_ref: None,
                    description: None,
                },
                daruda_store::project::SerializedLane {
                    id: 5,
                    kind: daruda_store::project::LaneKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/side"),
                    name: Some("Side".into()),
                    tab_order: 1,
                    is_unread: true,
                    last_activity: 0,
                    tabs: vec![
                        make_tab(200, "/tmp/test_wt_restore_all/side"),
                        make_tab(201, "/tmp/test_wt_restore_all/side"),
                    ],
                    active_tab_index: 1,
                    base_ref: None,
                    description: None,
                },
            ],
            active_lane_id: 5,
            active_dock_view: daruda_store::project::LeftDockView::default(),
            active_right_panel_view: daruda_store::project::RightDockView::default(),
            tabs: Vec::new(),
            active_tab_index: 0,
            focused_pane_id: 201,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        let workspace_state = daruda_store::project::legacy::WorkspaceState::from_legacy(state);
        ws.restore_state(&workspace_state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        // Active lane is 5 with 2 tabs → `active_runtime()` resolves it.
        assert_eq!(ws.active.lane, 5);
        assert_eq!(ws.active_runtime().tabs.len(), 2);
        assert_eq!(ws.active_runtime().active_tab_index, 1);
        // Both lanes rebuilt into the single store; lane 0 (parked) has 1 tab.
        assert_eq!(ws.main_area.runtimes.len(), 2);
        let proj = ws.active_ref().project;
        let parked = ws
            .main_area
            .runtimes
            .get(&daruda_store::project::LaneRef {
                project: proj,
                lane: 0,
            })
            .unwrap();
        assert_eq!(parked.tabs.len(), 1);
    });
}

// TODO Task 11: rewrite for new schema (uses deleted `save_state`).
#[cfg(any())]
#[gpui::test]
fn test_save_state_captures_bootstrapped_lane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_wt");
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
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    let primary = state.primary_project().unwrap();
    assert_eq!(primary.lanes.len(), 1);
    assert_eq!(primary.lanes[0].id, 0);
    assert_eq!(
        primary.lanes[0].path,
        std::path::PathBuf::from("/tmp/test_save_wt")
    );
    assert_eq!(state.active.lane, 0);
    // Tabs live inside the active lane from W-2 onward.
    assert!(!primary.lanes[0].tabs.is_empty());
}

#[gpui::test]
fn test_restore_state_reads_tabs_from_active_lane(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    // The lane root must exist on disk: restore classifies a missing
    // root as inaccessible and skips its pane/tab rebuild (the active
    // lane then renders the inaccessible empty-state with zero tabs).
    // This test exercises the healthy "rebuild the persisted tab" path.
    std::fs::create_dir_all("/tmp/test_restore_wt_tabs").unwrap();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_wt_tabs");
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
            path: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: vec![daruda_store::project::SerializedTab {
                layout: daruda_store::project::SerializedLayout::Leaf {
                    pane_id: 1,
                    cwd: Some(std::path::PathBuf::from("/tmp/test_restore_wt_tabs")),
                    file: None,
                    agent_chat: None,
                },
                last_focused_pane: 1,
                user_label: None,
            }],
            active_tab_index: 0,
            base_ref: None,
            description: None,
        };
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
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
        assert_eq!(ws.active_runtime().tabs.len(), 1);
        assert_eq!(ws.active_lanes().len(), 1);
        assert_eq!(ws.active.lane, 0);
    });
}

#[gpui::test]
fn test_restore_inaccessible_active_lane_leaves_no_tab(cx: &mut TestAppContext) {
    // When the active lane's root is missing on disk, restore must not
    // seed a stray $HOME terminal tab — the main area renders the
    // inaccessible empty-state instead, which requires zero tabs so the
    // empty-tab branch in `render` is reached (Task 2 guard).
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
                },
                last_focused_pane: 1,
                user_label: None,
            }],
            active_tab_index: 0,
            base_ref: None,
            description: None,
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
fn test_dock_view_defaults_to_worktrees(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Lanes
        );
    });
}

#[gpui::test]
fn test_set_dock_view_switches_and_notifies(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
        // No-op when already on that view (shouldn't panic).
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_dock_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_dock_view");
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
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.active_dock_view,
        daruda_store::project::LeftDockView::Files
    );
}

#[gpui::test]
fn test_restore_state_applies_active_dock_view(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_dock_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_dock_view"),
            name: None,
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
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
            active_lane: None,
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::GitChanges,
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
            ws.left_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_right_panel_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_right_dock_view");
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
    ws.update(cx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Tools, cx);
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.active_right_panel_view,
        daruda_store::project::RightDockView::Tools
    );
}

#[gpui::test]
fn test_restore_state_applies_active_right_panel_view(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_right_dock_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_right_dock_view"),
            name: None,
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
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
            active_lane: None,
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::Tasks,
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
            ws.right_dock_view,
            daruda_store::project::RightDockView::Tasks
        );
    });
}

#[gpui::test]
fn test_workspace_renders_with_docks_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.toggle());
        ws.bottom_dock.update(cx, |d, _| d.toggle());
        ws.right_dock.update(cx, |d, _| d.toggle());
        assert!(ws.left_dock.read(cx).is_open);
        assert!(ws.bottom_dock.read(cx).is_open);
        assert!(ws.right_dock.read(cx).is_open);
    });
}

// ---- Inaccessible active lane: action guards + self-healing ----

/// Build a workspace whose single bootstrapped lane is forced to
/// `Missing` with an empty runtime (no tabs / panes / focused pane) —
/// the inaccessible empty-state. Returns the live window handle + entity.
fn workspace_with_inaccessible_active_lane(
    cx: &mut TestAppContext,
    root: &str,
) -> (gpui::WindowHandle<Workspace>, gpui::Entity<Workspace>) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(root);
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
        let r = ws.active_ref();
        ws.set_lane_availability(r, crate::lane::availability::LaneAvailability::Missing);
        // Drop the bootstrapped runtime so the lane is paneless — mirrors
        // the restored inaccessible-lane empty-state.
        ws.active_runtime_mut().tabs.clear();
        ws.active_runtime_mut().panes.clear();
        ws.active_runtime_mut().active_tab_index = 0;
        ws.active_runtime_mut().focused_pane_id = u64::default();
        ws.main_area.zoomed_pane_id = None;
    });
    (wh, ws)
}

#[gpui::test]
fn add_tab_noop_on_inaccessible_active_lane(cx: &mut TestAppContext) {
    let (wh, ws) = workspace_with_inaccessible_active_lane(cx, "/tmp/daruda_add_tab_inaccessible");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            assert_eq!(
                ws.active_runtime().tabs.len(),
                0,
                "add_tab on a non-Present lane must not spawn a tab/PTY"
            );
            assert_eq!(ws.active_runtime().panes.len(), 0);
        });
    })
    .unwrap();
}

#[gpui::test]
fn split_noop_on_inaccessible_active_lane(cx: &mut TestAppContext) {
    let (wh, ws) = workspace_with_inaccessible_active_lane(cx, "/tmp/daruda_split_inaccessible");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
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
        });
    })
    .unwrap();
}

#[gpui::test]
fn zoom_noop_on_inaccessible_active_lane(cx: &mut TestAppContext) {
    let (wh, ws) = workspace_with_inaccessible_active_lane(cx, "/tmp/daruda_zoom_inaccessible");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.on_toggle_zoom_pane(&ToggleZoomPane, window, cx);
            assert_eq!(
                ws.main_area.zoomed_pane_id, None,
                "zoom must stay None when focused_pane_id matches no real pane"
            );
        });
    })
    .unwrap();
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
        });
    })
    .unwrap();
}

#[gpui::test]
fn same_lane_reactivate_self_heals_and_seeds_tab(cx: &mut TestAppContext) {
    // Active lane starts Missing with an empty runtime. The directory is
    // then (re)created and a same-lane activate re-probes it: availability
    // recovers to Present and a tab is seeded so the user lands in a shell.
    let root = "/tmp/daruda_same_lane_self_heal";
    let _ = std::fs::remove_dir_all(root);
    let (wh, ws) = workspace_with_inaccessible_active_lane(cx, root);
    // Sanity: starts Missing + paneless.
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_lane().map(|l| l.availability),
            Some(crate::lane::availability::LaneAvailability::Missing),
        );
        assert_eq!(ws.active_runtime().tabs.len(), 0);
    });
    // The user recreates the directory (or grants access).
    std::fs::create_dir_all(root).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            ws.activate_lane(active, window, cx);
            assert_eq!(
                ws.active_lane().map(|l| l.availability),
                Some(crate::lane::availability::LaneAvailability::Present),
                "same-lane click must re-probe and recover a recreated dir"
            );
            assert_eq!(
                ws.active_runtime().tabs.len(),
                1,
                "recovered lane with empty tabs must seed a shell tab"
            );
            assert_eq!(ws.active_runtime().panes.len(), 1);
        });
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[gpui::test]
fn apply_discovered_lanes_rekeys_active_ref_and_snap_target(cx: &mut TestAppContext) {
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
}

#[gpui::test]
fn apply_discovered_lanes_non_git_is_noop(cx: &mut TestAppContext) {
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
