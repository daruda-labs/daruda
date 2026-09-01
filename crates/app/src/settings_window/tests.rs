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

#[gpui::test]
fn boolean_setting_applies_immediately(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |window, cx| {
        let next = !window.cursor_blinking;
        assert!(window.persist_bool_setting(BoolSetting::CursorBlinking, next, cx));
        window.cursor_blinking = next;
    });

    win.read_with(cx, |window, cx| {
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .cursor
                .blinking,
            window.cursor_blinking
        );
    });
}

#[gpui::test]
fn valid_text_draft_applies_on_commit(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");
    let input = win.read_with(cx, |window, _| window.font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::FontSize, cx);
    });

    win.read_with(cx, |_window, cx| {
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .font
                .size,
            16.0
        );
    });
}

#[gpui::test]
fn same_field_external_edit_requires_an_explicit_choice(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::FontSize(20.0))
                .expect("external edit");
        });
    });
    let input = win.read_with(cx, |window, _| window.font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::FontSize, cx);
    });

    win.read_with(cx, |window, cx| {
        assert!(window.conflict.is_some());
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .font
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
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .font
                .size,
            16.0
        );
    });
}

#[gpui::test]
fn conflict_can_reload_the_external_value(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::FontSize(20.0))
                .expect("external edit");
        });
    });
    let input = win.read_with(cx, |window, _| window.font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::FontSize, cx);
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
        assert_eq!(window.font_size_input.read(cx).value().as_ref(), "20");
    });
}

#[gpui::test]
fn boolean_conflict_overwrite_updates_store_and_visible_value(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let desired = win.read_with(cx, |window, _| !window.cursor_blinking);
    win.update(cx, |window, cx| {
        window.cursor_blinking = desired;
        cx.notify();
    });
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::CursorBlinking(desired))
                .expect("external edit");
        });
    });

    win.update(cx, |window, cx| {
        assert!(!window.persist_bool_setting(BoolSetting::CursorBlinking, desired, cx));
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
        assert_eq!(window.cursor_blinking, desired);
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .cursor
                .blinking,
            desired
        );
    });
}

#[gpui::test]
fn clean_form_reloads_an_external_setting_immediately(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::FontSize(20.0))
                .expect("external edit");
        });
    });

    win.read_with(cx, |window, cx| {
        assert_eq!(window.font_size_input.read(cx).value().as_ref(), "20");
        assert_eq!(window.base_config.font.size, 20.0);
        assert!(window.conflict.is_none());
    });
}

#[gpui::test]
fn committing_one_draft_does_not_swallow_an_unrelated_external_edit(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::GeneralLanguage(
                    "en".to_string(),
                ))
                .expect("external edit");
        });
    });

    let input = win.read_with(cx, |window, _| window.font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::FontSize, cx);
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
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");
    set_input(&wh, &win, cx, |window| window.opacity_input.clone(), "0.8");
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::FontSize(20.0))
                .expect("external font edit");
            store
                .apply_patch(daruda_config::SettingsPatch::WindowOpacity(0.7))
                .expect("external opacity edit");
        });
    });

    let font = win.read_with(cx, |window, _| window.font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&font, TextSetting::FontSize, cx);
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
    set_input(&wh, &win, cx, |window| window.font_size_input.clone(), "16");

    let mut external_entries = config.session_hosts.clone();
    external_entries[0].label = "External label".to_string();
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
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
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .session_hosts
                .is_empty()
        );
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
fn validate_scalar_fields_cover_boundaries(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_input(&wh, &win, cx, |w| w.font_size_input.clone(), "999");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("72"));
    });
    set_input(&wh, &win, cx, |w| w.font_size_input.clone(), "13");

    set_input(&wh, &win, cx, |w| w.vertical_spacing_input.clone(), "5.0");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("2.0"));
    });
    set_input(&wh, &win, cx, |w| w.vertical_spacing_input.clone(), "1.0");

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

#[gpui::test]
fn validate_collects_agent_catalog(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_for_add = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_for_add.update(cx, |w, cx| {
            // What the "Add Preset" button does.
            w.add_agent_row(
                daruda_config::AgentDefinition::codex_default(),
                Some("codex-acp".to_string()),
                window,
                cx,
            );
        });
    })
    .unwrap();

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents,
            vec![
                // The built-in default keeps its own stable id — never promoted
                // to the `claude-acp` preset it shares a command with.
                daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition::claude_default()),
                // The added preset is stored as a reference, not a copy.
                daruda_config::AgentEntry::Preset {
                    preset: "codex-acp".to_string(),
                    overrides: daruda_config::PresetOverrides::default(),
                },
            ]
        );
        assert_eq!(
            cfg.resolved_agents(),
            vec![
                daruda_config::AgentDefinition::claude_default(),
                daruda_config::AgentDefinition::codex_default(),
            ]
        );
    });
}

