use super::*;

/// A card opens folded, except for what it hides: a row already running a
/// command other than its preset's opens with the advanced block showing.
/// The built-in Claude's own id is not such a difference.
#[gpui::test]
fn a_card_opens_its_advanced_block_when_it_runs_something_else(cx: &mut TestAppContext) {
    let config = daruda_config::Config {
        agents: vec![
            daruda_config::AgentEntry::for_definition(
                daruda_config::AgentDefinition::claude_default(),
                None,
            ),
            daruda_config::AgentEntry::preset_with(
                "codex-acp",
                daruda_config::PresetOverrides {
                    command: Some("npx -y @agentclientprotocol/codex-acp@1.0.0".to_string()),
                    ..daruda_config::PresetOverrides::default()
                },
            ),
        ],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        assert_eq!(w.agent_card_fold_for_test(0), Some(CardFold::default()));
        assert!(!w.agent_editable_row(0).unwrap().advanced_overridden(cx));
        assert_eq!(
            w.agent_card_fold_for_test(1),
            Some(CardFold {
                expanded: false,
                advanced: true,
            })
        );
        assert!(w.agent_editable_row(1).unwrap().advanced_overridden(cx));
    });
}

/// Switching a preset on is the whole gesture, so its card arrives folded; a
/// custom agent opens both its details and its advanced block, where the
/// command it still needs lives.
#[gpui::test]
fn a_custom_card_opens_ready_to_edit_and_a_preset_arrives_folded(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.read_with(cx, |w, _| {
        assert_eq!(w.agent_card_fold_for_test(1), Some(CardFold::default()));
    });
    let settings = win.clone();
    wh.update(cx, |_root, window, cx| {
        settings.update(cx, |w, cx| w.add_custom_agent_row_for_test(window, cx));
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(
            w.agent_card_fold_for_test(2),
            Some(CardFold {
                expanded: true,
                advanced: true,
            })
        );
    });
}

/// Folding is view state: it writes nothing, and a save that rebuilds the
/// catalog from the written config keeps the card the user is editing open.
#[gpui::test]
fn folding_a_card_saves_nothing_and_survives_a_save(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let before = cx.read(|cx| {
        crate::store::SettingsStore::global(cx)
            .user()
            .agents
            .clone()
    });
    win.update(cx, |w, cx| {
        w.toggle_agent_card_expanded(0, cx);
        w.toggle_agent_card_advanced(0, cx);
    });
    let after = cx.read(|cx| {
        crate::store::SettingsStore::global(cx)
            .user()
            .agents
            .clone()
    });
    assert_eq!(after, before, "a fold toggle writes nothing");

    // A tail pick persists and then reloads every row from the live config.
    confirm_agent_row_select(&wh, &win, cx, 0, |r| r.tail_window_select.clone(), "3");
    win.read_with(cx, |w, _| {
        assert_eq!(
            w.agent_card_fold_for_test(0),
            Some(CardFold {
                expanded: true,
                advanced: true,
            })
        );
    });
}

/// A preset that ships binaries only has nothing to run, so switching it on
/// adds no entry.
#[gpui::test]
fn enabling_a_preset_that_needs_a_manual_install_adds_no_row(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "cursor");
    win.read_with(cx, |w, _| assert_eq!(w.agent_editable_rows().count(), 1));
}

/// Switching an entry off keeps it — every field it states included — and
/// saves it as off; the runtime catalog then leaves it out.
#[gpui::test]
fn switching_an_entry_off_keeps_its_fields(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    confirm_agent_row_select(&wh, &win, cx, 1, |r| r.tail_window_select.clone(), "3");
    win.update(cx, |w, cx| w.set_agent_enabled(1, false, cx));

    cx.read(|cx| {
        let user = crate::store::SettingsStore::global(cx).user();
        let gemini = &user.agents[1];
        assert!(!gemini.enabled);
        assert_eq!(gemini.resolve().and_then(|d| d.tail_window), Some(3));
        let ids: Vec<String> = user.resolved_agents().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["claude"]);
    });
}

/// The only agent still on cannot be switched off: a new chat always has one
/// to open with.
#[gpui::test]
fn the_last_agent_on_cannot_be_switched_off(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.update(cx, |w, cx| w.set_agent_enabled(0, false, cx));
    win.update(cx, |w, cx| {
        assert!(w.agent_is_last_enabled(1));
        w.set_agent_enabled(1, false, cx);
    });
    win.read_with(cx, |w, _| {
        assert!(w.agent_editable_row(1).unwrap().enabled, "refused");
        assert_eq!(
            w.agent_default_index(),
            Some(1),
            "the next one on is the default"
        );
    });
}

/// "Make default" moves the entry to the front — the first entry on is the
/// one a new chat opens with.
#[gpui::test]
fn making_an_entry_the_default_moves_it_first(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.update(cx, |w, cx| w.make_agent_default(1, cx));
    cx.read(|cx| {
        let user = crate::store::SettingsStore::global(cx).user();
        assert_eq!(user.resolved_agents()[0].id, "gemini");
    });
    win.read_with(cx, |w, _| assert_eq!(w.agent_default_index(), Some(0)));
}

