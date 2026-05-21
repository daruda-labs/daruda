use super::*;

#[test]
fn seed_default_has_four_ai_macros() {
    let state = seed_default();
    assert_eq!(state.schema_version, SCHEMA_VERSION);
    assert_eq!(state.tabs.len(), 1);
    assert_eq!(state.tabs[0].name, "AI");
    assert_eq!(state.tabs[0].widgets.len(), 4);
    let labels: Vec<&str> = state.tabs[0]
        .widgets
        .iter()
        .filter_map(|w| match w {
            MacroKey::Button(b) => Some(b.label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, vec!["Claude", "Codex", "Gemini", "Opencode"]);
    assert_eq!(state.active_tab_id.as_ref(), Some(&state.tabs[0].id));
}

#[test]
fn seed_default_buttons_are_builtin() {
    let state = seed_default();
    for w in &state.tabs[0].widgets {
        if let MacroKey::Button(b) = w {
            assert!(
                b.builtin,
                "seed button '{}' must have builtin=true",
                b.label
            );
        }
    }
}

#[test]
fn seed_default_sends_canonical_commands() {
    use super::seed::SEED_AI_ENTRIES;
    let state = seed_default();
    let sends: Vec<&str> = state.tabs[0]
        .widgets
        .iter()
        .filter_map(|w| match w {
            MacroKey::Button(b) => Some(b.send.as_str()),
            _ => None,
        })
        .collect();
    let expected: Vec<&str> = SEED_AI_ENTRIES.iter().map(|(_, s)| *s).collect();
    assert_eq!(sends, expected);
}

#[test]
fn auto_enter_default_true() {
    let json = r#"{
        "id": "w1",
        "label": "X",
        "send": "x"
    }"#;
    let btn: ButtonWidget = serde_json::from_str(json).unwrap();
    assert!(btn.auto_enter);
}

#[test]
fn display_default_text() {
    let json = r#"{
        "id": "w1",
        "label": "X",
        "send": "x"
    }"#;
    let btn: ButtonWidget = serde_json::from_str(json).unwrap();
    assert_eq!(btn.display, ButtonDisplay::Text);
}

#[test]
fn tab_height_null_means_auto_fit() {
    let json = r#"{
        "id": "t1",
        "name": "AI",
        "order": 0,
        "widgets": []
    }"#;
    let tab: PanelTab = serde_json::from_str(json).unwrap();
    assert!(tab.height.is_none());
}

#[test]
fn tab_layout_default_flex_wrap() {
    let json = r#"{
        "id": "t1",
        "name": "AI",
        "order": 0
    }"#;
    let tab: PanelTab = serde_json::from_str(json).unwrap();
    assert_eq!(tab.layout, TabLayout::FlexWrap);
    assert!(tab.widgets.is_empty());
}

#[test]
fn unknown_widget_deserializes_with_full_value() {
    let json = r#"{
        "schema_version": 1,
        "tabs": [{
            "id": "t1",
            "name": "Future",
            "order": 0,
            "widgets": [
                {"type": "button", "id": "w1", "label": "Real", "send": "ls"},
                {"type": "future_bar", "id": "w2", "value": 42, "max": 100, "color": "red"}
            ]
        }],
        "active_tab_id": "t1"
    }"#;
    let state: PanelsState = serde_json::from_str(json).unwrap();
    assert_eq!(state.tabs[0].widgets.len(), 2);
    assert!(matches!(&state.tabs[0].widgets[0], MacroKey::Button(_)));
    let v = match &state.tabs[0].widgets[1] {
        MacroKey::Unknown(v) => v,
        _ => panic!("expected Unknown variant"),
    };
    assert_eq!(v["type"], "future_bar");
    assert_eq!(v["value"], 42);
    assert_eq!(v["max"], 100);
    assert_eq!(v["color"], "red");
}

#[test]
fn unknown_widget_round_trips_preserving_fields() {
    let original = r#"{
        "schema_version": 1,
        "tabs": [{
            "id": "t1",
            "name": "Future",
            "order": 0,
            "widgets": [
                {"type": "future_gauge", "id": "w1", "min": 0, "max": 100, "current": 42}
            ]
        }],
        "active_tab_id": null
    }"#;
    let state: PanelsState = serde_json::from_str(original).unwrap();
    let serialized = serde_json::to_string(&state).unwrap();
    let reparsed: PanelsState = serde_json::from_str(&serialized).unwrap();
    let v = match &reparsed.tabs[0].widgets[0] {
        MacroKey::Unknown(v) => v,
        _ => panic!("expected Unknown after round-trip"),
    };
    assert_eq!(v["type"], "future_gauge");
    assert_eq!(v["min"], 0);
    assert_eq!(v["max"], 100);
    assert_eq!(v["current"], 42);
    assert_eq!(v["id"], "w1");
}