/// Test-only — pick `preset_id` in the catalog's preset dropdown, exactly as
/// clicking the dropdown does. Returns whether the dropdown offered that id at
/// all (`set_selected_value` clears the selection for an unknown value).
fn select_agent_preset(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    preset_id: &str,
) -> bool {
    let state = win.read_with(cx, |w, _| w.agent_preset_select.clone());
    let value = SharedString::from(preset_id.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
    win.read_with(cx, |w, cx| {
        w.agent_preset_select.read(cx).selected_value() == Some(&value)
    })
}

/// Test-only — click "Add Preset" for whatever the dropdown currently holds.
fn add_selected_agent_preset(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.add_selected_preset_row_for_test(window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — pick `kind` in one catalog row's transport dropdown.
fn select_agent_transport(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    kind: &str,
) {
    let state = win.read_with(cx, |w, _| {
        w.agent_editable_row(index)
            .unwrap()
            .transport_select
            .clone()
    });
    let value = SharedString::from(kind.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — pick `value` in one catalog row's mode/model picker, including
/// the confirm event a real click emits (`set_selected_value` alone is silent).
fn confirm_agent_row_select(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<SelectState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    let value = SharedString::from(value.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| {
            s.set_selected_value(&value, window, cx);
            cx.emit(crate::ui::select::SelectEvent::Confirm(Some(value.clone())));
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — whether one catalog row's picker offers `value` at all.
/// `set_selected_value` clears the selection for a value the option list does
/// not carry, which is the only public read of what a `SelectState` holds.
fn agent_row_select_offers(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<SelectState>,
    value: &str,
) -> bool {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    let value = SharedString::from(value.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
    win.read_with(cx, |_w, cx| state.read(cx).selected_value() == Some(&value))
}

/// Test-only — install a known vocabulary cache into the open Settings window.
fn set_agent_vocabulary(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    vocabulary: daruda_store::agent_vocabulary::AgentVocabularyCache,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.set_agent_vocabulary_for_test(vocabulary, window, cx)
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — write `value` into one catalog row's input.
fn set_agent_row_input(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_owned(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

/// The dropdown offers the whole preset table, not just the launchable subset —
/// hiding the rest left the user with no sign those agents exist.
#[gpui::test]
fn the_preset_dropdown_offers_every_built_in_preset(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let mut needs_install = 0;
    for preset in daruda_config::agent_presets() {
        assert!(
            select_agent_preset(&wh, &win, cx, preset.id),
            "the preset dropdown is missing {}",
            preset.id
        );
        if matches!(
            preset.launchability,
            daruda_config::PresetLaunchability::NeedsManualInstall { .. }
        ) {
            needs_install += 1;
        }
    }
    assert!(
        needs_install > 0,
        "the table has manual-install presets, so the dropdown must have been exercised with some"
    );
}

/// The real "Add Preset" path: pick an id, click Add, and the row is saved as a
/// reference to that preset rather than a frozen copy of its fields.
#[gpui::test]
fn adding_a_preset_from_the_dropdown_collects_a_reference(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(select_agent_preset(&wh, &win, cx, "gemini"));
    add_selected_agent_preset(&wh, &win, cx);
    win.read_with(cx, |w, cx| {
        assert_eq!(w.agent_editable_rows().count(), 2);
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::Preset {
                preset: "gemini".to_string(),
                overrides: daruda_config::PresetOverrides::default(),
            }
        );
        // Nothing was edited, so the row reports no override to diff.
        assert!(
            !w.agent_editable_row(1)
                .unwrap()
                .provenance(cx)
                .is_overridden()
        );
    });
}

/// Editing one field of a preset row overrides that field only — every other
/// field keeps following the preset, and the row shows the preset's own value
/// next to the one that changed.
#[gpui::test]
fn editing_one_field_of_a_preset_row_overrides_only_that_field(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(select_agent_preset(&wh, &win, cx, "gemini"));
    add_selected_agent_preset(&wh, &win, cx);
    set_agent_row_input(&wh, &win, cx, 1, |r| r.name_input.clone(), "My Gemini");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::Preset {
                preset: "gemini".to_string(),
                overrides: daruda_config::PresetOverrides {
                    name: Some("My Gemini".to_string()),
                    command: None,
                    default_mode: None,
                    default_model: None,
                    fold_mode: None,
                    tail_window: None,
                    display_filter: None,
                },
            }
        );
        let provenance = w.agent_editable_row(1).unwrap().provenance(cx);
        assert!(provenance.is_overridden());
        let base = daruda_config::AgentDefinition::registry_preset("gemini").expect("runnable");
        let name_diff = provenance
            .name_base
            .expect("the renamed field shows its base");
        assert!(name_diff.contains(&base.name), "{name_diff}");
        assert_eq!(provenance.command_base, None, "command still follows");
        assert_eq!(provenance.default_mode_base, None, "mode still follows");
    });
}

/// A remote transport is not expressible as a preset override, so saving detaches
/// the row into a self-contained custom entry.
#[gpui::test]
fn switching_a_preset_row_to_ssh_detaches_it_into_a_custom_entry(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(select_agent_preset(&wh, &win, cx, "gemini"));
    add_selected_agent_preset(&wh, &win, cx);
    select_agent_transport(&wh, &win, cx, 1, "ssh");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.host_input.clone(), "vm-work");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        let base = daruda_config::AgentDefinition::registry_preset("gemini").expect("runnable");
        let daruda_config::AgentLaunch::Raw(command) = base.launch else {
            panic!("presets launch Raw");
        };
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition {
                id: base.id,
                name: base.name,
                launch: daruda_config::AgentLaunch::Ssh {
                    adapter_command: command,
                    host: "vm-work".to_string(),
                },
                default_mode: None,
                default_model: None,
                fold_mode: None,
                tail_window: None,
                display_filter: None,
            })
        );
    });
}

/// The agent-side `Ssh`/`Docker` transport is deprecated (Session Host on
/// the Lane is the new axis) but stays fully functional for one more
/// release — an already-persisted `Ssh` row must keep loading and saving
/// byte-for-byte, untouched.
#[gpui::test]
fn an_existing_ssh_row_round_trips_unchanged_through_save(cx: &mut TestAppContext) {
    let ssh_entry = daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition {
        id: "remote-claude".to_string(),
        name: "Remote Claude".to_string(),
        launch: daruda_config::AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            host: "vm-work".to_string(),
        },
        default_mode: None,
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
    });
    let config = daruda_config::Config {
        agents: vec![ssh_entry.clone()],
        ..Default::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("an untouched ssh row must validate");
        assert_eq!(cfg.agents[0], ssh_entry);
    });
}

