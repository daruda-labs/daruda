use super::*;

/// A hand-written `[agents.env]` loads into the Environment field and comes
/// back out of it unchanged — a save driven by an unrelated field must not
/// silently drop it.
#[gpui::test]
fn an_agents_environment_survives_a_save_that_never_touched_it(cx: &mut TestAppContext) {
    let env = vec![(
        "CODEX_CONFIG".to_string(),
        r#"{"features":{"multi_agent_v2":true}}"#.to_string(),
    )];
    let stated = daruda_config::AgentDefinition {
        env: Some(env.clone()),
        ..daruda_config::AgentDefinition::new(
            "stated".to_string(),
            "Stated".to_string(),
            daruda_config::AgentLaunch::Raw("npx -y some-acp".to_string()),
        )
    };
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(stated.clone())],
        ..Default::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(cfg.agents, vec![daruda_config::AgentEntry::custom(stated)]);
        assert_eq!(cfg.resolved_agents()[0].env, Some(env));
    });
}

/// The opt-out the Environment field exists for: a `codex-acp` row inherits
/// the preset's `CODEX_CONFIG`, and emptying the field has to *clear* it
/// rather than fall back to following the preset. Asserted on the resolved
/// definition — what the adapter would actually launch with — not on the row.
#[gpui::test]
fn clearing_a_preset_shipping_environment_opts_the_row_out(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "codex-acp");

    let preset_env = daruda_config::AgentDefinition::registry_preset("codex-acp")
        .expect("codex-acp is runnable")
        .env
        .expect("the preset ships an environment");
    assert!(!preset_env.is_empty());

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset("codex-acp".to_string()),
            "the untouched row states nothing of its own"
        );
        assert_eq!(cfg.resolved_agents()[1].env.as_ref(), Some(&preset_env));
        assert!(
            !w.agent_editable_row(1).unwrap().env_overridden(cx),
            "showing the preset's own value is not an override"
        );
    });

    set_agent_row_input(&wh, &win, cx, 1, |r| r.env_input.clone(), "");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset_with(
                "codex-acp".to_string(),
                daruda_config::PresetOverrides {
                    env: Some(Vec::new()),
                    ..daruda_config::PresetOverrides::default()
                }
            ),
            "an emptied field is an explicit clear, not a return to the preset"
        );
        assert_eq!(
            cfg.resolved_agents()[1].env,
            Some(Vec::new()),
            "the launched environment must be empty, not the preset's"
        );
        assert!(
            w.agent_editable_row(1).unwrap().env_overridden(cx),
            "clearing the preset's environment is an override"
        );
    });
}

/// The Codex note and its caveat belong to a row that actually launches with
/// the overlay — not to the default catalog's custom `claude` row. Keyed on
/// the variable, so the answer tracks the field rather than the preset id:
/// clearing it drops the note, typing it in earns it.
#[gpui::test]
fn only_a_row_launching_the_codex_overlay_carries_its_note(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let ships = |win: &Entity<SettingsView>, cx: &mut TestAppContext, index: usize| {
        win.read_with(cx, |w, cx| {
            w.agent_editable_row(index)
                .unwrap()
                .ships_codex_subagent_overlay(cx)
        })
    };

    assert!(
        !ships(&win, cx, 0),
        "the default claude row ships no overlay"
    );

    enable_agent_preset(&wh, &win, cx, "codex-acp");
    assert!(ships(&win, cx, 1), "the codex row inherits the overlay");

    set_agent_row_input(&wh, &win, cx, 1, |r| r.env_input.clone(), "");
    assert!(
        !ships(&win, cx, 1),
        "a row that opted out is not told what the overlay does"
    );

    set_agent_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.env_input.clone(),
        "CODEX_CONFIG={\"features\":{\"multi_agent_v2\":true}}",
    );
    assert!(
        ships(&win, cx, 0),
        "a hand-built codex row earns the note without naming a preset"
    );
}

/// Codex's own injected instruction is "do not spawn sub-agents unless the
/// user *or applicable AGENTS.md/skill instructions* explicitly ask for it".
/// A caveat that names only the user reads as plainly wrong to anyone who set
/// the delegation up in AGENTS.md — the exact reader it is written for.
#[test]
fn the_codex_caveat_names_every_way_codex_is_asked_to_delegate() {
    for locale in ["en", "ko"] {
        let caveat = rust_i18n::t!("settings.agent_env_codex_caveat", locale = locale);
        assert!(
            caveat.contains("AGENTS.md"),
            "{locale}: the user is not the only thing codex takes a delegation from: {caveat}"
        );
    }
}

/// Typing an environment into a preset row overrides only that field, and a
/// custom row (nothing to clear) that leaves the field empty writes no key —
/// the third state, which is what keeps such a row free to pick up a preset
/// default added later.
#[gpui::test]
fn a_typed_environment_overrides_and_an_empty_one_stays_unstated(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");

    // `gemini` ships no environment, so an untouched field states none.
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(cfg.resolved_agents()[1].env, None);
    });

    set_agent_row_input(
        &wh,
        &win,
        cx,
        1,
        |r| r.env_input.clone(),
        "RUST_LOG=debug\nB=two words",
    );

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset_with(
                "gemini".to_string(),
                daruda_config::PresetOverrides {
                    env: Some(vec![
                        ("B".to_string(), "two words".to_string()),
                        ("RUST_LOG".to_string(), "debug".to_string()),
                    ]),
                    ..daruda_config::PresetOverrides::default()
                }
            )
        );
        assert!(
            w.agent_editable_row(1).unwrap().env_overridden(cx),
            "a row that states one where the preset states none is an override"
        );
    });
}

