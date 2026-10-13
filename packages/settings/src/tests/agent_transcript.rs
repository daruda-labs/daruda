use super::*;

/// A hand-written fold matrix, an off-list step count and a partial visible
/// set. Fold and Filter load onto their editors, which can state every value
/// those keys hold; the off-list step count is the one the tail picker cannot
/// state, so it alone is offered as its own entry and kept verbatim.
fn hand_tuned_transcript_config() -> (Vec<String>, Vec<String>, daruda_config::Config) {
    let fold_mode = vec!["summary".to_string(), "last.thinking=expanded".to_string()];
    let display_filter = vec!["prose".to_string(), "tool_read".to_string()];
    let tuned = daruda_config::AgentDefinition {
        fold_mode: Some(fold_mode.clone()),
        tail_window: Some(12),
        display_filter: Some(display_filter.clone()),
        ..daruda_config::AgentDefinition::new(
            "hand-tuned-claude".to_string(),
            "Hand Tuned".to_string(),
            daruda_config::AgentLaunch::Raw(
                "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
            ),
        )
    };
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(tuned)],
        ..Default::default()
    };
    (fold_mode, display_filter, config)
}

/// The save path rebuilds the whole definition from the row, so a hand-tuned
/// value has to survive an edit to another field. Fold and Filter survive by
/// round-tripping through their editor's own value — the tokens are rewritten
/// in canonical form, which reads back as the same matrix and the same visible
/// set.
#[gpui::test]
fn transcript_defaults_survive_a_catalog_save_that_does_not_edit_them(cx: &mut TestAppContext) {
    let (fold_mode, display_filter, config) = hand_tuned_transcript_config();
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
        assert_eq!(saved.tail_window, Some(12), "kept verbatim, off the list");
        let saved_fold = saved.fold_mode.expect("the matrix is still written");
        assert_eq!(
            FoldMode::from_tokens(saved_fold.iter().map(String::as_str)),
            FoldMode::from_tokens(fold_mode.iter().map(String::as_str)),
            "the same matrix, however it is spelled"
        );
        let saved_filter = saved
            .display_filter
            .expect("the visible set is still written");
        assert_eq!(
            DisplayFilter::from_stored(&saved_filter),
            DisplayFilter::from_stored(&display_filter),
            "the same visible set, however it is spelled"
        );
    });
}

/// The off-list step count is stated by its own extra entry rather than
/// silently snapping to a neighbouring choice — the visible half of the
/// preservation the test above asserts on the save path.
#[gpui::test]
fn an_off_list_step_count_gets_its_own_entry(cx: &mut TestAppContext) {
    let (_fold_mode, _display_filter, config) = hand_tuned_transcript_config();
    let (wh, win) = build_window_with_config(cx, config);
    let field = |r: &AgentCatalogRow| r.tail_window_select.clone();
    let selected = win.read_with(cx, |w, cx| {
        field(w.agent_editable_row(0).unwrap())
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
    });
    assert_eq!(
        selected.as_deref(),
        Some(sections::agent_transcript::CUSTOM),
        "the stored value must stay selected, not fall back to Default"
    );
    assert!(
        agent_row_select_offers(&wh, &win, cx, 0, field, sections::agent_transcript::CUSTOM),
        "the entry standing for the stored value has to be in the list"
    );
}

#[gpui::test]
fn range_editor_edits_and_resets_without_losing_the_other_custom_size(cx: &mut TestAppContext) {
    let (_, _, config) = hand_tuned_transcript_config();
    let (wh, win) = build_window_with_config(cx, config);
    let original = win.read_with(cx, |s, cx| {
        s.agent_editable_row(0).unwrap().range_values(cx)
    });
    wh.update(cx, |_, window, cx| {
        win.update(cx, |s, cx| {
            s.set_agent_row_range_size(0, RangeLevel::Calls, 3, window, cx)
        });
    })
    .unwrap();
    win.read_with(cx, |s, cx| {
        assert_eq!(
            s.agent_editable_row(0).unwrap().range_values(cx),
            [original[0], 3]
        )
    });
    wh.update(cx, |_, window, cx| {
        win.update(cx, |s, cx| s.reset_agent_row_range(0, window, cx));
    })
    .unwrap();
    win.read_with(cx, |s, cx| {
        let row = s.agent_editable_row(0).unwrap();
        assert_eq!(row.tail_window(cx), None);
        assert_eq!(row.tail_window_calls(cx), None);
    });
}