/// Same as the `Ssh` case, for `Docker`.
#[gpui::test]
fn an_existing_docker_row_round_trips_unchanged_through_save(cx: &mut TestAppContext) {
    let docker_entry = daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition {
        id: "remote-codex".to_string(),
        name: "Remote Codex".to_string(),
        launch: daruda_config::AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".to_string(),
            container: "dev-1".to_string(),
        },
        default_mode: None,
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
    });
    let config = daruda_config::Config {
        agents: vec![docker_entry.clone()],
        ..Default::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("an untouched docker row must validate");
        assert_eq!(cfg.agents[0], docker_entry);
    });
}

/// The Settings form renders no editor for `fold_mode` / `tail_window` /
/// `display_filter`, so the save path has to carry a row's values across
/// untouched — an unrelated edit must not erase a hand-written key.
#[gpui::test]
fn transcript_defaults_survive_a_catalog_save_that_does_not_edit_them(cx: &mut TestAppContext) {
    let fold_mode = vec!["summary".to_string(), "last.thinking=expanded".to_string()];
    let display_filter = vec!["prose".to_string(), "tool_read".to_string()];
    let tuned = daruda_config::AgentDefinition {
        id: "hand-tuned-claude".to_string(),
        name: "Hand Tuned".to_string(),
        launch: daruda_config::AgentLaunch::Raw(
            "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
        ),
        default_mode: None,
        default_model: None,
        fold_mode: Some(fold_mode.clone()),
        tail_window: Some(12),
        display_filter: Some(display_filter.clone()),
    };
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::Custom(tuned)],
        ..Default::default()
    };
    let (wh, win) = build_window_with_config(cx, config);
    // Edit a field the form does own: the save rebuilds the whole definition
    // from the row, so this is the path that would drop the three keys.
    let name_input = win.read_with(cx, |w, _| {
        w.agent_editable_row(0).unwrap().name_input.clone()
    });
    wh.update(cx, |_root, window, cx| {
        name_input.update(cx, |i, cx_state| {
            i.set_value("Renamed".to_owned(), window, cx_state)
        });
    })
    .unwrap();
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a renamed row must validate");
        let saved = cfg.agents[0].resolve().expect("a custom entry resolves");
        assert_eq!(saved.name, "Renamed");
        assert_eq!(saved.fold_mode, Some(fold_mode));
        assert_eq!(saved.tail_window, Some(12));
        assert_eq!(saved.display_filter, Some(display_filter));
    });
}

/// A freshly added custom row defaults to the `Raw` transport — remote-ness
/// is the lane's job now, so a brand-new row has no reason to default to
/// the deprecated agent-side host axis.
#[gpui::test]
fn a_new_custom_row_defaults_to_the_raw_transport(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| w.add_custom_agent_row_for_test(window, cx));
    })
    .unwrap();
    win.read_with(cx, |w, cx| {
        let selected = w
            .agent_editable_row(1)
            .unwrap()
            .transport_select
            .read(cx)
            .selected_value()
            .map(|v| v.to_string());
        assert_eq!(selected.as_deref(), Some("raw"));
    });
}

/// The built-in default row's command is `npx`-prefixed, so it never gets a
/// local-PATH warning — daruda provisions Node.js itself for these.
#[gpui::test]
fn the_default_npx_row_has_no_path_warning(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, _cx| {
        assert_eq!(w.agent_editable_row(0).unwrap().path_warning, None);
    });
}

/// A custom row whose command names a binary that is not on `PATH` shows a
/// warning, but Save still succeeds — registering an agent before installing
/// its CLI (or adding it to PATH) is a legitimate flow, so the check must
/// only warn, never block.
#[gpui::test]
fn a_custom_row_with_a_missing_command_warns_but_still_saves(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| {
            w.add_agent_row(
                daruda_config::AgentDefinition {
                    id: "local-cli".to_string(),
                    name: "Local CLI".to_string(),
                    launch: daruda_config::AgentLaunch::Raw(
                        "daruda-settings-path-warning-test-missing-binary acp".to_string(),
                    ),
                    default_mode: None,
                    default_model: None,
                    fold_mode: None,
                    tail_window: None,
                    display_filter: None,
                },
                None,
                window,
                cx,
            );
        });
    })
    .unwrap();

    win.read_with(cx, |w, cx| {
        assert_eq!(
            w.agent_editable_row(1).unwrap().path_warning.as_deref(),
            Some("daruda-settings-path-warning-test-missing-binary")
        );
        assert!(
            w.validate(cx).is_ok(),
            "a missing local command must not block Save"
        );
    });
}

