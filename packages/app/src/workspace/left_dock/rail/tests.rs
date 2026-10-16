use super::project_initial;
#[test]
fn initials_keep_unicode_graphemes() {
    assert_eq!(project_initial(" daruda"), "D");
    assert_eq!(project_initial("한글"), "한");
    assert_eq!(project_initial("👩‍💻 app"), "👩‍💻");
    assert_eq!(project_initial(" "), "?");
}

#[test]
fn group_labels_are_short_unicode_abbreviations() {
    assert_eq!(super::group_abbreviation("Development"), "DEV");
    assert_eq!(super::group_abbreviation("Open source"), "OS");
    assert_eq!(super::group_abbreviation("개발팀"), "개발");
    assert_eq!(super::group_abbreviation(""), "?");
}

#[gpui::test]
async fn groups_are_the_only_fold_and_empty_worktrees_remain_reachable(
    cx: &mut gpui::TestAppContext,
) {
    let root = std::env::temp_dir().join("daruda-compact-group-test");
    std::fs::create_dir_all(&root).unwrap();
    let (wh, entity) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    use gpui::AppContext as _;
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            let original = ws.active;
            let group = ws.add_group("Development".into(), None, cx);
            ws.move_project_to_group(ws.active.project, Some(group), cx);
            let project = ws.active_project_mut().unwrap();
            project.is_collapsed = true;
            project
                .lanes
                .push(crate::lane::Lane::default_for_project(77, root.clone()));
            project
                .lanes
                .push(crate::lane::Lane::default_for_project(78, root.clone()));
            ws.activate_lane(
                daruda_store::project::LaneRef {
                    project: original.project,
                    lane: 78,
                },
                window,
                cx,
            );
            ws.open_agent_chat_pane(window, cx);
            ws.activate_lane(original, window, cx);
            ws.docks.left.update(cx, |dock, cx| {
                dock.is_open = false;
                cx.notify();
            });
            cx.notify();
        });
    })
    .unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    let (active, group) = entity.read_with(vcx, |ws, _| (ws.active, ws.groups[0].id));
    let project_selector: &'static str =
        Box::leak(format!("rail-project-{}", active.project).into_boxed_str());
    let lane_selector: &'static str =
        Box::leak(format!("rail-lane-{}-{}", active.project, active.lane).into_boxed_str());
    let empty_selector: &'static str =
        Box::leak(format!("rail-lane-{}-77", active.project).into_boxed_str());
    let group_selector: &'static str = Box::leak(format!("rail-group-{group}").into_boxed_str());
    assert!(
        bounds(vcx, lane_selector).is_some(),
        "a terminal-only worktree remains visible even if the expanded project is folded"
    );
    assert!(
        bounds(vcx, empty_selector).is_none(),
        "an empty worktree takes no rail row"
    );
    vcx.update(|window, cx| {
        entity.update(cx, |ws, cx| ws.close_tab_at(0, window, cx));
    });
    vcx.run_until_parked();
    assert!(
        bounds(vcx, lane_selector).is_none(),
        "closing the last tab removes the rail row immediately"
    );
    let project = bounds(vcx, project_selector).unwrap();
    vcx.simulate_click(project.center(), Default::default());
    vcx.run_until_parked();
    assert!(
        entity.read_with(vcx, |ws, _| ws.active_project().unwrap().is_collapsed),
        "the project click leaves the expanded tree's fold state alone"
    );
    let empty_choice: &'static str =
        Box::leak(format!("dock-worktree-{}-77", active.project).into_boxed_str());
    assert!(
        bounds(vcx, empty_choice).is_some(),
        "hidden worktrees are present in the project chooser"
    );
    vcx.simulate_click(project.center(), Default::default());
    vcx.run_until_parked();
    assert!(
        entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_none()),
        "the project trigger also toggles closed"
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    assert_eq!(
        entity.read_with(vcx, |ws, _| ws.active.lane),
        77,
        "keyboard selection can enter an empty worktree"
    );
    assert!(bounds(vcx, empty_selector).is_none());
    let group_bounds = bounds(vcx, group_selector).unwrap();
    vcx.simulate_click(group_bounds.center(), Default::default());
    vcx.run_until_parked();
    assert!(
        bounds(vcx, project_selector).is_none(),
        "the group folds its projects"
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(bounds(vcx, project_selector).is_some());
}

fn bounds(
    vcx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    // WORKAROUND: GPUI cached paint omits debug_bounds replay; refresh only
    // this test instead of changing vendored instrumentation for a feature test.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.debug_bounds(selector)
}
