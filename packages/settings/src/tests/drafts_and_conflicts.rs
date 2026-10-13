use super::*;

#[gpui::test]
fn validate_accepts_defaults(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_ok());
    });
}

#[gpui::test]
fn boolean_setting_applies_immediately(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |window, cx| {
        let next = !window.window_blur;
        window.set_bool_setting(BoolSetting::WindowBlur, next, cx);
        assert!(window.error.is_none());
        assert_eq!(window.window_blur, next);
    });

    win.read_with(cx, |window, cx| {
        assert_eq!(
            crate::store::SettingsStore::global(cx).user().window.blur,
            window.window_blur
        );
    });
}

/// A text setting persists on Enter or Blur only, so a field still holding
/// focus when the view goes away has never been written. The exit path has to
/// land it.
#[gpui::test]
fn exit_commits_a_text_field_never_blurred(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );

    commit_pending_edits(&wh, &win, cx);

    win.read_with(cx, |_window, cx| {
        assert_eq!(
            crate::store::SettingsStore::global(cx)
                .user()
                .font
                .terminal
                .size,
            16.0
        );
    });
}

/// The same path must not write a value the field would have rejected — it
/// reverts instead, so the input agrees with the config it left behind.
#[gpui::test]
fn exit_reverts_a_text_field_that_cannot_parse(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "999",
    );

    commit_pending_edits(&wh, &win, cx);

    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.read_with(cx, |window, cx| {
        assert_eq!(
            crate::store::SettingsStore::global(cx)
                .user()
                .font
                .terminal
                .size,
            13.0
        );
        assert_eq!(input.read(cx).value(), "13");
        assert!(window.error.is_none());
    });
}

/// Run the exit commit and assert it did not refuse — every caller here is a
/// case that must settle, so a silent `false` would hide the interesting part.
fn commit_pending_edits(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
) {
    let settled = wh
        .update(cx, |_root, window, cx| {
            win.update(cx, |w, cx| w.commit_pending_edits(window, cx))
        })
        .expect("settings window should still be open during the test");
    assert!(settled, "nothing here should block the exit");
}

#[gpui::test]
fn valid_text_draft_applies_on_commit(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::TerminalFontSize, cx);
    });

    win.read_with(cx, |_window, cx| {
        assert_eq!(
            crate::store::SettingsStore::global(cx)
                .user()
                .font
                .terminal
                .size,
            16.0
        );
    });
}

#[gpui::test]
fn same_field_external_edit_requires_an_explicit_choice(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(20.0))
                .expect("external edit");
        });
    });
    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::TerminalFontSize, cx);
    });

    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_some());
        assert_eq!(
            crate::store::SettingsStore::global(cx)
                .user()
                .font
                .terminal
                .size,
            20.0
        );
    });

    let win_for_overwrite = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_overwrite.update(cx, |settings, cx| {
            settings.overwrite_conflict(window, cx);
        });
    })
    .unwrap();
    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_none());
        assert_eq!(
            crate::store::SettingsStore::global(cx)
                .user()
                .font
                .terminal
                .size,
            16.0
        );
    });
}

#[gpui::test]
fn conflict_can_reload_the_external_value(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(20.0))
                .expect("external edit");
        });
    });
    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::TerminalFontSize, cx);
    });
    let win_for_reload = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_reload.update(cx, |settings, cx| {
            settings.reload_conflict(window, cx);
        });
    })
    .unwrap();

    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_none());
        assert_eq!(
            window.terminal_font_size_input.read(cx).value().as_ref(),
            "20"
        );
    });
}

#[gpui::test]
fn boolean_conflict_overwrite_updates_store_and_visible_value(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let desired = win.read_with(cx, |window, _| !window.window_blur);
    win.update(cx, |window, cx| {
        window.window_blur = desired;
        cx.notify();
    });
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::WindowBlur(desired))
                .expect("external edit");
        });
    });

    win.update(cx, |window, cx| {
        assert!(!window.persist_bool_setting(BoolSetting::WindowBlur, desired, cx));
        assert!(window.conflict.is_some());
    });
    let win_for_overwrite = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_overwrite.update(cx, |settings, cx| {
            settings.overwrite_conflict(window, cx);
        });
    })
    .unwrap();

    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_none());
        assert_eq!(window.window_blur, desired);
        assert_eq!(
            crate::store::SettingsStore::global(cx).user().window.blur,
            desired
        );
    });
}

#[gpui::test]
fn clean_form_reloads_an_external_setting_immediately(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(20.0))
                .expect("external edit");
        });
    });

    win.read_with(cx, |window, cx| {
        assert_eq!(
            window.terminal_font_size_input.read(cx).value().as_ref(),
            "20"
        );
        assert_eq!(window.base_config.font.terminal.size, 20.0);
        assert!(window.conflict.is_none());
    });
}

#[gpui::test]
fn external_off_list_font_family_stays_selected(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    let family = "Daruda Off List Font For Test".to_string();
    let external_family = family.clone();
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontFamily(
                    external_family,
                ))
                .expect("external font edit");
        });
    });

    win.read_with(cx, |window, cx| {
        let selected = window
            .terminal_font_family_select
            .read(cx)
            .selected_value()
            .map(|value| value.to_string());
        assert_eq!(selected.as_deref(), Some(family.as_str()));
        assert_eq!(
            window.validate(cx).unwrap().font.terminal.family,
            family.as_str()
        );
    });
}