/// Editing a row's command recomputes the warning live — typing a missing
/// binary shows it, and correcting the command to a real one clears it.
#[gpui::test]
fn editing_the_command_field_recomputes_the_path_warning(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_agent_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.command_input.clone(),
        "daruda-settings-path-warning-test-missing-binary",
    );
    win.read_with(cx, |w, _cx| {
        assert_eq!(
            w.agent_editable_row(0).unwrap().path_warning.as_deref(),
            Some("daruda-settings-path-warning-test-missing-binary")
        );
    });

    set_agent_row_input(&wh, &win, cx, 0, |r| r.command_input.clone(), "sh -c true");
    win.read_with(cx, |w, _cx| {
        assert_eq!(w.agent_editable_row(0).unwrap().path_warning, None);
    });
}

/// The cached PATH warning is transport-independent by design — switching a
/// row to ssh does not clear it. The ssh/docker exemption is applied at
/// render time instead (`sections::agent::render_agent_catalog_row`, unit-
/// tested by `transport_needs_local_path_check`), since that needs no fresh
/// `which` call. Save must still succeed: an ssh row's command runs on the
/// remote host, so this machine's PATH is irrelevant to it.
#[gpui::test]
fn switching_transport_does_not_clear_the_cached_path_warning(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_agent_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.command_input.clone(),
        "daruda-settings-path-warning-test-missing-binary",
    );
    win.read_with(cx, |w, _cx| {
        assert!(w.agent_editable_row(0).unwrap().path_warning.is_some());
    });

    select_agent_transport(&wh, &win, cx, 0, "ssh");
    set_agent_row_input(&wh, &win, cx, 0, |r| r.host_input.clone(), "vm-work");
    win.read_with(cx, |w, cx| {
        assert!(w.agent_editable_row(0).unwrap().path_warning.is_some());
        assert!(
            w.validate(cx).is_ok(),
            "an ssh row's command runs remotely, so Save must succeed regardless"
        );
    });
}

/// Picking a preset that ships binaries only must not silently do nothing:
/// no row is added, and the section has an install page to point at instead.
#[gpui::test]
fn picking_a_preset_that_needs_a_manual_install_adds_no_row(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(select_agent_preset(&wh, &win, cx, "cursor"));
    win.read_with(cx, |w, cx| {
        let (name, install_url) = w
            .selected_preset_needs_install(cx)
            .expect("cursor ships prebuilt binaries, so it cannot be launched as-is");
        assert_eq!(name, "Cursor");
        assert!(install_url.starts_with("https://"), "{install_url}");
    });

    add_selected_agent_preset(&wh, &win, cx);
    win.read_with(cx, |w, cx| {
        assert_eq!(
            w.agent_editable_rows().count(),
            1,
            "a preset with no launch command must not become a row"
        );
        assert!(
            w.selected_preset_needs_install(cx).is_some(),
            "the install guidance stays up after the click"
        );
    });
}

/// A launchable pick clears the install guidance — the two states are exclusive.
#[gpui::test]
fn a_launchable_preset_shows_no_install_guidance(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    assert!(select_agent_preset(&wh, &win, cx, "codex-acp"));
    win.read_with(cx, |w, cx| {
        assert!(w.selected_preset_needs_install(cx).is_none());
    });
}

/// A catalog entry the editor cannot render a row for (a preset id daruda no
/// longer knows) must survive a Save — the alternative is silently deleting
/// part of the user's config on an unrelated settings change.
#[gpui::test]
fn validate_preserves_unresolved_agent_catalog_entries(cx: &mut TestAppContext) {
    let unresolved = daruda_config::AgentEntry::Preset {
        preset: "retired-agent".to_string(),
        overrides: daruda_config::PresetOverrides::default(),
    };
    let claude =
        daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition::claude_default());
    let config = daruda_config::Config {
        // Unresolved first, editable second.
        agents: vec![unresolved.clone(), claude.clone()],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        // Only the resolvable entry got an editable row…
        assert_eq!(w.agent_editable_rows().count(), 1);
        // …the other is held for the section's "not available" warning, which is
        // the only place the user can find out why that agent never shows up…
        assert_eq!(
            w.agent_unresolved_entries()
                .map(|(_, e)| e.clone())
                .collect::<Vec<_>>(),
            vec![unresolved.clone()]
        );
        let cfg = w.validate(cx).expect("agent catalog must validate");
        // …and it is still there after a save, in its original position.
        assert_eq!(cfg.agents, vec![unresolved, claude], "order preserved");
    });

    // A catalog whose every entry is unresolvable still has entries, so a Save
    // of any unrelated setting must go through. `preset = "cursor"` reaches
    // this with a preset daruda ships today — the manual-install ones resolve
    // to nothing.
    let needs_install = daruda_config::AgentEntry::Preset {
        preset: "cursor".to_string(),
        overrides: daruda_config::PresetOverrides::default(),
    };
    let config = daruda_config::Config {
        agents: vec![needs_install.clone()],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        assert!(w.agent_editable_rows().next().is_none());
        assert_eq!(
            w.agent_unresolved_entries()
                .map(|(_, e)| e.clone())
                .collect::<Vec<_>>(),
            vec![needs_install.clone()]
        );
        let cfg = w
            .validate(cx)
            .expect("an unresolved-only catalog is not an empty catalog");
        assert_eq!(cfg.agents, vec![needs_install]);
        // The runtime still has an agent to launch: the fallback in
        // `resolved_agents` covers what the persisted catalog cannot.
        assert!(!cfg.resolved_agents().is_empty());
        // The one predicate the section's placeholder reads too, so it cannot
        // call this catalog empty while the Save above succeeds.
        assert!(!w.agent_catalog_is_empty());
    });
}