#[test]
fn button_widget_round_trips_via_widget_enum() {
    let widget = MacroKey::Button(ButtonWidget {
        id: "w1".to_string(),
        label: "Build".to_string(),
        send: "cargo build".to_string(),
        auto_enter: true,
        display: ButtonDisplay::Icon,
        icon: Some("🔨".to_string()),
        shortcut: Some("cmd-shift-b".to_string()),
        style: None,
        builtin: false,
    });
    let json = serde_json::to_string(&widget).unwrap();
    assert!(json.contains("\"type\":\"button\""));
    assert!(json.contains("Build"));
    assert!(json.contains("cargo build"));
    let parsed: MacroKey = serde_json::from_str(&json).unwrap();
    let btn = match parsed {
        MacroKey::Button(b) => b,
        _ => panic!("expected Button"),
    };
    assert_eq!(btn.label, "Build");
    assert_eq!(btn.icon.as_deref(), Some("🔨"));
    assert_eq!(btn.shortcut.as_deref(), Some("cmd-shift-b"));
}

#[test]
fn button_serializes_type_field_first_or_present() {
    let widget = MacroKey::Button(ButtonWidget {
        id: "w1".to_string(),
        label: "X".to_string(),
        send: "x".to_string(),
        auto_enter: true,
        display: ButtonDisplay::Text,
        icon: None,
        shortcut: None,
        style: None,
        builtin: false,
    });
    let json = serde_json::to_string(&widget).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["type"], "button");
    assert_eq!(v["id"], "w1");
}

#[test]
fn widget_id_accessor_for_button_and_unknown() {
    let btn = MacroKey::Button(ButtonWidget {
        id: "btn-id".to_string(),
        label: "X".to_string(),
        send: "x".to_string(),
        auto_enter: true,
        display: ButtonDisplay::Text,
        icon: None,
        shortcut: None,
        style: None,
        builtin: false,
    });
    assert_eq!(btn.id(), Some("btn-id"));

    let unknown = MacroKey::Unknown(serde_json::json!({"type": "bar", "id": "unk-id"}));
    assert_eq!(unknown.id(), Some("unk-id"));

    let unknown_no_id = MacroKey::Unknown(serde_json::json!({"type": "bar"}));
    assert_eq!(unknown_no_id.id(), None);
}

#[test]
fn ulid_generation_unique_across_calls() {
    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
        ids.insert(new_tab_id());
        ids.insert(new_widget_id());
    }
    assert_eq!(ids.len(), 200);
}

#[test]
fn save_then_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let original = seed_default();
    save_panels_in(dir.path(), &original).unwrap();

    let loaded = load_panels_in(dir.path()).expect("load after save");
    assert_eq!(loaded.tabs.len(), original.tabs.len());
    assert_eq!(loaded.tabs[0].name, original.tabs[0].name);
    assert_eq!(loaded.tabs[0].widgets.len(), original.tabs[0].widgets.len());
    assert_eq!(loaded.active_tab_id, original.active_tab_id);
}

#[test]
fn load_returns_none_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_panels_in(dir.path()).is_none());
}

#[test]
fn load_returns_none_when_file_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("panels.json"), b"{ not json").unwrap();
    assert!(load_panels_in(dir.path()).is_none());
}

#[test]
fn load_rejects_higher_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let bad = r#"{
        "schema_version": 999,
        "tabs": [],
        "active_tab_id": null
    }"#;
    std::fs::write(dir.path().join("panels.json"), bad).unwrap();
    assert!(load_panels_in(dir.path()).is_none());
}

#[test]
fn save_uses_atomic_write_no_partial_file_on_pretty_format() {
    let dir = tempfile::tempdir().unwrap();
    let state = seed_default();
    save_panels_in(dir.path(), &state).unwrap();
    let written = std::fs::read_to_string(dir.path().join("panels.json")).unwrap();
    assert!(written.starts_with('{'));
    assert!(written.trim_end().ends_with('}'));
    assert!(written.contains('\n'));
}

#[test]
fn save_creates_data_dir_if_missing() {
    let parent = tempfile::tempdir().unwrap();
    let nested = parent.path().join("does/not/exist/yet");
    save_panels_in(&nested, &seed_default()).unwrap();
    assert!(nested.join("panels.json").exists());
}

#[test]
fn unknown_widget_inside_seeded_tab_survives_round_trip_via_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mixed = r#"{
        "schema_version": 1,
        "tabs": [{
            "id": "t1",
            "name": "Mix",
            "order": 0,
            "widgets": [
                {"type": "button", "id": "w1", "label": "X", "send": "x"},
                {"type": "future_bar", "id": "w2", "v": 7}
            ]
        }],
        "active_tab_id": "t1"
    }"#;
    std::fs::write(dir.path().join("panels.json"), mixed).unwrap();
    let loaded = load_panels_in(dir.path()).unwrap();
    save_panels_in(dir.path(), &loaded).unwrap();
    let reloaded = load_panels_in(dir.path()).unwrap();
    let unknown = match &reloaded.tabs[0].widgets[1] {
        MacroKey::Unknown(v) => v,
        _ => panic!("future_bar should remain Unknown"),
    };
    assert_eq!(unknown["type"], "future_bar");
    assert_eq!(unknown["v"], 7);
}