/// Clicking a card's chevrons in the rendered window flips each fold exactly
/// once. The advanced chevron sits inside a row that owns the same toggle, and
/// gpui fires every hovered ancestor's click: a listener on both would undo
/// itself, which calling the toggle directly can never show.
#[gpui::test]
fn clicking_a_cards_chevrons_flips_each_fold_once(cx: &mut TestAppContext) {
    use gpui::{VisualTestContext, size};

    let (window, settings) = build_window(cx);
    cx.simulate_window_resize(window.into(), size(px(1400.), px(6000.)));
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.update(|window, cx| {
        settings.update(cx, |settings, cx| {
            settings.focus_section(BuiltinSection::Agent, window, cx);
            settings.toggle_agent_card_expanded(0, cx);
        });
    });
    vcx.run_until_parked();

    let click = |vcx: &mut VisualTestContext, selector: &'static str| {
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"));
        vcx.simulate_click(bounds.center(), Default::default());
        vcx.run_until_parked();
    };
    let fold = |vcx: &mut VisualTestContext| {
        settings.read_with(vcx, |w, _| w.agent_card_fold_for_test(0).unwrap())
    };

    click(&mut vcx, "agent-card-advanced-fold-0");
    assert!(
        fold(&mut vcx).advanced,
        "one click opens the advanced block"
    );
    click(&mut vcx, "agent-card-advanced-fold-0");
    assert!(!fold(&mut vcx).advanced, "and one more folds it");

    click(&mut vcx, "agent-card-fold-0");
    assert!(
        !fold(&mut vcx).expanded,
        "the header chevron folds the card"
    );
}

/// A custom row that runs a preset's command verbatim is saved as a reference
/// to that preset; the row then says so too, so the catalog stops offering
/// the preset as unused — switching it on again would add the same agent twice.
#[gpui::test]
fn a_custom_row_saved_as_a_preset_reference_takes_that_preset(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let settings = win.clone();
    wh.update(cx, |_root, window, cx| {
        settings.update(cx, |w, cx| w.add_custom_agent_row_for_test(window, cx));
    })
    .unwrap();
    let gemini = daruda_config::AgentDefinition::registry_preset("gemini").expect("runnable");
    let daruda_config::AgentLaunch::Raw(command) = gemini.launch else {
        panic!("a preset runs a plain command");
    };
    set_agent_row_input(&wh, &win, cx, 1, |r| r.id_input.clone(), "my-gemini");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.name_input.clone(), "My Gemini");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.command_input.clone(), &command);
    win.read_with(cx, |w, _| {
        assert_eq!(w.agent_editable_row(1).unwrap().preset, None)
    });

    win.update(cx, |w, cx| assert!(w.persist_agent_catalog(cx)));

    win.read_with(cx, |w, _| {
        assert_eq!(
            w.agent_editable_row(1).unwrap().preset.as_deref(),
            Some("gemini"),
            "the row takes the origin its saved entry has"
        );
    });
    cx.read(|cx| {
        let user = crate::store::SettingsStore::global(cx).user();
        assert_eq!(user.agents[1].preset_id(), Some("gemini"));
        assert_eq!(user.resolved_agents()[1].id, "my-gemini");
    });
}

/// Only an entry that is on can become the default; the method refuses a
/// switched-off one even though the card never offers it.
#[gpui::test]
fn a_switched_off_entry_cannot_be_made_the_default(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.update(cx, |w, cx| {
        w.set_agent_enabled(1, false, cx);
        w.make_agent_default(1, cx);
    });
    cx.read(|cx| {
        let user = crate::store::SettingsStore::global(cx).user();
        assert_eq!(
            user.agents[0].resolve().map(|d| d.id).as_deref(),
            Some("claude")
        );
    });
}

/// A custom row that runs a preset's command and states no environment is
/// saved as a reference that follows the preset's environment — and has to
/// keep following it on every later save. Its empty field never held the
/// preset's environment, so emptiness is not the user clearing it.
#[gpui::test]
fn a_promoted_rows_empty_environment_keeps_following_the_preset(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let settings = win.clone();
    wh.update(cx, |_root, window, cx| {
        settings.update(cx, |w, cx| w.add_custom_agent_row_for_test(window, cx));
    })
    .unwrap();
    let codex = daruda_config::AgentDefinition::registry_preset("codex-acp").expect("runnable");
    let daruda_config::AgentLaunch::Raw(command) = codex.launch.clone() else {
        panic!("a preset runs a plain command");
    };
    set_agent_row_input(&wh, &win, cx, 1, |r| r.id_input.clone(), "my-codex");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.name_input.clone(), "My Codex");
    set_agent_row_input(&wh, &win, cx, 1, |r| r.command_input.clone(), &command);

    for save in ["first", "second"] {
        win.update(cx, |w, cx| {
            assert!(w.persist_agent_catalog(cx), "{save} save")
        });
        cx.read(|cx| {
            let user = crate::store::SettingsStore::global(cx).user();
            assert_eq!(
                user.agents[1],
                daruda_config::AgentEntry::preset_with(
                    "codex-acp",
                    daruda_config::PresetOverrides {
                        id: Some("my-codex".to_string()),
                        name: Some("My Codex".to_string()),
                        ..daruda_config::PresetOverrides::default()
                    }
                ),
                "{save} save states no environment of its own"
            );
            assert_eq!(user.resolved_agents()[1].env, codex.env, "{save} save");
        });
    }
    win.read_with(cx, |w, cx| {
        assert!(!w.agent_editable_row(1).unwrap().env_overridden(cx));
    });
}
