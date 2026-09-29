//! Writing a file pane back to disk: the disk check before a save, and the
//! close prompts' Save routing a file pane to a real write.

use super::*;
use crate::surface::strings;
use crate::workspace::main_area::file_view_pane::PaneFileContent;

fn open_temp_file(
    cx: &mut TestAppContext,
    body: &[u8],
) -> (
    gpui::WindowHandle<crate::ui::Root>,
    gpui::Entity<Workspace>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.txt"), body).unwrap();
    init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let workspace_for_root = std::cell::RefCell::new(None);
    let wh = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::new_with_project_for_test(
                &config,
                Some(project),
                fresh_test_data_dir(),
                window,
                cx,
            )
        });
        *workspace_for_root.borrow_mut() = Some(workspace.clone());
        crate::ui::Root::new(workspace, window, cx)
    });
    let ws = workspace_for_root.into_inner().unwrap();
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let id = ws.read_with(cx, |ws, _| ws.active_ref());
    ws.update(cx, |ws, cx| ws.ensure_file_tree(id, cx));
    cx.run_until_parked();
    // `b.txt` keeps a second tab open, so closing `a.txt`'s tab closes the
    // tab rather than the window. The Files view opens by absolute path; a
    // relative one is never saved.
    std::fs::write(temp.path().join("b.txt"), b"other").unwrap();
    for name in ["b.txt", "a.txt"] {
        let path = temp.path().join(name);
        cx.update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                ws.open_files_entry(
                    id,
                    path,
                    crate::workspace::main_area::tab_ops::OpenIntent::Enter,
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.run_until_parked();
    (wh, ws, temp)
}

fn type_text(
    wh: gpui::WindowHandle<crate::ui::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    text: &str,
) {
    let text = text.to_string();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let state = ws
                .focused_file_content()
                .map(|fc| fc.editor_state.clone())
                .expect("focused pane is a file viewer");
            state.update(cx, |s, cx| s.set_value(text.as_str(), window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn save(
    wh: gpui::WindowHandle<crate::ui::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.save_focused_file_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
}

fn disk(temp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(temp.path().join("a.txt")).unwrap()
}

fn editor_text(ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext) -> String {
    ws.read_with(cx, |ws, cx| {
        ws.focused_file_content()
            .expect("file pane")
            .editor_state
            .read(cx)
            .text()
            .to_string()
    })
}

#[gpui::test]
async fn a_save_with_the_disk_unchanged_writes_without_asking(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    save(wh, &ws, cx);
    assert!(!cx.has_pending_prompt(), "nothing to ask about");
    assert_eq!(disk(&temp), "mine");
}

#[gpui::test]
async fn a_save_over_a_file_changed_on_disk_asks_before_writing(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    std::fs::write(temp.path().join("a.txt"), b"agent").unwrap();
    save(wh, &ws, cx);
    assert!(cx.has_pending_prompt(), "the conflict is put to the user");
    assert_eq!(disk(&temp), "agent", "nothing is written before the answer");

    cx.simulate_prompt_answer(&strings::file_save_conflict_overwrite());
    cx.run_until_parked();
    assert_eq!(disk(&temp), "mine", "Overwrite writes the buffer");
    let dirty = ws.read_with(cx, |ws, cx| {
        ws.active_runtime().panes.iter().any(|p| p.is_dirty(cx))
    });
    assert!(!dirty, "the written text is the new baseline");
}

#[gpui::test]
async fn reloading_on_a_save_conflict_takes_the_disk_copy(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    std::fs::write(temp.path().join("a.txt"), b"agent").unwrap();
    save(wh, &ws, cx);
    cx.simulate_prompt_answer(&strings::file_save_conflict_reload());
    cx.run_until_parked();
    assert_eq!(disk(&temp), "agent", "Reload never writes");
    assert_eq!(
        editor_text(&ws, cx),
        "agent",
        "the buffer shows the disk copy"
    );
}

#[gpui::test]
async fn a_file_deleted_on_disk_counts_as_changed(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    std::fs::remove_file(temp.path().join("a.txt")).unwrap();
    save(wh, &ws, cx);
    assert!(
        cx.has_pending_prompt(),
        "recreating a deleted file is asked first"
    );
    assert!(!temp.path().join("a.txt").exists());
}

fn close_active_tab(
    wh: gpui::WindowHandle<crate::ui::Root>,
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) {
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let index = ws.active_runtime().active_tab_index;
            ws.request_close_tab(index, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn has_a_txt_pane(ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext) -> bool {
    ws.read_with(cx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.file_view())
            .any(|fv| fv.path.ends_with("a.txt"))
    })
}

#[gpui::test]
async fn save_all_on_tab_close_writes_a_dirty_file_pane(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    close_active_tab(wh, &ws, cx);
    cx.simulate_prompt_answer(&strings::tab_close_batch_save_all());
    cx.run_until_parked();
    assert_eq!(disk(&temp), "mine", "Save all must write the file");
    assert!(!has_a_txt_pane(&ws, cx), "and then close the tab");
}

#[gpui::test]
async fn save_all_keeps_the_tab_when_the_file_changed_on_disk(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    std::fs::write(temp.path().join("a.txt"), b"agent").unwrap();
    close_active_tab(wh, &ws, cx);
    cx.simulate_prompt_answer(&strings::tab_close_batch_save_all());
    cx.run_until_parked();
    assert_eq!(disk(&temp), "agent", "the agent's copy is not overwritten");
    assert!(has_a_txt_pane(&ws, cx), "the unsaved buffer stays open");
    assert_eq!(editor_text(&ws, cx), "mine");
}

#[gpui::test]
async fn a_file_cut_at_the_size_cap_opens_read_only(cx: &mut TestAppContext) {
    let body = vec![b'x'; crate::ui::theme::FILE_VIEWER_MAX_BYTES + 10];
    let (_wh, ws, _temp) = open_temp_file(cx, &body);
    ws.read_with(cx, |ws, _| {
        let fc = ws.focused_file_content().expect("file pane");
        assert!(
            matches!(
                fc.view.content,
                PaneFileContent::LoadedRaw { truncated: true }
            ),
            "the cut is recorded on the content"
        );
        assert!(
            !fc.view.holds_editable_buffer(),
            "saving a cut buffer would truncate the file"
        );
    });
}

#[gpui::test]
async fn closing_a_dirty_file_pane_with_save_writes_it(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_focused_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_prompt_answer(&strings::task_edit_save());
    cx.run_until_parked();
    assert_eq!(disk(&temp), "mine", "Save writes the file");
    assert!(!has_a_txt_pane(&ws, cx), "then closes the pane");
}

#[gpui::test]
async fn closing_the_window_saves_a_dirty_pane_in_a_parked_lane(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    let other = tempfile::tempdir().unwrap();
    let (home, parked_to) = ws.update(cx, |ws, _| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, other.path().to_path_buf());
        lane.tab_order = 1;
        ws.project_for_mut(project).unwrap().lanes.push(lane);
        (
            ws.active,
            daruda_store::project::LaneRef {
                project,
                lane: lane_id,
            },
        )
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(parked_to, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let dirty = ws.read_with(cx, |ws, cx| {
        assert_ne!(ws.active, home, "the file's lane is parked");
        ws.collect_dirty_pane_descriptors(cx)
    });
    assert_eq!(
        dirty.len(),
        1,
        "the parked lane's edit is in the close prompt"
    );
    let saved = ws.update(cx, |ws, cx| {
        ws.commit_dirty_panes_with_failure_toast(&dirty, cx)
    });
    assert!(saved);
    assert_eq!(disk(&temp), "mine", "Save all reaches the parked lane");
}

#[gpui::test]
async fn escape_on_a_dirty_file_pane_asks_before_closing(cx: &mut TestAppContext) {
    let (wh, ws, temp) = open_temp_file(cx, b"hello");
    type_text(wh, &ws, cx, "mine");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_focused_file_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "Escape asks like the close button");
    assert!(has_a_txt_pane(&ws, cx), "the buffer is still open");
    assert_eq!(disk(&temp), "hello");
}

#[gpui::test]
async fn closing_the_window_saves_a_task_draft_in_a_parked_lane(cx: &mut TestAppContext) {
    let (wh, ws, _temp) = open_temp_file(cx, b"hello");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_task_edit_pane(None, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let title = ws
                .active_runtime()
                .panes
                .iter()
                .find_map(|p| p.task_edit_content())
                .map(|te| te.title_input.clone())
                .expect("task edit pane");
            title.update(cx, |s, cx| s.set_value("parked task", window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    let other = tempfile::tempdir().unwrap();
    let target = ws.update(cx, |ws, _| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, other.path().to_path_buf());
        lane.tab_order = 1;
        ws.project_for_mut(project).unwrap().lanes.push(lane);
        daruda_store::project::LaneRef {
            project,
            lane: lane_id,
        }
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(target, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let dirty = ws.read_with(cx, |ws, cx| ws.collect_dirty_pane_descriptors(cx));
    assert_eq!(dirty.len(), 1, "the parked draft is in the close prompt");
    let saved = ws.update(cx, |ws, cx| {
        ws.commit_dirty_panes_with_failure_toast(&dirty, cx)
    });
    assert!(
        saved,
        "Save all reaches a task draft outside the active lane"
    );
}