/// Emit the event the input widget raises for Enter. `secondary` is the
/// Cmd/Ctrl+Enter flavour; a multi-line field raises the plain one *after*
/// inserting the newline (`enter` in the vendored input state).
fn press_enter(
    wh: &WindowHandle<daruda_ui::Root>,
    state: &Entity<InputState>,
    cx: &mut TestAppContext,
    secondary: bool,
) {
    wh.update(cx, |_root, _window, cx| {
        state.update(cx, |_, cx_state| {
            cx_state.emit(InputEvent::PressEnter { secondary })
        });
    })
    .expect("settings window should still be open during the test");
}

/// The environment of the first editable row, as it stands in the live
/// settings — the thing a save actually moves.
fn stored_first_agent_env(
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
) -> Option<Vec<(String, String)>> {
    win.read_with(cx, |_w, cx| {
        crate::store::SettingsStore::global(cx)
            .user()
            .resolved_agents()[0]
            .env
            .clone()
    })
}

/// A newline in the Environment textarea is not a save. The multi-line widget
/// inserts the `\n` and *then* emits `PressEnter { secondary: false }`, so a
/// field wired to the single-line subscription rewrites `config.toml` on every
/// line break — and pops the inline banner on a line the user has not
/// finished typing. Only the secondary form commits.
#[gpui::test]
fn a_newline_in_the_environment_field_is_not_a_save(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let env_input = win.read_with(cx, |w, _| {
        w.agent_editable_row(0).unwrap().env_input.clone()
    });
    set_agent_row_input(&wh, &win, cx, 0, |r| r.env_input.clone(), "A=1");
    assert_eq!(
        stored_first_agent_env(&win, cx),
        None,
        "typing alone must not write"
    );

    press_enter(&wh, &env_input, cx, false);
    assert_eq!(
        stored_first_agent_env(&win, cx),
        None,
        "a plain Enter is a newline in a multi-line field, not a save"
    );

    press_enter(&wh, &env_input, cx, true);
    assert_eq!(
        stored_first_agent_env(&win, cx),
        Some(vec![("A".to_string(), "1".to_string())]),
        "Cmd/Ctrl+Enter still commits the field"
    );

    // Clicking away commits too — the same way every other Settings field
    // behaves, and the only way a user who never presses Enter gets saved.
    set_agent_row_input(&wh, &win, cx, 0, |r| r.env_input.clone(), "A=1\nB=2");
    blur(&wh, &env_input, cx);
    assert_eq!(
        stored_first_agent_env(&win, cx),
        Some(vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "2".to_string())
        ]),
        "losing focus commits the field"
    );
}

/// A line that is not `KEY=value` blocks the save and names itself, the same
/// way a bad agent id or a missing ssh host does.
#[gpui::test]
fn a_malformed_environment_line_blocks_the_save(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_agent_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.env_input.clone(),
        "RUST_LOG=debug\nCODEX_CONFIG",
    );
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        // Pinned whole: `contains("CODEX_CONFIG")` alone would also pass on
        // the wrong row ordinal, or on a message that named the *well-formed*
        // line instead.
        assert_eq!(err, s::settings::err_agent_catalog_env(1, "CODEX_CONFIG"));
        assert!(!err.contains("RUST_LOG"), "{err}");
    });
    // An empty key is the other malformation the field has to catch, and the
    // line it reports is the whole offending line, not the empty name.
    set_agent_row_input(&wh, &win, cx, 0, |r| r.env_input.clone(), "=oops");
    win.read_with(cx, |w, cx| {
        assert_eq!(
            w.validate(cx).unwrap_err(),
            s::settings::err_agent_catalog_env(1, "=oops")
        );
    });
}

/// The injection refused at the Settings entry point: the assembler quotes an
/// env *value* but emits the *name* bare, so a name carrying `;` would run as
/// its own command inside the remote `sh -c` script. Driven through the real
/// save path (`validate` → `collect_agent_catalog`), not the field helper, so
/// a check that only lived in the helper would not pass this.
#[gpui::test]
fn an_environment_name_that_could_break_out_of_the_shell_blocks_the_save(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    set_agent_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.env_input.clone(),
        "K; echo PWNED >&2 ; X=1",
    );
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("K; echo PWNED >&2 ; X"), "{err}");
    });

    // A name inside the POSIX charset still saves, and reaches the value.
    set_agent_row_input(&wh, &win, cx, 0, |r| r.env_input.clone(), "K_OK=1");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a usable name must validate");
        assert_eq!(
            cfg.resolved_agents()[0].env,
            Some(vec![("K_OK".to_string(), "1".to_string())])
        );
    });
}
