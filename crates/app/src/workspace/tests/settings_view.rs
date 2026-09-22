//! Settings as a body-level view of the workspace window: it goes up, it comes
//! down, and the pane that had focus gets it back.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::workspace::OpenSettings;

#[gpui::test]
async fn open_settings_puts_the_view_on_screen(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(
                !ws.settings_is_open(),
                "a fresh workspace shows no settings"
            );
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::Font),
                window,
                cx,
            );
            let view = ws.settings_view().expect("settings should be on screen");
            assert_eq!(
                view.read(cx).active_section(),
                daruda_config::BuiltinSection::Font,
                "the action's section is the one shown",
            );
        });
    })
    .unwrap();
}

/// A second dispatch must move the open view rather than build another — the
/// first one holds the user's in-flight edits.
#[gpui::test]
async fn reopening_moves_the_same_view(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::Font),
                window,
                cx,
            );
            let first = ws.settings_view().expect("settings on screen").clone();

            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::About),
                window,
                cx,
            );
            let view = ws.settings_view().expect("settings still on screen");
            assert_eq!(
                view.entity_id(),
                first.entity_id(),
                "reopening should reuse the live view",
            );
            assert_eq!(
                view.read(cx).active_section(),
                daruda_config::BuiltinSection::About,
            );
        });
    })
    .unwrap();
}

/// The view asks to go away by emitting; nothing else may take it down, so the
/// subscription is what this covers.
#[gpui::test]
async fn the_view_s_close_event_takes_it_down(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    let view = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.on_open_settings(
                    &OpenSettings(daruda_config::BuiltinSection::General),
                    window,
                    cx,
                );
                ws.settings_view().expect("settings on screen").clone()
            })
        })
        .unwrap();

    cx.update_window(window_handle.into(), |_, _window, cx| {
        view.update(cx, |_, cx| {
            cx.emit(crate::settings::SettingsEvent::Close);
        });
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, _window, cx| {
        assert!(
            !workspace.read(cx).settings_is_open(),
            "Close should take the view down",
        );
    })
    .unwrap();
}

/// A chord that drives a dock must not land while Settings covers it. The
/// registration is skipped wholesale in that mode, so this is the check that
/// the skip actually reaches the dispatch tree rather than just the source.
///
/// This only means anything while focus is genuinely inside the view: an
/// unfocused open collapses the dispatch path to the root, under which *no*
/// workspace action fires and the assertion would pass for the wrong reason.
/// [`escape_closes_settings_from_a_fresh_open`] is what holds that up.
#[gpui::test]
async fn settings_mode_does_not_answer_a_dock_action(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);

    // Control: outside Settings the same dispatch flips the dock.
    let before = workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open);
    vcx.dispatch_action(crate::workspace::ToggleLeftDock);
    vcx.run_until_parked();
    let toggled = workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open);
    assert_ne!(before, toggled, "the dock action works outside Settings");

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    vcx.dispatch_action(crate::workspace::ToggleLeftDock);
    vcx.run_until_parked();

    assert_eq!(
        workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open),
        toggled,
        "Settings must swallow the dock chord, not pass it to a hidden dock",
    );
}

/// The gesture the design decision names, taken the way a user takes it —
/// through key dispatch, not by emitting the event. A fresh open used to leave
/// focus on the pane it replaced, gpui resolved that to the root dispatch
/// node, and Escape reached nothing.
#[gpui::test]
async fn escape_closes_settings_from_a_fresh_open(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    assert!(
        workspace.read_with(&vcx, |ws, _| !ws.settings_is_open()),
        "Escape must reach the view on the first open, with no click first",
    );
}

/// The other half of the same contract: the window-level actions stay on the
/// dispatch path. `CloseWindow` is the one whose global fallback was deleted,
/// so nothing else would answer it.
#[gpui::test]
async fn a_fresh_open_still_answers_window_level_actions(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    vcx.dispatch_action(OpenSettings(daruda_config::BuiltinSection::About));
    vcx.run_until_parked();

    assert_eq!(
        workspace.read_with(&vcx, |ws, cx| ws
            .settings_view()
            .map(|view| view.read(cx).active_section())),
        Some(daruda_config::BuiltinSection::About),
        "a window-level action must still reach the workspace behind Settings",
    );
}

/// Closing from a workspace with no panes must still leave the window focused
/// on something it renders — otherwise the keyboard dies the same way an
/// unfocused open killed it.
#[gpui::test]
async fn closing_without_a_pane_to_return_to_keeps_the_keyboard(cx: &mut TestAppContext) {
    // Deliberately not `build_workspace`: that seeds a tab, and the state under
    // test is the one a workspace lands in after its last project closes.
    let (window_handle, workspace) = build_workspace_without_tabs(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(
                ws.active_runtime().panes.is_empty(),
                "this test is about the no-pane workspace",
            );
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    vcx.update(|window, cx| {
        assert!(!workspace.read(cx).settings_is_open());
        assert!(
            workspace.read(cx).focus_handle.is_focused(window),
            "the workspace root has to take focus when no pane can",
        );
    });
}

/// [`super::build_workspace`] without its seeded tab — the shape a workspace
/// has once its last project is gone, which `render/center.rs` draws as
/// Landing.
fn build_workspace_without_tabs(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<crate::workspace::Workspace>,
) {
    crate::test_support::init_gpui_component(cx);
    let held = std::cell::RefCell::new(None);
    let window_handle = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            crate::workspace::Workspace::new_with_project_for_test(
                &daruda_config::Config::default(),
                None,
                super::fresh_test_data_dir(),
                window,
                cx,
            )
        });
        *held.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = held.borrow().clone().unwrap();
    (window_handle, workspace)
}

/// The status pulse dirties the dock entities four times a second for as long
/// as any agent is animating — which is exactly when someone opens Settings.
/// Behind Settings the docks are off screen, and gpui already filters
/// invalidation to windows that *display* an entity
/// (`App::notify` → `tracked_entities`). The one thing that can defeat that
/// filter is `Workspace::render` reading the docks anyway, which registers
/// them as displayed and turns every tick into a full workspace render whose
/// body is thrown away.
#[gpui::test]
async fn a_dock_pulse_does_not_wake_the_workspace_behind_settings(cx: &mut TestAppContext) {
    use crate::workspace::render::WORKSPACE_RENDERS;

    let (window_handle, workspace) = build_workspace(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);
    vcx.run_until_parked();

    let pulse = |vcx: &mut gpui::VisualTestContext| {
        WORKSPACE_RENDERS.with(|n| n.set(0));
        vcx.update(|_, cx| {
            workspace.update(cx, |ws, cx| {
                ws.notify_left_dock(cx);
                ws.notify_right_dock(cx);
            });
        });
        vcx.run_until_parked();
        WORKSPACE_RENDERS.with(|n| n.get())
    };

    // Control: on screen, the same pulse has to reach them.
    assert!(
        pulse(&mut vcx) > 0,
        "with the docks on screen a pulse must render the workspace",
    );

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    assert_eq!(
        pulse(&mut vcx),
        0,
        "a pulse for docks Settings covers must wake nothing",
    );
}