/// Fold and Filter need no such entry: their editors state every value those
/// keys can hold, so a hand-tuned one loads onto the editor itself.
#[gpui::test]
fn a_hand_tuned_fold_and_filter_load_onto_their_editors(cx: &mut TestAppContext) {
    let (fold_mode, display_filter, config) = hand_tuned_transcript_config();
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, _cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(
            row.fold_mode_value(),
            FoldMode::from_tokens(fold_mode.iter().map(String::as_str))
        );
        assert_eq!(
            row.fold_mode_value().preset(),
            None,
            "the fixture is a matrix no preset states"
        );
        assert_eq!(
            row.display_filter_value(),
            DisplayFilter::from_stored(&display_filter)
        );
        assert!(!row.display_filter_value().shows_everything());
    });
}

/// Editing an axis is the user replacing the value, so the loaded one stops
/// being written — on every axis, including the one whose stored value was
/// off-list.
#[gpui::test]
fn editing_a_transcript_axis_replaces_the_loaded_value(cx: &mut TestAppContext) {
    let (_fold_mode, display_filter, config) = hand_tuned_transcript_config();
    let loaded = DisplayFilter::from_stored(&display_filter);
    assert!(
        !loaded.contains(FilterFacet::Thinking),
        "the fixture starts with thinking hidden, so the toggle below shows it"
    );
    let (wh, win) = build_window_with_config(cx, config);
    confirm_agent_row_select(&wh, &win, cx, 0, |r| r.tail_window_select.clone(), "3");
    // Showing thinking leaves the set still narrowed, so the axis keeps
    // stating a value rather than falling back to the built-in.
    win.update(cx, |w, cx| {
        w.select_agent_row_fold_preset(0, Some(FoldPreset::Expanded), cx);
        w.toggle_agent_row_filter_facet(0, FilterFacet::Thinking, cx);
    });

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("the edited row must validate");
        let saved = cfg.agents[0].resolve().expect("a custom entry resolves");
        assert_eq!(saved.fold_mode, Some(vec!["expanded".to_string()]));
        assert_eq!(saved.tail_window, Some(3));
        let filter =
            DisplayFilter::from_stored(&saved.display_filter.expect("an edited filter is written"));
        assert!(
            filter.contains(FilterFacet::Thinking),
            "the toggle flipped the loaded value rather than restating it"
        );
        assert_ne!(filter, loaded, "and it is no longer what was loaded");
        assert!(!filter.shows_everything(), "still a narrowed set");
    });
}

/// The built-in value writes no key: an absent key and a key stating that value
/// resolve alike, so the row keeps the file clean rather than restating it. The
/// editors reach it through their reset footer, which drops the key outright.
#[gpui::test]
fn handing_an_axis_back_writes_no_key(cx: &mut TestAppContext) {
    let (_fold_mode, _display_filter, config) = hand_tuned_transcript_config();
    let (wh, win) = build_window_with_config(cx, config);
    confirm_agent_row_select(
        &wh,
        &win,
        cx,
        0,
        |r| r.tail_window_select.clone(),
        &daruda_config::TAIL_WINDOW_DEFAULT.to_string(),
    );
    win.update(cx, |w, cx| {
        w.reset_agent_row_fold_mode(0, cx);
        w.reset_agent_row_display_filter(0, cx);
    });

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("the edited row must validate");
        let saved = cfg.agents[0].resolve().expect("a custom entry resolves");
        assert_eq!(saved.fold_mode, None);
        assert_eq!(saved.tail_window, None);
        assert_eq!(saved.display_filter, None);
    });
}

/// The round trip the rule above depends on: a row that wrote nothing reopens
/// on the built-in value, not on a blank or a stale selection.
#[gpui::test]
fn a_row_that_wrote_no_key_reopens_on_the_built_in_value(cx: &mut TestAppContext) {
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(
            daruda_config::AgentDefinition::claude_default(),
        )],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let row = w.agent_editable_row(0).expect("a custom row");
        assert_eq!(
            row.tail_window_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string()),
            Some(daruda_config::TAIL_WINDOW_DEFAULT.to_string())
        );
        assert_eq!(row.fold_mode_value(), FoldMode::default());
        assert_eq!(row.display_filter_value(), DisplayFilter::default());
        // And reading them back writes nothing, so an untouched row cannot
        // grow three keys just by being rendered.
        assert_eq!(row.fold_mode(), None);
        assert_eq!(row.tail_window(cx), None);
        assert_eq!(row.display_filter(), None);
    });
}

