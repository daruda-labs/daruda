//! The 250 ms status pulse must not repaint the left dock when the dock is
//! showing a view with no animated badge in it.
//!
//! `AgentStatusBadge` appears only in the Lanes view's rows
//! (`left_dock/projects/agent_badges.rs`), but the pulse dirtied the whole
//! dock. On the Git Changes view that rebuilt one row element per changed
//! file, four times a second, to animate nothing: measured 6.8 ms per render
//! at 200 changed files and 32 ms at 1000.

use gpui::{AppContext as _, TestAppContext};

use super::{Workspace, fresh_test_data_dir};
use daruda_store::project::LeftDockView;

fn workspace_with_project(
    cx: &mut TestAppContext,
) -> (gpui::WindowHandle<Workspace>, gpui::Entity<Workspace>) {
    crate::test_support::init_gpui_component(cx);
    let config = daruda_config::Config::default();
    let root = std::env::temp_dir().join(format!("daruda_pulse_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
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
    (wh, ws)
}

#[gpui::test]
async fn status_pulse_repaints_the_left_dock_only_where_a_badge_lives(cx: &mut TestAppContext) {
    let (wh, ws) = workspace_with_project(cx);
    cx.run_until_parked();

    cx.update_window(wh.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| {
            ws.left_dock.update(cx, |d, cx| {
                d.is_open = true;
                cx.notify();
            });
        });
    })
    .unwrap();
    cx.run_until_parked();

    // Only the Lanes view draws a badge, so only it has anything a tick could
    // advance.
    for (view, expected) in [
        (LeftDockView::Lanes, true),
        (LeftDockView::GitChanges, false),
        (LeftDockView::Files, false),
    ] {
        let paints = cx
            .update_window(wh.into(), |_, _window, cx| {
                ws.update(cx, |ws, cx| {
                    ws.set_left_dock_view(view, cx);
                    ws.left_dock_paints_status_pulse(cx)
                })
            })
            .unwrap();
        assert_eq!(
            paints,
            expected,
            "{view:?}: the pulse should{} repaint the left dock",
            if expected { "" } else { " not" }
        );
    }

    // A closed dock paints nothing at all, badge view or not.
    let closed = cx
        .update_window(wh.into(), |_, _window, cx| {
            ws.update(cx, |ws, cx| {
                ws.set_left_dock_view(LeftDockView::Lanes, cx);
                ws.left_dock.update(cx, |d, cx| {
                    d.is_open = false;
                    cx.notify();
                });
                ws.left_dock_paints_status_pulse(cx)
            })
        })
        .unwrap();
    assert!(!closed, "a closed dock has nothing to advance");

    // The other half of the contract: where the gate says yes, a notify really
    // does repaint — otherwise the gate would silently freeze the badge.
    cx.update_window(wh.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| {
            ws.left_dock.update(cx, |d, cx| {
                d.is_open = true;
                cx.notify();
            });
        });
    })
    .unwrap();
    cx.run_until_parked();

    let before = ws.read_with(cx, |ws, cx| ws.left_dock.read(cx).render_count.get());
    ws.update(cx, |ws, cx| ws.notify_left_dock(cx));
    cx.run_until_parked();
    let after = ws.read_with(cx, |ws, cx| ws.left_dock.read(cx).render_count.get());
    assert!(
        after > before,
        "notify_left_dock did not repaint the dock ({before} -> {after}); \
         the pulse gate would then be hiding a badge that never animates"
    );
}