#[gpui::test]
fn terminal_font_options_do_not_import_agent_chat_system_ui(cx: &mut TestAppContext) {
    let mut config = daruda_config::Config::default();
    config.font.terminal.family = "Daruda Off List Terminal Font".to_string();
    config.font.agent_chat.family = daruda_config::SYSTEM_UI_FONT_FAMILY.to_string();

    cx.update(|cx| {
        let terminal_options = font_select_options(cx, &[&config.font.terminal.family]);
        assert!(
            terminal_options
                .iter()
                .any(|option| option.value.as_ref() == config.font.terminal.family.as_str())
        );
        assert!(
            terminal_options
                .iter()
                .all(|option| { option.value.as_ref() != daruda_config::SYSTEM_UI_FONT_FAMILY })
        );

        let agent_chat_options = font_select_options(cx, &[&config.font.agent_chat.family]);
        assert!(
            agent_chat_options
                .iter()
                .any(|option| { option.value.as_ref() == daruda_config::SYSTEM_UI_FONT_FAMILY })
        );
    });
}

#[gpui::test]
fn committing_one_draft_does_not_swallow_an_unrelated_external_edit(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::GeneralLanguage(
                    "en".to_string(),
                ))
                .expect("external edit");
        });
    });

    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::TerminalFontSize, cx);
    });
    cx.run_until_parked();

    win.read_with(cx, |window, cx| {
        assert_eq!(
            window
                .language_select
                .read(cx)
                .selected_value()
                .map(|value| value.as_ref()),
            Some("en")
        );
        assert_eq!(window.base_config.general.language, "en");
    });
}

#[gpui::test]
fn resolving_one_conflict_preserves_another_drafts_baseline(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    set_input(&wh, &win, cx, |window| window.opacity_input.clone(), "0.8");
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(20.0))
                .expect("external font edit");
            store
                .apply_patch(daruda_config::SettingsPatch::WindowOpacity(0.7))
                .expect("external opacity edit");
        });
    });

    let font = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&font, TextSetting::TerminalFontSize, cx);
        assert!(window.conflict.is_some());
    });
    let win_for_reload = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_reload.update(cx, |settings, cx| {
            settings.reload_conflict(window, cx);
        });
    })
    .unwrap();

    let opacity = win.read_with(cx, |window, _| window.opacity_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&opacity, TextSetting::WindowOpacity, cx);
        assert!(window.conflict.is_some());
    });
}

#[gpui::test]
fn structural_overwrite_reloads_the_persisted_catalog(cx: &mut TestAppContext) {
    let config = one_ssh_row_config();
    let (wh, win) = build_window_with_config(cx, config.clone());
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );

    let mut external_entries = config.session_hosts.clone();
    external_entries[0].label = "External label".to_string();
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::SessionHosts {
                    entries: external_entries,
                    tombstones: Vec::new(),
                })
                .expect("external host edit");
        });
    });

    win.update(cx, |window, cx| {
        window.remove_session_host_row(0, cx);
        assert!(window.conflict.is_some());
        assert_eq!(
            window.session_host_rows().count(),
            1,
            "failed edit rolls back"
        );
    });
    let win_for_overwrite = win.clone();
    wh.update(cx, |_root, window, cx| {
        win_for_overwrite.update(cx, |settings, cx| {
            settings.overwrite_conflict(window, cx);
        });
    })
    .unwrap();

    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_none());
        assert_eq!(window.session_host_rows().count(), 0);
        assert!(
            crate::store::SettingsStore::global(cx)
                .user()
                .session_hosts
                .is_empty()
        );
    });
}

#[gpui::test]
fn validate_scalar_fields_cover_boundaries(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.terminal_font_size_input.clone(), "999");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("72"));
    });
    set_input(&wh, &win, cx, |w| w.terminal_font_size_input.clone(), "13");

    set_input(
        &wh,
        &win,
        cx,
        |w| w.terminal_line_height_input.clone(),
        "5.0",
    );
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("2.0"));
    });
    set_input(
        &wh,
        &win,
        cx,
        |w| w.terminal_line_height_input.clone(),
        "1.0",
    );

    set_input(&wh, &win, cx, |w| w.opacity_input.clone(), "0.0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("0.1"));
    });
    set_input(&wh, &win, cx, |w| w.opacity_input.clone(), "1.0");

    // 0 bytes < 4096 minimum.
    set_input(&wh, &win, cx, |w| w.clipboard_streaming_input.clone(), "0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("4 096") || err.contains("4096"));
    });
    set_input(
        &wh,
        &win,
        cx,
        |w| w.clipboard_streaming_input.clone(),
        "4096",
    );

    // 0 columns < 1 minimum.
    set_input(&wh, &win, cx, |w| w.panels_grid_columns_input.clone(), "0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("1") && err.contains("16"));
    });
    set_input(&wh, &win, cx, |w| w.panels_grid_columns_input.clone(), "8");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("8 columns must validate");
        assert_eq!(cfg.panels.grid_columns, 8);
    });
}
