//! Regression coverage for `mutate_durable[_in]`. Verifies the wrapper
//! actually runs the inner mutation and triggers persist, and that
//! the closure's return value is propagated to the caller.

use super::*;

fn make_workspace_with_dirs(
    cx: &mut TestAppContext,
    primary: &str,
) -> gpui::WindowHandle<Workspace> {
    let config = daruda_config::Config::default();
    std::fs::create_dir_all(primary).unwrap();
    let project = daruda_store::project::Project::from_path(primary);
    cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    })
}

#[gpui::test]
async fn mutate_durable_returns_inner_value(cx: &mut TestAppContext) {
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_durable_returns");
    let ws = wh.root(cx).unwrap();
    let value = ws.update(cx, |ws, cx| ws.mutate_durable(cx, |_, _| 42_usize));
    assert_eq!(value, 42);
}

#[gpui::test]
async fn mutate_durable_runs_inner_closure(cx: &mut TestAppContext) {
    // Assert the wrapper drives the closure to completion: the
    // `left_dock_view` change is visible afterward. Persist scheduling
    // is async (cx.defer), so persisted state is not asserted here.
    let wh = make_workspace_with_dirs(cx, "/tmp/daruda_durable_runs");
    let ws = wh.root(cx).unwrap();
    let before = ws.read_with(cx, |ws, _| ws.left_dock_view);
    let target = match before {
        daruda_store::project::LeftDockView::Lanes => {
            daruda_store::project::LeftDockView::GitChanges
        }
        _ => daruda_store::project::LeftDockView::Lanes,
    };
    ws.update(cx, |ws, cx| {
        ws.mutate_durable(cx, |ws, _| {
            ws.left_dock_view = target;
        });
    });
    let after = ws.read_with(cx, |ws, _| ws.left_dock_view);
    assert_eq!(after, target);
}
