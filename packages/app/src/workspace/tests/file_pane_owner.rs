//! A file pane lives in the runtime of the lane whose file it shows.
//!
//! Opening another lane's file moves to that lane when the user asked for
//! it; the pane must never sit in one lane's runtime while its content is
//! loaded against another, or the load finds no pane and it stays
//! `Loading`.

use super::*;
use crate::workspace::main_area::file_view_pane::PaneFileContent;
use crate::workspace::main_area::tab_ops::OpenIntent;

/// A workspace on project root `home` plus a second, parked lane rooted at
/// `parked`. Returns the window, the workspace and the parked lane's ref.
fn workspace_with_parked_lane(
    cx: &mut TestAppContext,
    home: &std::path::Path,
    parked: &std::path::Path,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
    daruda_store::project::LaneRef,
) {
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(home)),
    );
    let parked_ref = ws.update(cx, |ws, _| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, parked.to_path_buf());
        lane.tab_order = 1;
        ws.project_for_mut(project).unwrap().lanes.push(lane);
        daruda_store::project::LaneRef {
            project,
            lane: lane_id,
        }
    });
    (wh, ws, parked_ref)
}

fn is_loaded(ws: &Workspace, lane: daruda_store::project::LaneRef, path: &std::path::Path) -> bool {
    ws.main_area.runtimes.get(&lane).is_some_and(|runtime| {
        runtime.panes.iter().any(|p| {
            p.file_content().is_some_and(|fc| {
                fc.view.path == path && !matches!(fc.view.content, PaneFileContent::Loading)
            })
        })
    })
}

#[gpui::test]
fn opening_a_parked_lanes_file_moves_there_and_loads(cx: &mut TestAppContext) {
    let home = tempfile::tempdir().unwrap();
    let parked = tempfile::tempdir().unwrap();
    let note = parked.path().join("note.txt");
    std::fs::write(&note, "parked lane's file").unwrap();
    let (wh, ws, parked_ref) = workspace_with_parked_lane(cx, home.path(), parked.path());

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(parked_ref, note.clone(), OpenIntent::Enter, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active, parked_ref,
            "a user open moves to the file's lane"
        );
        assert!(
            is_loaded(ws, parked_ref, &note),
            "the pane sits in its own lane's runtime and its content arrives",
        );
    });
}

/// An agent may link any path. One a lane owns opens in that lane; one no
/// lane holds opens for reference where the user is, read-only.
#[gpui::test]
fn a_linked_file_opens_in_its_lane_or_for_reference(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::FileOrigin;

    let home = tempfile::tempdir().unwrap();
    let parked = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let owned = parked.path().join("owned.txt");
    let stray = elsewhere.path().join("stray.txt");
    std::fs::write(&owned, "a lane's file").unwrap();
    std::fs::write(&stray, "outside every lane").unwrap();
    let (wh, ws, parked_ref) = workspace_with_parked_lane(cx, home.path(), parked.path());
    let home_ref = ws.read_with(cx, |ws, _| ws.active);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_linked_file(stray.clone(), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, home_ref, "a stray file opens where the user is");
        let origin = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.file_content().filter(|fc| fc.view.path == stray))
            .map(|fc| fc.view.origin);
        assert_eq!(origin, Some(FileOrigin::Reference));
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_linked_file(owned.clone(), window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, parked_ref, "an owned file opens in its lane");
        assert!(is_loaded(ws, parked_ref, &owned));
    });
}

#[gpui::test]
fn a_reference_file_opens_read_only_in_the_lane_on_screen(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::FileOrigin;

    let home = tempfile::tempdir().unwrap();
    let skills = tempfile::tempdir().unwrap();
    let skill = skills.path().join("SKILL.md");
    std::fs::write(&skill, "# a skill").unwrap();
    let (wh, ws, _) = workspace_with_parked_lane(cx, home.path(), &home.path().join("parked"));
    let active = ws.read_with(cx, |ws, _| ws.active);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_reference_file(skill.clone(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, active, "a reference open moves nowhere");
        let fc = ws
            .active_runtime()
            .panes
            .iter()
            .find_map(|p| p.file_content().filter(|fc| fc.view.path == skill))
            .expect("the file opened in the lane on screen");
        assert_eq!(fc.view.origin, FileOrigin::Reference);
        assert!(!matches!(fc.view.content, PaneFileContent::Loading));
        assert!(
            !fc.view.holds_editable_buffer(),
            "a reference file is read-only"
        );
    });
}

