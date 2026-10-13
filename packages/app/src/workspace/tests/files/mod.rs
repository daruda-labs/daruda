use super::*;
use crate::files::tree::EntryKind;
use crate::workspace::main_area::file_view_pane::DiffSource;
use std::sync::Arc;

mod lanes;
mod panel_focus;
mod tree;
mod viewer;

// ================================================================
// Files view — integration + cache invalidation regression tests
// ================================================================

/// Build a Workspace rooted at a fresh tempdir with a few files +
/// one subdirectory. The TempDir stays in scope for the lifetime
/// of the test; dropping it cleans everything up.
fn build_workspace_with_temp_project(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<Workspace>,
    gpui::Entity<Workspace>,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    std::fs::write(root.join("b.txt"), b"world").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("nested.txt"), b"deep").unwrap();

    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    (wh, ws, temp)
}

/// Find the loaded entry id under root with name `name`. Caller
/// must have already driven `ensure_file_tree` + `run_until_parked`.
fn child_id_by_name(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    name: &str,
) -> crate::files::tree::EntryId {
    ws.read_with(cx, |ws, _| {
        let id = ws.active_ref();
        let tree = ws.lane_file_tree(id).expect("file tree exists");
        for entry in tree.child_entries(tree.root_id) {
            if entry.name == name {
                return entry.id;
            }
        }
        panic!("no child entry named {name:?}");
    })
}
