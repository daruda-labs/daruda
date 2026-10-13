use super::*;

#[gpui::test]
fn validate_collects_agent_catalog(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_for_add = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_for_add.update(cx, |w, cx| {
            // What switching a preset on does.
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
                // The built-in default references the `claude-acp` preset it
                // shares a command with, under its own stable id.
                daruda_config::AgentEntry::for_definition(
                    daruda_config::AgentDefinition::claude_default(),
                    None,
                ),
                // The added preset is stored as a reference, not a copy.
                daruda_config::AgentEntry::preset("codex-acp".to_string()),
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

/// Test-only — pick `kind` in one catalog row's transport dropdown.
fn select_agent_transport(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
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

/// Test-only — install a known vocabulary cache into the open Settings window.
fn set_agent_vocabulary(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
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

/// Switching a preset on saves a reference to it rather than a frozen copy of
/// its fields.
#[gpui::test]
fn enabling_a_preset_collects_a_reference(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.read_with(cx, |w, cx| {
        assert_eq!(w.agent_editable_rows().count(), 2);
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset("gemini".to_string())
        );
        // Nothing was edited, so nothing it runs differs from the preset.
        assert!(!w.agent_editable_row(1).unwrap().advanced_overridden(cx));
    });
}

/// Editing one field of a preset row overrides that field only — every other
/// field keeps following the preset.
#[gpui::test]
fn editing_one_field_of_a_preset_row_overrides_only_that_field(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.name_input.clone(), "My Gemini");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset_with(
                "gemini".to_string(),
                daruda_config::PresetOverrides {
                    name: Some("My Gemini".to_string()),
                    ..daruda_config::PresetOverrides::default()
                }
            )
        );
        let row = w.agent_editable_row(1).unwrap();
        assert!(!row.command_overridden(cx), "command still follows");
        assert!(
            !row.advanced_overridden(cx),
            "a rename changes nothing that runs"
        );
    });
}

/// A remote transport is not expressible as a preset override, so saving detaches
/// the row into a self-contained custom entry.
#[gpui::test]
fn switching_a_preset_row_to_ssh_detaches_it_into_a_custom_entry(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
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
            daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::new(
                base.id,
                base.name,
                daruda_config::AgentLaunch::Ssh {
                    adapter_command: command,
                    host: "vm-work".to_string(),
                }
            ))
        );
    });
}

/// The agent-side `Ssh`/`Docker` transport is deprecated (Session Host on
/// the Lane is the new axis) but stays fully functional for one more
/// release — an already-persisted `Ssh` row must keep loading and saving
/// byte-for-byte, untouched.
#[gpui::test]
fn an_existing_ssh_row_round_trips_unchanged_through_save(cx: &mut TestAppContext) {
    let ssh_entry = daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::new(
        "remote-claude".to_string(),
        "Remote Claude".to_string(),
        daruda_config::AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            host: "vm-work".to_string(),
        },
    ));
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
    let docker_entry = daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::new(
        "remote-codex".to_string(),
        "Remote Codex".to_string(),
        daruda_config::AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".to_string(),
            container: "dev-1".to_string(),
        },
    ));
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
                daruda_config::AgentDefinition::new(
                    "local-cli".to_string(),
                    "Local CLI".to_string(),
                    daruda_config::AgentLaunch::Raw(
                        "daruda-settings-path-warning-test-missing-binary acp".to_string(),
                    ),
                ),
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
/// render time instead (`sections::agent_catalog::render_agent_catalog_row`, unit-
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

/// A catalog entry the editor cannot render a row for (a preset id daruda no
/// longer knows) must survive a Save — the alternative is silently deleting
/// part of the user's config on an unrelated settings change.
#[gpui::test]
fn validate_preserves_unresolved_agent_catalog_entries(cx: &mut TestAppContext) {
    let unresolved = daruda_config::AgentEntry::preset("retired-agent".to_string());
    let claude = daruda_config::AgentEntry::for_definition(
        daruda_config::AgentDefinition::claude_default(),
        None,
    );
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
    let needs_install = daruda_config::AgentEntry::preset("cursor".to_string());
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
    let unresolved = daruda_config::AgentEntry::preset("retired-agent".to_string());
    let config = daruda_config::Config {
        agents: vec![
            unresolved.clone(),
            daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::claude_default()),
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
            daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::codex_default()),
            daruda_config::AgentEntry::custom(daruda_config::AgentDefinition::codex_default()),
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
            let agents = &crate::store::SettingsStore::global(cx).user().agents;
            let definition = agents[0].resolve().expect("the built-in default resolves");
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
    let entry = daruda_config::AgentEntry::custom(daruda_config::AgentDefinition {
        default_mode: Some("legacy-mode".to_string()),
        default_model: Some("legacy-model".to_string()),
        ..daruda_config::AgentDefinition::new(
            // No preset runs this command: `AgentEntry::for_definition` promotes
            // one that does, which would make this a Preset entry.
            "hand-written".to_string(),
            "Hand Written".to_string(),
            daruda_config::AgentLaunch::Raw("hand-written-agent --acp".to_string()),
        )
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