/// Removal addresses the catalog, not the editable subset: dropping the row
/// that sits *after* a non-editable entry must not drop the wrong entry.
#[gpui::test]
fn removing_a_row_after_an_unresolved_entry_drops_the_right_one(cx: &mut TestAppContext) {
    let unresolved = daruda_config::AgentEntry::Preset {
        preset: "retired-agent".to_string(),
        overrides: daruda_config::PresetOverrides::default(),
    };
    let config = daruda_config::Config {
        agents: vec![
            unresolved.clone(),
            daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition::claude_default()),
        ],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    // The sole editable row is at catalog index 1, not 0.
    let catalog_index = win.read_with(cx, |w, _| {
        w.agent_editable_rows().next().expect("one editable row").0
    });
    assert_eq!(catalog_index, 1);
    win.update(cx, |w, cx| w.remove_agent_catalog_item(catalog_index, cx));
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("the unresolved entry still counts");
        assert_eq!(cfg.agents, vec![unresolved], "only the row went away");
    });
}

#[gpui::test]
fn validate_rejects_agent_catalog_errors(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| w.remove_agent_catalog_item(0, cx));
    win.read_with(cx, |w, cx| {
        assert_eq!(w.agent_editable_rows().count(), 1);
        assert!(w.validate(cx).is_ok(), "the last valid agent is restored");
        assert!(w.error.is_some(), "the rejected removal is explained");
    });

    let config = daruda_config::Config {
        agents: vec![
            daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition::codex_default()),
            daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition::codex_default()),
        ],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("codex"));
    });

    let (wh, win) = build_window(cx);
    let id_input = win.read_with(cx, |w, _| w.agent_editable_row(0).unwrap().id_input.clone());
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

/// Regression: constructor-seeded agent rows (loaded from config, unlike
/// rows added at runtime via `add_agent_row`) must wire up every field's
/// subscription through `subscribe_agent_row`, including the mode and model
/// pickers — a hand-copied duplicate of that wiring in the constructor once
/// dropped the mode field's silently.
#[gpui::test]
fn a_mode_or_model_pick_persists_for_a_constructor_seeded_row(cx: &mut TestAppContext) {
    for (field, expected) in [
        (
            (|r: &AgentCatalogRow| r.default_mode_select.clone()) as fn(&AgentCatalogRow) -> _,
            ("plan", None),
        ),
        (
            (|r: &AgentCatalogRow| r.default_model_select.clone()) as fn(&AgentCatalogRow) -> _,
            ("opus", Some("opus")),
        ),
    ] {
        let (wh, win) = build_window(cx);
        set_agent_vocabulary(
            &wh,
            &win,
            cx,
            daruda_store::agent_vocabulary::AgentVocabularyCache::default(),
        );
        win.update(cx, |w, cx| {
            w.error = Some("boom".into());
            cx.notify();
        });
        confirm_agent_row_select(&wh, &win, cx, 0, field, expected.0);
        win.read_with(cx, |w, cx| {
            assert!(w.error.is_none(), "a successful persist clears the error");
            let agents = &crate::settings_store::SettingsStore::global(cx)
                .user()
                .agents;
            let daruda_config::AgentEntry::Custom(definition) = &agents[0] else {
                panic!("the built-in default is a custom entry");
            };
            let (mode, model) = match expected.1 {
                Some(model) => (None, Some(model.to_string())),
                None => (Some(expected.0.to_string()), None),
            };
            assert_eq!(definition.default_mode, mode);
            assert_eq!(definition.default_model, model);
        });
    }
}

/// The cached vocabulary is keyed on the row's **id**, which stays editable
/// after the row was built — retyping it has to re-source both pickers. The
/// two axes are sourced independently: a cache that only knows this agent's
/// modes must not erase the models the adapter seed supplies.
#[gpui::test]
fn retyping_a_row_id_switches_the_pickers_to_that_agents_vocabulary(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let mut vocabulary = daruda_store::agent_vocabulary::AgentVocabularyCache::default();
    vocabulary.record_modes(
        "beta",
        "npx -y @agentclientprotocol/claude-agent-acp@latest",
        vec![daruda_store::agent_vocabulary::VocabEntry::new(
            "beta-mode",
            "Beta Mode",
        )],
    );
    set_agent_vocabulary(&wh, &win, cx, vocabulary);

    let mode = |r: &AgentCatalogRow| r.default_mode_select.clone();
    let model = |r: &AgentCatalogRow| r.default_model_select.clone();
    // The built-in row's id is "claude", which the cache knows nothing about,
    // so both axes come from the Claude adapter seed its command names.
    assert!(agent_row_select_offers(&wh, &win, cx, 0, mode, "plan"));
    assert!(!agent_row_select_offers(
        &wh,
        &win,
        cx,
        0,
        mode,
        "beta-mode"
    ));
    assert!(agent_row_select_offers(&wh, &win, cx, 0, model, "opus"));

    set_agent_row_input(&wh, &win, cx, 0, |r| r.id_input.clone(), "beta");
    assert!(agent_row_select_offers(&wh, &win, cx, 0, mode, "beta-mode"));
    assert!(
        !agent_row_select_offers(&wh, &win, cx, 0, mode, "plan"),
        "a live mode list replaces the seed rather than joining it"
    );
    assert!(
        agent_row_select_offers(&wh, &win, cx, 0, model, "opus"),
        "the cache knows no models for this agent, so the seed still fills that axis"
    );
}