#[test]
fn migrate_builtin_flags_sets_flag_on_known_sends() {
    let mut state = seed_default();
    // Simulate a panels.json written before the builtin field existed.
    for tab in &mut state.tabs {
        for w in &mut tab.widgets {
            if let MacroKey::Button(b) = w {
                b.builtin = false;
            }
        }
    }
    assert!(migrate_builtin_flags(&mut state), "should report changes");
    for tab in &state.tabs {
        for w in &tab.widgets {
            if let MacroKey::Button(b) = w {
                assert!(
                    b.builtin,
                    "seed button '{}' must be builtin after migrate",
                    b.label
                );
            }
        }
    }
}

#[test]
fn migrate_builtin_flags_no_op_when_already_set() {
    let mut state = seed_default(); // builtin already true on all seed buttons
    assert!(!migrate_builtin_flags(&mut state), "no changes expected");
}

#[test]
fn migrate_builtin_flags_skips_user_buttons() {
    let mut state = PanelsState::default();
    state.tabs.push(PanelTab {
        id: "t1".to_string(),
        name: "Custom".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![MacroKey::Button(ButtonWidget {
            id: "w1".to_string(),
            label: "MyTool".to_string(),
            send: "my-custom-tool".to_string(),
            auto_enter: true,
            display: ButtonDisplay::Text,
            icon: None,
            shortcut: None,
            style: None,
            builtin: false,
        })],
    });
    assert!(
        !migrate_builtin_flags(&mut state),
        "no seed buttons — nothing to migrate"
    );
    let btn = match &state.tabs[0].widgets[0] {
        MacroKey::Button(b) => b,
        _ => panic!("expected Button"),
    };
    assert!(!btn.builtin, "user button must stay builtin=false");
}

#[test]
fn migrate_does_not_promote_user_button_with_modified_send() {
    // builtin=false + send changed from canonical → label alone is not
    // enough evidence; the button is left untouched.
    let mut state = seed_default();
    for tab in &mut state.tabs {
        for w in &mut tab.widgets {
            if let MacroKey::Button(b) = w {
                b.builtin = false;
                b.send = "custom-edited".to_string();
            }
        }
    }
    assert!(!migrate_builtin_flags(&mut state), "nothing should change");
    for tab in &state.tabs {
        for w in &tab.widgets {
            if let MacroKey::Button(b) = w {
                assert!(!b.builtin, "button must remain builtin=false");
                assert_eq!(b.send, "custom-edited", "send must not be restored");
            }
        }
    }
}

#[test]
fn migrate_does_not_promote_user_button_with_matching_label() {
    // A user-created button whose label happens to equal a seed label
    // but whose send is different must not be promoted to builtin.
    let mut state = PanelsState::default();
    state.tabs.push(PanelTab {
        id: "t1".to_string(),
        name: "Custom".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![MacroKey::Button(ButtonWidget {
            id: "w1".to_string(),
            label: "Claude".to_string(),
            send: "my-own-claude-wrapper".to_string(),
            auto_enter: true,
            display: ButtonDisplay::Text,
            icon: None,
            shortcut: None,
            style: None,
            builtin: false,
        })],
    });
    assert!(
        !migrate_builtin_flags(&mut state),
        "label match alone must not trigger promotion"
    );
    let btn = match &state.tabs[0].widgets[0] {
        MacroKey::Button(b) => b,
        _ => panic!("expected Button"),
    };
    assert!(!btn.builtin, "must remain builtin=false");
    assert_eq!(
        btn.send, "my-own-claude-wrapper",
        "send must not be overwritten"
    );
}

#[test]
fn migrate_restores_send_when_builtin_true_but_send_was_changed() {
    // panels.json where builtin=true but send was edited directly in the file.
    let canonical = seed_default();
    let mut state = seed_default();
    for tab in &mut state.tabs {
        for w in &mut tab.widgets {
            if let MacroKey::Button(b) = w {
                b.send = "tampered".to_string(); // builtin stays true
            }
        }
    }
    assert!(migrate_builtin_flags(&mut state));
    let widgets = &state.tabs[0].widgets;
    let canonical_widgets = &canonical.tabs[0].widgets;
    for (w, cw) in widgets.iter().zip(canonical_widgets.iter()) {
        let (btn, canon) = match (w, cw) {
            (MacroKey::Button(b), MacroKey::Button(c)) => (b, c),
            _ => panic!("expected Button"),
        };
        assert_eq!(btn.send, canon.send, "send must be restored");
    }
}
