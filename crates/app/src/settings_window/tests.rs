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
fn validate_preserves_an_unresolved_catalog_entry(cx: &mut TestAppContext) {
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
        // …and it is still there after a save.
        assert!(cfg.agents.contains(&unresolved), "{:?}", cfg.agents);
    });
}

/// A catalog whose every entry is unresolvable still has entries, so a Save of
/// any unrelated setting must go through. `preset = "cursor"` reaches this with
/// a preset daruda ships today — the manual-install ones resolve to nothing.
#[gpui::test]
fn validate_accepts_a_catalog_of_only_unresolved_entries(cx: &mut TestAppContext) {
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

/// A non-editable entry keeps its config position across a Save. It used to be
/// appended after the editable rows, so an entry the user had listed first came
/// back last — and would silently become a different default agent the day its
/// preset resolves again.
#[gpui::test]
fn an_unresolved_entry_keeps_its_position_across_a_save(cx: &mut TestAppContext) {
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
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(cfg.agents, vec![unresolved, claude], "order preserved");
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
fn validate_rejects_empty_agent_catalog(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| w.remove_agent_catalog_item(0, cx));
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("agent") || err.contains("에이전트"));
    });
}

#[gpui::test]
fn validate_rejects_duplicate_agent_id(cx: &mut TestAppContext) {
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
}

#[gpui::test]
fn validate_rejects_invalid_agent_id(cx: &mut TestAppContext) {
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
/// rows added at runtime via `add_agent_row`) must wire up every input's
/// submit/clear-error subscription through `subscribe_agent_row`, including
/// `default_mode_input` — a hand-copied duplicate of that wiring in the
/// constructor once dropped it silently.
#[gpui::test]
fn default_mode_input_change_clears_error_for_constructor_seeded_row(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.error = Some("boom".into());
        cx.notify();
    });
    let default_mode_input = win.read_with(cx, |w, _| {
        w.agent_editable_row(0).unwrap().default_mode_input.clone()
    });
    wh.update(cx, |_root, window, cx| {
        default_mode_input.update(cx, |i, cx_state| {
            i.set_value("plan".to_owned(), window, cx_state)
        });
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert!(w.error.is_none());
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
async fn copy_telegram_pair_command_writes_full_command_to_clipboard(cx: &mut TestAppContext) {
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
}

#[gpui::test]
async fn copy_telegram_pair_command_copied_label_reverts_after_one_second(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    cx.update(|cx| {
        win.update(cx, |w, cx| {
            w.copy_telegram_pair_command_for_test("AB12CD", cx)
        });
    });

    win.read_with(cx, |w, _cx| {
        assert!(
            w.telegram_pair_command_copied(),
            "Copied label should be up immediately"
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