/// No lane's view writes past its root. A pane only reaches a foreign path
/// through a bug today, so this pins the last line of defence directly.
#[gpui::test]
fn a_lane_pane_never_saves_outside_its_lane(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_save_ops::FileSaveOutcome;
    use crate::workspace::main_area::file_view_pane::{DiffSource, FileOrigin, FileViewMode};

    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let foreign = elsewhere.path().join("foreign.txt");
    std::fs::write(&foreign, "untouched").unwrap();
    let (wh, ws, _) = workspace_with_parked_lane(cx, home.path(), &home.path().join("parked"));

    let pane_id = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let pane = ws.create_file_pane(
                    FileOrigin::Lane,
                    foreign.clone(),
                    DiffSource::WorkingTree,
                    None,
                    FileViewMode::Raw,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.load_pending_file_panes(cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    let outcome = ws.update(cx, |ws, cx| ws.write_file_pane(pane_id, true, cx));
    assert!(matches!(outcome, FileSaveOutcome::NotSavable));
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "untouched");
}

/// A lane whose path is gone owns nothing and is not moved to.
#[gpui::test]
fn a_lane_whose_path_is_gone_opens_nothing(cx: &mut TestAppContext) {
    let home = tempfile::tempdir().unwrap();
    let parked = tempfile::tempdir().unwrap();
    let note = parked.path().join("note.txt");
    std::fs::write(&note, "a file").unwrap();
    let (wh, ws, parked_ref) = workspace_with_parked_lane(cx, home.path(), parked.path());
    let active = ws.update(cx, |ws, _| {
        ws.lane_for_mut(parked_ref).unwrap().availability =
            crate::lane::availability::LaneAvailability::Missing;
        ws.active
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_linked_file(note.clone(), window, cx));
        ws.update(cx, |ws, cx| {
            ws.open_files_entry(parked_ref, note.clone(), OpenIntent::Enter, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active, active, "nothing moves to a gone lane");
        assert!(
            !ws.main_area
                .runtimes
                .get(&parked_ref)
                .is_some_and(|rt| rt.panes.iter().any(|p| p.file_content().is_some())),
            "no pane lands in a gone lane",
        );
    });
}

/// A session written before panes carried their origin may hold a lane
/// pane whose file is outside the lane. It restores read-only, so a pane
/// never offers an edit its save would refuse.
#[gpui::test]
fn a_restored_pane_outside_its_lane_restores_read_only(cx: &mut TestAppContext) {
    use crate::workspace::main_area::file_view_pane::{DiffSource, FileOrigin, FileViewMode};

    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let foreign = elsewhere.path().join("foreign.txt");
    std::fs::write(&foreign, "elsewhere").unwrap();
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(home.path())),
    );
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let pane = ws.create_file_pane(
                FileOrigin::Lane,
                foreign.clone(),
                DiffSource::WorkingTree,
                None,
                FileViewMode::Raw,
                window,
                cx,
            );
            let pane_id = pane.id;
            let tab_id = ws.alloc_id();
            let runtime = ws.active_runtime_mut();
            runtime.panes.push(pane);
            runtime
                .tabs
                .push(crate::workspace::main_area::pane::TabEntry {
                    id: tab_id,
                    layout: crate::workspace::main_area::pane_tree::PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
        });
    })
    .unwrap();
    let (workspace_state, project_states) =
        ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));

    let restored_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx);
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();
    restored.read_with(cx, |ws, _| {
        let origin = ws
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .find_map(|p| p.file_content().filter(|fc| fc.view.path == foreign))
            .map(|fc| fc.view.origin);
        assert_eq!(origin, Some(FileOrigin::Reference));
    });
}