/// Once the tail picker replaces a preserved value, the still-open Settings
/// window has to forget the old configured-elsewhere entry. Otherwise selecting
/// it again would write back a value that is no longer in the live config.
#[gpui::test]
fn replacing_a_custom_step_count_rebases_the_row(cx: &mut TestAppContext) {
    let (_fold_mode, _display_filter, config) = hand_tuned_transcript_config();
    let (wh, win) = build_window_with_config(cx, config);
    assert!(
        agent_row_select_offers(
            &wh,
            &win,
            cx,
            0,
            |r| r.tail_window_select.clone(),
            sections::agent_transcript::CUSTOM,
        ),
        "the fixture starts on a preserved step count"
    );
    win.read_with(cx, |w, _cx| {
        assert_eq!(
            w.agent_editable_row(0).unwrap().tail_window_loaded,
            Some(12)
        );
    });

    confirm_agent_row_select(&wh, &win, cx, 0, |r| r.tail_window_select.clone(), "3");

    win.read_with(cx, |w, cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.tail_window(cx), Some(3));
        assert_eq!(
            row.tail_window_loaded, None,
            "the old count must not remain hidden behind the picker"
        );
    });
    win.read_with(cx, |_w, cx| {
        let saved = crate::store::SettingsStore::global(cx).user().agents[0]
            .resolve()
            .expect("the row remains runnable");
        assert_eq!(saved.tail_window, Some(3));
    });
    assert!(
        !agent_row_select_offers(
            &wh,
            &win,
            cx,
            0,
            |r| r.tail_window_select.clone(),
            sections::agent_transcript::CUSTOM,
        ),
        "the old configured-elsewhere choice is no longer present"
    );
}

/// The drift the shared editor exists to prevent: a rule edit that lands back
/// on a preset still has to leave the matrix recallable, exactly as the chat
/// pane's `set_fold_mode` does. Settings reaches this through `on_change`,
/// which no preset-strip test covers.
#[gpui::test]
fn a_rule_edit_back_onto_a_preset_keeps_the_matrix_recallable(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    let matrix = FoldPreset::Summary.mode().with_rule(
        TurnPosition::Last,
        FoldBlock::Diff,
        BlockRule::Collapsed,
    );
    win.update(cx, |w, cx| {
        w.select_agent_row_fold_preset(0, Some(FoldPreset::Summary), cx);
        w.set_agent_row_fold_mode(0, Some(matrix), cx);
        // Straight back to the preset — the matrix is now only reachable
        // through the `Custom` segment.
        w.set_agent_row_fold_mode(0, Some(FoldPreset::Summary.mode()), cx);
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(
            row.fold_editor.custom(),
            Some(matrix),
            "the edit must survive landing back on a preset"
        );
        assert_eq!(row.fold_mode_value(), FoldPreset::Summary.mode());

        w.select_agent_row_fold_preset(0, None, cx);
        assert_eq!(
            w.agent_editable_row(0).unwrap().fold_mode_value(),
            matrix,
            "the Custom segment returns to it"
        );
    });
}

/// Editing an axis onto the built-in writes no key: an absent key and a key
/// stating that value resolve alike. The departure is still recallable.
#[gpui::test]
fn editing_onto_the_built_in_drops_the_key(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    let matrix = FoldPreset::Auto.mode().with_rule(
        TurnPosition::Past,
        FoldBlock::Thinking,
        BlockRule::Collapsed,
    );
    win.update(cx, |w, cx| {
        w.set_agent_row_fold_mode(0, Some(matrix), cx);
        assert!(w.agent_editable_row(0).unwrap().fold_mode().is_some());

        w.set_agent_row_fold_mode(0, Some(FoldMode::default()), cx);
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.fold_mode(), None, "the built-in states nothing");
        assert_eq!(row.fold_editor.custom(), Some(matrix), "still recallable");

        w.toggle_agent_row_filter_facet(0, FilterFacet::Thinking, cx);
        assert!(w.agent_editable_row(0).unwrap().display_filter().is_some());
        w.toggle_agent_row_filter_facet(0, FilterFacet::Thinking, cx);
        assert_eq!(
            w.agent_editable_row(0).unwrap().display_filter(),
            None,
            "back to the unfiltered set, which is the built-in one"
        );
    });
}

/// The editor's disclosures are view switches, not values: opening them must
/// not make the row state a key it did not state before.
#[gpui::test]
fn opening_fold_editor_disclosures_writes_nothing(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.toggle_agent_row_fold_history_rules(0, cx);
        w.toggle_agent_row_fold_tools(0, TurnPosition::Past, cx);
        let row = w.agent_editable_row(0).unwrap();
        assert!(row.fold_editor.history_rules_open());
        assert!(row.fold_editor.tools_open(TurnPosition::Past));
        assert_eq!(row.fold_mode(), None, "a view switch is not a value");
    });
}