/// An id is stable across edits, but its command is not. Vocabulary learned
/// from the old adapter must not follow the row after that command changes.
#[gpui::test]
fn changing_a_rows_command_invalidates_cached_vocabulary_from_the_old_adapter(
    cx: &mut TestAppContext,
) {
    const CLAUDE: &str = "npx -y @agentclientprotocol/claude-agent-acp@latest";
    const CODEX: &str = "npx -y @agentclientprotocol/codex-acp@latest";

    let (wh, win) = build_window(cx);
    let mut vocabulary = daruda_store::agent_vocabulary::AgentVocabularyCache::default();
    vocabulary.record_modes(
        "claude",
        CLAUDE,
        vec![daruda_store::agent_vocabulary::VocabEntry::new(
            "claude-live-only",
            "Claude Live Only",
        )],
    );
    set_agent_vocabulary(&wh, &win, cx, vocabulary);

    let mode = |row: &AgentCatalogRow| row.default_mode_select.clone();
    set_agent_row_input(&wh, &win, cx, 0, |row| row.command_input.clone(), CODEX);
    assert!(
        !agent_row_select_offers(&wh, &win, cx, 0, mode, "claude-live-only"),
        "the previous adapter's live vocabulary is suppressed"
    );
    assert!(
        agent_row_select_offers(&wh, &win, cx, 0, mode, "agent"),
        "until Codex connects, its own seed supplies the replacement vocabulary"
    );
}

/// A mode pinned before the agent was ever connected — and for an adapter
/// daruda has no seed for — must survive as a selectable entry, or opening
/// Settings would silently drop it on the next save.
#[gpui::test]
fn a_saved_value_the_vocabulary_does_not_list_is_kept(cx: &mut TestAppContext) {
    let entry = daruda_config::AgentEntry::Custom(daruda_config::AgentDefinition {
        // Not a preset id: `AgentEntry::for_definition` promotes a definition
        // whose id names a known preset, which would make this a Preset entry.
        id: "hand-written".to_string(),
        name: "Hand Written".to_string(),
        launch: daruda_config::AgentLaunch::Raw(
            "npx -y @google/gemini-cli@latest --acp".to_string(),
        ),
        default_mode: Some("legacy-mode".to_string()),
        default_model: Some("legacy-model".to_string()),
        fold_mode: None,
        tail_window: None,
        display_filter: None,
    });
    let config = daruda_config::Config {
        agents: vec![entry.clone()],
        ..Default::default()
    };
    let (wh, win) = build_window_with_config(cx, config.clone());
    set_agent_vocabulary(
        &wh,
        &win,
        cx,
        daruda_store::agent_vocabulary::AgentVocabularyCache::default(),
    );
    win.read_with(cx, |w, cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.default_mode(cx).as_deref(), Some("legacy-mode"));
        assert_eq!(row.default_model(cx).as_deref(), Some("legacy-model"));
        assert_eq!(w.validate(cx).expect("must validate").agents, config.agents);
    });
}

#[gpui::test]
fn basic_toggles_and_section_focus(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::General);
    });

    let (initial_cursor, initial_close) =
        win.read_with(cx, |w, _| (w.cursor_blinking, w.close_pane_on_exit));
    win.update(cx, |w, cx| {
        w.cursor_blinking = !w.cursor_blinking;
        w.close_pane_on_exit = !w.close_pane_on_exit;
        cx.notify();
    });
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("toggled settings must validate");
        assert_eq!(cfg.cursor.blinking, !initial_cursor);
        assert_eq!(cfg.shell.close_pane_on_exit, !initial_close);
    });

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

/// Regression: `validate()` must never let clicking Save revert a
/// Telegram pairing that completed asynchronously (the bridge's poll
/// loop, via `InboundAction::Paired`) while the Settings window was
/// still open. `base_config` is a snapshot taken at window-open time,
/// so the fix re-reads `authorized_chat_id` live from `SettingsStore`
/// instead of trusting that stale snapshot.
#[gpui::test]
fn validate_does_not_revert_background_pairing(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    // Simulate the poll loop's `InboundAction::Paired` persist step —
    // it mutates `SettingsStore`'s live config directly, bypassing this
    // window entirely, exactly as `telegram::global::dispatch_action`
    // does. Uses `set_user_for_testing` (in-memory only), NOT the real
    // `patch_user` — `patch_user` calls `daruda_config::patch_config_file`,
    // which writes to the actual `config_path()` on disk regardless of
    // test context (there is no test-mode redirect), so calling it here
    // would silently overwrite the developer's real `~/Library/Application
    // Support/daruda/config.toml` on every test run. This bug shipped and
    // was caught by a live user report of a repeatedly-clobbered config file.
    cx.update(|cx| {
        use gpui::BorrowAppContext as _;
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user_arc()).clone();
            cfg.telegram.authorized_chat_id = Some(42);
            store.set_user_for_testing(cfg);
        });
    });

    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("defaults plus a live pairing must still validate");
        assert_eq!(
            cfg.telegram.authorized_chat_id,
            Some(42),
            "Save must not revert a pairing that completed while Settings was open"
        );
    });
}

