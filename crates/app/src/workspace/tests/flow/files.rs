//! Flow *files* — making one, renaming it, deleting it, listing them.

use super::*;

/// The picker offers both places at once. Stated at this level and not
/// only against `flow_paths` because what the window looks in is a field
/// it holds — the two can drift, and the way that showed up first was a
/// test reading the developer's own flows.
#[gpui::test]
async fn the_picker_offers_the_person_s_own_flows_beside_the_lane_s(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    // The window's own data directory, which the test fixture already
    // points at a temp dir — the same one production resolves the global
    // flows under.
    let global = ws.update(cx, |ws, _| {
        crate::workspace::flow_paths::global_flows_dir(&ws.data_dir)
    });
    std::fs::create_dir_all(&global).expect("mkdir");
    std::fs::write(global.join("tidy.yaml"), "version: 1\n").expect("write");

    let rows = ws.update(cx, |ws, cx| {
        ws.open_flow_picker(
            crate::workspace::command::flow_picker::FlowPurpose::Validate,
            cx,
        );
        picker_rows(ws)
    });
    assert_eq!(rows, vec!["ship.yaml".to_string(), "tidy.yaml".to_string()]);
}

/// The panel lists the lane's flow files, and only while its tab is showing.
///
/// The list is a directory read behind a snapshot the render pass rebuilds
/// every frame, so the gate is not an optimisation detail — without it a tab
/// nobody is looking at costs two listings per frame. The origin travels with
/// each row because the repository's folder and the person's own can hold the
/// same name.
#[gpui::test]
async fn the_panel_lists_the_lane_s_flow_files_only_while_showing(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    std::fs::write(
        crate::workspace::flow_paths::flows_dir(lane.path()).join("notes.md"),
        "not a flow",
    )
    .expect("write");

    let hidden = ws.update(cx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Usage, cx);
        ws.flow_list_for_panel()
    });
    assert!(
        hidden.is_empty(),
        "a tab that is not showing must not read the directory: {hidden:?}"
    );

    let listed = ws.update(cx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
        ws.flow_list_for_panel()
    });
    let names: Vec<String> = listed
        .iter()
        .map(|found| {
            found
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        names,
        vec!["ship.yaml".to_string()],
        "flows only, not notes"
    );
    assert_eq!(
        listed[0].origin,
        crate::workspace::flow_paths::FlowOrigin::Repo,
        "a flow in the lane's own .daruda/flows is the repository's"
    );
}

/// A new flow lands in this project's own directory under the app home, is
/// listed from there, and loads — which is what makes the graph it opens a
/// graph rather than an error.
#[gpui::test]
async fn a_created_flow_is_listed_and_loads(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);

    // Warm the cache first, with the tab showing — the state the panel is in
    // when `[+]` is clicked. Reading it only afterwards passes even with the
    // invalidation deleted, because a hidden tab never fills the cache.
    let before = ws.update(&mut vcx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
        ws.flow_list_for_panel()
    });
    assert!(
        !before.iter().any(|f| f.path.ends_with("ship it.yaml")),
        "the warmed cache starts without it"
    );

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.create_flow("ship it", window, cx)
    });
    vcx.run_until_parked();

    let listed = ws.update(&mut vcx, |ws, _| ws.flow_list_for_panel());
    let made = listed
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "ship it.yaml"))
        .expect("the created flow is listed");
    assert_eq!(
        made.origin,
        crate::workspace::flow_paths::FlowOrigin::Project,
        "a created flow belongs to the project, not the working tree"
    );
    assert!(
        !made.path.starts_with(lane.path()),
        "nothing was written into the repository: {:?}",
        made.path
    );

    let text = std::fs::read_to_string(&made.path).expect("readable");
    daruda_flow::load(&text, None)
        .expect("the starter flow has to load, or the graph opens broken");

    // And the graph pane for it is what opening left behind.
    let drawn: Vec<std::path::PathBuf> = ws.read_with(&vcx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.flow_graph_content().map(|fg| fg.path.clone()))
            .collect()
    });
    assert_eq!(drawn, vec![made.path.clone()]);
}

/// Renaming a flow follows it in the pane drawing it.
///
/// The tab surviving with a stale path would report the old name as unreadable
/// on the next repaint — technically true and useless, since the person renamed
/// the file rather than losing it.
#[gpui::test]
async fn renaming_a_flow_is_followed_by_the_graph_of_it(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.create_flow("before", window, cx)
    });
    vcx.run_until_parked();
    let before = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.path.clone()))
        })
        .expect("the created flow is open");

    ws.update(&mut vcx, |ws, cx| ws.rename_flow(&before, "after", cx));
    vcx.run_until_parked();

    let (path, title) = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime().panes.iter().find_map(|p| {
                p.flow_graph_content()
                    .map(|fg| (fg.path.clone(), fg.cached_title.clone()))
            })
        })
        .expect("the pane is still open");
    assert_eq!(
        path.file_name().map(|n| n.to_string_lossy().into_owned()),
        Some("after.yaml".to_string())
    );
    assert_eq!(title.as_ref(), "after.yaml", "the tab says the new name");
    assert!(!before.exists(), "the old file is gone");
    assert!(path.exists(), "the new one is there");

    // The view holds the path too, and a reload reads it. Left behind, it would
    // read the file that was renamed away and report the graph as gone.
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the pane is still open");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, _| v.unreadable_for_test().is_none()),
        "and the view reads the new name, not the old one"
    );
}

/// Deleting a flow tells the pane drawing it. Nothing re-reads a flow file, so
/// a pane left alone keeps showing a graph of something that is no longer
/// there — looking fine until the next launch, and persisting the dead path.
#[gpui::test]
async fn deleting_a_flow_tells_the_graph_of_it(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.create_flow("doomed", window, cx)
    });
    vcx.run_until_parked();
    let (path, view) = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime().panes.iter().find_map(|p| {
                p.flow_graph_content()
                    .map(|fg| (fg.path.clone(), fg.view.clone()))
            })
        })
        .expect("the created flow is open");
    assert!(
        view.read_with(&vcx, |v, _| v.unreadable_for_test().is_none()),
        "it draws while the file is there"
    );

    ws.update(&mut vcx, |ws, cx| ws.delete_flow(&path, cx));
    vcx.run_until_parked();

    assert!(!path.exists(), "the file is gone");
    let named = view.read_with(&vcx, |v, _| match v.unreadable_for_test() {
        Some(crate::workspace::main_area::flow_graph_pane::FlowGraphError::Read {
            path, ..
        }) => path.clone(),
        other => panic!("expected the pane to report the file gone, got {other:?}"),
    });
    assert_eq!(named, path);
}

/// A file that comes back is drawn again — even byte-for-byte the same.
///
/// The trap: `reload` returns early when the bytes it read are the ones it
/// already has, so a pane that fell to "the file is gone" while keeping its
/// text would recognise the restored file as no change and stay on the
/// message. Restoring identical bytes is the case that proves it, and it is
/// the common one: `git checkout`, an undo, a file put back from the Trash.
#[gpui::test]
async fn a_flow_that_comes_back_is_drawn_again(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    let bytes = std::fs::read_to_string(&flow_path).expect("the flow is on disk");
    ws.update(&mut vcx, |ws, cx| ws.delete_flow(&flow_path, cx));
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, _| v.unreadable_for_test().is_some()),
        "it says the file is gone"
    );

    std::fs::write(&flow_path, &bytes).expect("put it back exactly as it was");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    assert!(
        view.read_with(&vcx, |v, _| v.unreadable_for_test().is_none()),
        "and draws it again once it is back"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        2,
        "with the flow's nodes on it"
    );
}