/// An axis nobody edited is written back exactly as it was read, so a save that
/// edits some other field cannot rewrite it — including a token this build does
/// not know, which the parsers drop on the way in.
#[gpui::test]
fn an_untouched_axis_keeps_tokens_this_build_cannot_state(cx: &mut TestAppContext) {
    let stored_fold = vec!["summary".to_string(), "last.notebook=collapsed".to_string()];
    let stored_filter = vec!["prose".to_string(), "brand_new_facet".to_string()];
    let tuned = daruda_config::AgentDefinition {
        fold_mode: Some(stored_fold.clone()),
        display_filter: Some(stored_filter.clone()),
        ..daruda_config::AgentDefinition::new(
            "forward".to_string(),
            "Forward".to_string(),
            daruda_config::AgentLaunch::Raw("npx -y some-acp".to_string()),
        )
    };
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(tuned)],
        ..Default::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, _cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.fold_mode(), Some(stored_fold.clone()));
        assert_eq!(row.display_filter(), Some(stored_filter.clone()));
    });

    // Editing the axis moves the value off what the tokens spell, so from then
    // on the row states the value and the unknown token is gone by choice.
    win.update(cx, |w, cx| {
        w.select_agent_row_fold_preset(0, Some(FoldPreset::Expanded), cx);
        let written = w.agent_editable_row(0).unwrap().fold_mode();
        assert_eq!(written, Some(vec!["expanded".to_string()]));
        assert_eq!(
            w.agent_editable_row(0).unwrap().display_filter(),
            Some(stored_filter),
            "the axis nobody touched is still untouched"
        );
    });
}

/// A row that states nothing writes nothing, so the agent keeps falling
/// through to the built-in values.
#[gpui::test]
fn a_fresh_row_leaves_every_transcript_axis_unset(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.fold_mode(), None);
        assert_eq!(row.tail_window(cx), None);
        assert_eq!(row.display_filter(), None);
        let cfg = w.validate(cx).expect("the default catalog must validate");
        let saved = cfg.agents[0].resolve().expect("the default entry resolves");
        assert_eq!(saved.fold_mode, None);
        assert_eq!(saved.tail_window, None);
        assert_eq!(saved.display_filter, None);
    });
}

/// A preset states none of the transcript axes, so picking a value on one is
/// saved as an override of that axis alone.
#[gpui::test]
fn a_transcript_pick_on_a_preset_row_is_saved_as_an_override(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    enable_agent_preset(&wh, &win, cx, "gemini");
    win.update(cx, |w, cx| {
        w.select_agent_row_fold_preset(1, Some(FoldPreset::Summary), cx)
    });

    // A preset row carries its overrides through `PresetOverrides`, the one
    // persistence path a `Custom` row never exercises.
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("agent catalog must validate");
        assert_eq!(
            cfg.agents[1],
            daruda_config::AgentEntry::preset_with(
                "gemini".to_string(),
                daruda_config::PresetOverrides {
                    fold_mode: Some(vec!["summary".to_string()]),
                    ..daruda_config::PresetOverrides::default()
                }
            ),
            "the picked axis is an override; the untouched ones stay unset"
        );
    });
}

/// A step count the picker *can* state loads onto the picker itself, not onto
/// the configured-elsewhere entry — otherwise every row would read as custom.
#[gpui::test]
fn a_transcript_value_the_controls_state_loads_onto_them(cx: &mut TestAppContext) {
    let stated = daruda_config::AgentDefinition {
        fold_mode: Some(vec!["summary".to_string()]),
        tail_window: Some(5),
        ..daruda_config::AgentDefinition::new(
            "stated".to_string(),
            "Stated".to_string(),
            daruda_config::AgentLaunch::Raw("npx -y some-acp".to_string()),
        )
    };
    let config = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::custom(stated)],
        ..Default::default()
    };
    let (wh, win) = build_window_with_config(cx, config);
    win.read_with(cx, |w, cx| {
        let row = w.agent_editable_row(0).unwrap();
        assert_eq!(row.fold_mode(), Some(vec!["summary".to_string()]));
        assert_eq!(row.tail_window(cx), Some(5));
        assert_eq!(row.display_filter(), None);
    });
    // No stored value needed preserving, so no extra entry was added.
    assert!(
        !agent_row_select_offers(
            &wh,
            &win,
            cx,
            0,
            |r: &AgentCatalogRow| r.tail_window_select.clone(),
            sections::agent_transcript::CUSTOM,
        ),
        "a row with nothing to preserve must not offer the custom entry"
    );
}