#[gpui::test]
fn telegram_enabled_toggle_round_trips_through_validate(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        assert!(!w.validate(cx).unwrap().telegram.enabled);
    });

    win.update(cx, |w, cx| {
        w.telegram_enabled = true;
        cx.notify();
    });

    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).unwrap().telegram.enabled);
    });
}

#[gpui::test]
async fn copy_telegram_pair_command_writes_clipboard_and_reverts_label(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    // Write something else first so we can verify the copy overwrites it.
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_string()));
    });

    cx.update(|cx| {
        win.update(cx, |w, cx| {
            w.copy_telegram_pair_command_for_test("AB12CD", cx)
        });
    });

    let actual = cx.read_from_clipboard().expect("clipboard populated");
    let text = actual.text().expect("clipboard item is text");
    assert_eq!(text, "/pair AB12CD");

    win.read_with(cx, |w, _cx| {
        assert!(
            w.telegram_pair_command_copied(),
            "Copied confirmation should be active"
        );
    });

    // Past the 1 s revert window — virtual clock so the test is
    // deterministic.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1500));
    cx.run_until_parked();

    win.read_with(cx, |w, _cx| {
        assert!(
            !w.telegram_pair_command_copied(),
            "Copied label should have reverted to Copy"
        );
    });
}

// ---- Session Hosts section ----

/// Test-only — click "Add Host".
fn add_session_host(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.add_session_host_row(window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — pick `kind` in one session-host row's kind dropdown.
fn select_session_host_kind(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    kind: &str,
) {
    let state = win.read_with(cx, |w, _| {
        w.session_host_row(index).unwrap().kind_select.clone()
    });
    let value = SharedString::from(kind.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — write `value` into one session-host row's input.
fn set_session_host_row_input(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_owned(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

#[gpui::test]
fn validate_session_host_catalog_cases(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("an empty registry is a valid state");
        assert!(cfg.session_hosts.is_empty());
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Build box");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a filled-in ssh row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_eq!(cfg.session_hosts[0].label, "Build box");
        assert_eq!(
            cfg.session_hosts[0].kind,
            daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string()
            }
        );
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Dev container");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");
    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("a filled-in docker row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_eq!(
            cfg.session_hosts[0].kind,
            daruda_config::SessionHostKind::Docker {
                container: "dev-1".to_string()
            }
        );
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains('1'));
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Build Box");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.label_input.clone(), "  build box  ");
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.target_input.clone(), "vm-other");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("build box") || err.contains("Build Box"));
    });

    // `target`/`container` are validated through the exact same
    // `lane::session_host::checked_bare_word` `SessionHostModal` uses.
    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Box");
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.target_input.clone(),
        "box; rm -rf /",
    );
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_err());
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Box");
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_err());
    });
}

/// Deleting a row that was loaded from config and saving must record a
/// tombstone for it.
#[gpui::test]
fn deleting_a_loaded_row_and_saving_records_a_tombstone(cx: &mut TestAppContext) {
    let id = daruda_store::project::SessionHostId::new();
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("removing the only row is a valid save");
        assert!(cfg.session_hosts.is_empty());
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, id);
        assert_eq!(cfg.session_host_tombstones[0].value, "vm-work");
        assert_eq!(cfg.session_host_tombstones[0].redirected_to, None);
    });
}

/// Re-adding a host with the same `(kind, value)` as a just-deleted one sets
/// `redirected_to` on the most recent matching tombstone, so a lane still
/// referencing the deleted id resolves through the redirect chain.
#[gpui::test]
fn readding_the_same_target_redirects_the_deleted_tombstone(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id: old_id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let (wh, win) = build_window_with_config(cx, config);
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.label_input.clone(),
        "Build box again",
    );
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("the recreated row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        let new_id = cfg.session_hosts[0].id;
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, old_id);
        assert_eq!(cfg.session_host_tombstones[0].redirected_to, Some(new_id));
    });
}

/// A one-row SSH catalog plus the lane host linked to it — the starting
/// state for the retype tests below.
fn ssh_catalog_config(
    id: daruda_store::project::SessionHostId,
) -> (
    daruda_config::Config,
    daruda_store::project::LaneSessionHost,
) {
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let lane_host = daruda_store::project::LaneSessionHost::Ssh {
        target: "vm-work".to_string(),
        session_path: "/srv/app".to_string(),
        registry_id: Some(id),
    };
    (config, lane_host)
}

