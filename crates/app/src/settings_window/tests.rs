use super::*;
use daruda_config::BuiltinSection;
use gpui::{BorrowAppContext, Entity, TestAppContext, WindowHandle};

use crate::test_support::init_gpui_component;

/// Construct a Settings window wrapped in `gpui_component::Root` —
/// matches the production windowing path so `gpui_component::Input`'s
/// `TextElement::paint` can resolve `Root::read` without panicking.
fn build_window(
    cx: &mut TestAppContext,
) -> (WindowHandle<gpui_component::Root>, Entity<SettingsWindow>) {
    build_window_with_config(cx, daruda_config::Config::default())
}

fn build_window_with_config(
    cx: &mut TestAppContext,
    config: daruda_config::Config,
) -> (WindowHandle<gpui_component::Root>, Entity<SettingsWindow>) {
    init_gpui_component(cx);
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(config);
        });
    });
    let settings_for_root = std::cell::RefCell::new(None);
    let wh = cx.add_window(|window, cx| {
        let settings = cx.new(|cx| SettingsWindow::new(window, cx));
        *settings_for_root.borrow_mut() = Some(settings.clone());
        gpui_component::Root::new(settings, window, cx)
    });
    let entity = settings_for_root.borrow().clone().unwrap();
    (wh, entity)
}

#[gpui::test]
fn validate_accepts_defaults(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_ok());
    });
}

/// Test-only — write `value` into one of the settings_window's inputs
/// through the real `InputState::set_value` pipeline. Tests don't hold
/// a live `&mut Window`, so re-enter via the window handle.
fn set_input(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    field: fn(&SettingsWindow) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_string(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

#[gpui::test]
fn validate_rejects_invalid_font_size(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.font_size_input.clone(), "999");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("72"));
    });
}

#[gpui::test]
fn validate_rejects_invalid_spacing(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.vertical_spacing_input.clone(), "5.0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("2.0"));
    });
}

#[gpui::test]
fn validate_rejects_invalid_opacity(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.opacity_input.clone(), "0.0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("0.1"));
    });
}

#[gpui::test]
fn validate_rejects_invalid_clipboard(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    // 0 bytes < 4096 minimum
    set_input(&wh, &win, cx, |w| w.clipboard_streaming_input.clone(), "0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("4 096") || err.contains("4096"));
    });
}

#[gpui::test]
fn validate_rejects_invalid_grid_columns(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    // 0 columns < 1 minimum
    set_input(&wh, &win, cx, |w| w.panels_grid_columns_input.clone(), "0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("1") && err.contains("16"));
    });
}

#[gpui::test]
fn validate_accepts_grid_columns_in_range(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.panels_grid_columns_input.clone(), "8");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("8 columns must validate");
        assert_eq!(cfg.panels.grid_columns, 8);
    });
}

#[gpui::test]
fn validate_collects_agent_catalog(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_for_add = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_for_add.update(cx, |w, cx| {
            w.add_agent_row(daruda_config::AgentDefinition::codex_default(), window, cx);
        });
    })
    .unwrap();

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents,
            vec![
                daruda_config::AgentDefinition::claude_default(),
                daruda_config::AgentDefinition::codex_default(),
            ]
        );
    });
}

#[gpui::test]
fn validate_rejects_empty_agent_catalog(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| w.remove_agent_row(0, cx));
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("agent") || err.contains("에이전트"));
    });
}

#[gpui::test]
fn validate_rejects_duplicate_agent_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config {
        agents: vec![
            daruda_config::AgentDefinition::codex_default(),
            daruda_config::AgentDefinition::codex_default(),
        ],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("codex"));
    });
}

#[gpui::test]
fn validate_rejects_invalid_agent_id(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let id_input = win.read_with(cx, |w, _| w.agent_rows[0].id_input.clone());
    wh.update(cx, |_root, window, cx| {
        id_input.update(cx, |i, cx_state| {
            i.set_value("bad id".to_owned(), window, cx_state)
        });
    })
    .unwrap();
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("bad id"));
    });
}

#[gpui::test]
fn cursor_blinking_toggle(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    let initial = win.read_with(cx, |w, _| w.cursor_blinking);
    win.update(cx, |w, cx| {
        w.cursor_blinking = !w.cursor_blinking;
        cx.notify();
    });
    let after = win.read_with(cx, |w, _| w.cursor_blinking);
    assert_ne!(initial, after);
}

#[gpui::test]
fn close_on_exit_toggle(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    let initial = win.read_with(cx, |w, _| w.close_pane_on_exit);
    win.update(cx, |w, cx| {
        w.close_pane_on_exit = !w.close_pane_on_exit;
        cx.notify();
    });
    let after = win.read_with(cx, |w, _| w.close_pane_on_exit);
    assert_ne!(initial, after);
}

#[gpui::test]
fn default_section_is_general(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::General);
    });
}

#[gpui::test]
fn focus_section_updates_active(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_clone = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_clone.update(cx, |w, cx| {
            w.focus_section(BuiltinSection::Font, window, cx);
        });
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::Font);
    });
}

#[gpui::test]
fn new_with_section_lands_on_target(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let settings_for_root = std::cell::RefCell::new(None);
    let _wh = cx.add_window(|window, cx| {
        let settings =
            cx.new(|cx| SettingsWindow::new_with_section(BuiltinSection::Keymap, window, cx));
        *settings_for_root.borrow_mut() = Some(settings.clone());
        gpui_component::Root::new(settings, window, cx)
    });
    let win = settings_for_root.borrow().clone().unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::Keymap);
    });
}

#[gpui::test]
fn focus_section_resets_scroll(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_a = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_a.update(cx, |w, cx| {
            // First, set a non-zero scroll while on the General page
            // (so the test exercises the reset path inside focus_section).
            w.scroll_handle
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-100.)));
            w.focus_section(BuiltinSection::Window, window, cx);
        });
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.scroll_handle.offset().y, gpui::px(0.));
    });
}