/// Changing a loaded row's Type retires its id. A lane's `registry_id`
/// resolves on id *and* kind, so keeping the id would leave every linked lane
/// unresolvable with nothing recorded anywhere; retiring it records the
/// removal and surfaces the lane as Orphaned (banner + "keep current" path)
/// instead of silently stale.
#[gpui::test]
fn retyping_a_loaded_row_retires_its_id_and_orphans_the_linked_lane(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let (config, lane_host) = ssh_catalog_config(old_id);
    let (wh, win) = build_window_with_config(cx, config);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a retyped row must still validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_ne!(
            cfg.session_hosts[0].id, old_id,
            "a row that switched Type must not keep the id SSH lanes point at"
        );
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, old_id);
        assert_eq!(
            cfg.session_host_tombstones[0].kind,
            daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string()
            }
        );
        assert_eq!(
            cfg.session_host_tombstones[0].redirected_to, None,
            "an SSH lane must never be redirected onto a Docker entry"
        );

        use crate::lane::session_host;
        assert_eq!(
            session_host::registry_link_status(&lane_host, &cfg.session_hosts),
            session_host::LinkStatus::Orphaned
        );
        assert_eq!(
            session_host::effective_session_host(
                Some(&lane_host),
                None,
                &daruda_config::AgentLaunch::Raw("adapter".to_string()),
                &cfg.session_hosts,
                &cfg.session_host_tombstones,
            ),
            lane_host,
            "an orphaned lane keeps its cached target, never the Docker one"
        );
    });
}

/// What the retype's tombstone buys: registering the SSH host again later
/// redirects the retired id, so the linked lane resolves to the new entry
/// instead of sitting on a stale cache forever.
#[gpui::test]
fn re_registering_a_retyped_host_heals_the_linked_lane(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let (config, lane_host) = ssh_catalog_config(old_id);

    let (wh, win) = build_window_with_config(cx, config);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");
    let after_retype = win.read_with(cx, |w, cx| w.validate(cx).expect("retype must validate"));

    // Second Save, over the config the first one produced: the user
    // re-registers the SSH host they moved off of.
    let (wh, win) = build_window_with_config(cx, after_retype);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        1,
        |r| r.label_input.clone(),
        "Build box (ssh)",
    );
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.target_input.clone(), "vm-work");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("re-registering must validate");
        let new_ssh_id = cfg
            .session_hosts
            .iter()
            .find(|e| matches!(e.kind, daruda_config::SessionHostKind::Ssh { .. }))
            .expect("the re-registered ssh entry")
            .id;

        use crate::lane::session_host;
        assert_eq!(
            session_host::resolved_registry_id(
                &lane_host,
                &cfg.session_hosts,
                &cfg.session_host_tombstones,
            ),
            Some(new_ssh_id),
            "the retired id must redirect onto the re-registered ssh entry"
        );
    });
}

#[gpui::test]
fn removing_a_session_host_row_updates_the_row_list_immediately(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    add_session_host(&wh, &win, cx);
    win.read_with(cx, |w, _| assert_eq!(w.session_host_rows().count(), 2));
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    win.read_with(cx, |w, _| assert_eq!(w.session_host_rows().count(), 1));
}

// ---- Session Hosts tab cycle ----

/// Test-only — open the Session Hosts page, landing focus where a sidebar
/// click would.
fn open_session_hosts_section(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.focus_section(BuiltinSection::SessionHosts, window, cx)
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — one Tab (`forward`) / Shift-Tab press.
fn press_tab(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    forward: bool,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.focus_next_input(forward, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — whether one session-host row's input currently holds focus.
fn session_host_input_is_focused(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
) -> bool {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).is_focused(window)
    })
    .expect("settings window should still be open during the test")
}

fn focus_session_host_input(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
) {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).focus(window, cx);
    })
    .expect("settings window should still be open during the test");
}

fn session_host_kind_is_focused(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsWindow>,
    cx: &mut TestAppContext,
    index: usize,
) -> bool {
    let state = win.read_with(cx, |w, _| {
        w.session_host_row(index).unwrap().kind_select.clone()
    });
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).is_focused(window)
    })
    .expect("settings window should still be open during the test")
}

/// A one-row SSH catalog config — the seed for the tab-cycle tests.
fn one_ssh_row_config() -> daruda_config::Config {
    daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id: daruda_store::project::SessionHostId::new(),
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    }
}

/// Tab has to walk a row's fields, not sit on its label: the section's
/// precomputed focus list holds only the first field, so the cycle is built
/// from the live rows instead.
#[gpui::test]
fn tab_cycle_covers_session_host_rows(cx: &mut TestAppContext) {
    let (wh, win) = build_window_with_config(cx, one_ssh_row_config());
    open_session_hosts_section(&wh, &win, cx);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "opening the page lands on the first row's label"
    );

    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_kind_is_focused(&wh, &win, cx, 0),
        "Tab from the label must reach the row's kind select"
    );
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "Tab from the kind select must reach the row's target field"
    );

    press_tab(&wh, &win, cx, false);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, false);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "Shift-Tab must walk back to the label"
    );

    add_session_host(&wh, &win, cx);
    select_session_host_kind(&wh, &win, cx, 1, "docker");

    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "the first row's value field comes after its label"
    );
    press_tab(&wh, &win, cx, true);
    // The second row's Remove button is a real tab stop and precedes its
    // fields in render order.
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 1, |r| r.label_input.clone()),
        "the cycle carries on into the second row"
    );
    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 1));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 1, |r| r.container_input.clone()),
        "a Docker row's container field is the one on screen"
    );
    assert!(
        !session_host_input_is_focused(&wh, &win, cx, 1, |r| r.target_input.clone()),
        "the hidden target field must never take focus"
    );
    let (wh, win) = build_window(cx);
    open_session_hosts_section(&wh, &win, cx);
    add_session_host(&wh, &win, cx);

    focus_session_host_input(&wh, &win, cx, 0, |r| r.label_input.clone());
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "the first field of a newly added row can enter the native tab order"
    );
    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "and carries on to its value field"
    );
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
