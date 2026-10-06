use super::*;

#[test]
fn load_or_seed_creates_file_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(daruda_store::panels::load_panels_in(dir.path()).is_none());

    let state = load_or_seed_panels(dir.path());
    assert_eq!(state.tabs.len(), 1);
    assert_eq!(state.tabs[0].name, "AI");

    // Same call after the seed exists must read it back rather
    // than reseed (preserving any edits the user made).
    let again = load_or_seed_panels(dir.path());
    assert_eq!(again.tabs[0].id, state.tabs[0].id);
}

fn make_tab(id: &str, name: &str, order: u32) -> PanelTab {
    PanelTab {
        id: id.to_string(),
        name: name.to_string(),
        order,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: Vec::new(),
    }
}

fn make_button(id: &str, label: &str, send: &str) -> ButtonWidget {
    ButtonWidget {
        id: id.to_string(),
        label: label.to_string(),
        send: send.to_string(),
        auto_enter: true,
        display: daruda_store::panels::ButtonDisplay::Text,
        icon: None,
        shortcut: None,
        style: None,
        builtin: false,
    }
}

#[test]
fn update_button_in_place_replaces_content_and_rejects_non_targets() {
    let mut tabs = vec![PanelTab {
        id: "t1".to_string(),
        name: "T".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![MacroKey::Button(make_button("w1", "Old", "old"))],
    }];
    let new_btn = ButtonWidget {
        id: "ignored".to_string(), // overwritten with existing id
        label: "New".to_string(),
        send: "new".to_string(),
        auto_enter: false,
        display: daruda_store::panels::ButtonDisplay::Icon,
        icon: Some("✨".to_string()),
        shortcut: Some("cmd-shift-x".to_string()),
        style: None,
        builtin: false,
    };
    assert!(update_button_in_place(&mut tabs, "t1", "w1", new_btn));
    let updated = match &tabs[0].widgets[0] {
        MacroKey::Button(b) => b,
        _ => panic!("button"),
    };
    assert_eq!(updated.id, "w1"); // id preserved
    assert_eq!(updated.label, "New");
    assert_eq!(updated.send, "new");
    assert!(!updated.auto_enter);
    assert_eq!(updated.icon.as_deref(), Some("✨"));

    assert!(!update_button_in_place(
        &mut tabs,
        "tnope",
        "w1",
        make_button("w1", "X2", "y2")
    ));

    assert!(!update_button_in_place(
        &mut tabs,
        "t1",
        "wnope",
        make_button("wnope", "X2", "y2")
    ));

    let mut unknown_tabs = vec![PanelTab {
        id: "t1".to_string(),
        name: "T".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![MacroKey::Unknown(
            serde_json::json!({"type": "future_bar", "id": "w1"}),
        )],
    }];
    // Existing widget id is "w1" but it's Unknown — update should
    // refuse to clobber the unknown JSON with a Button.
    assert!(!update_button_in_place(
        &mut unknown_tabs,
        "t1",
        "w1",
        make_button("w1", "X", "y")
    ));
}

#[test]
fn remove_widget_in_place_removes_match() {
    let mut tabs = vec![PanelTab {
        id: "t1".to_string(),
        name: "T".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![
            MacroKey::Button(make_button("w1", "A", "a")),
            MacroKey::Button(make_button("w2", "B", "b")),
        ],
    }];
    assert!(remove_widget_in_place(&mut tabs, "t1", "w1"));
    assert_eq!(tabs[0].widgets.len(), 1);
    assert_eq!(tabs[0].widgets[0].id(), Some("w2"));
}

#[test]
fn remove_widget_in_place_no_op_missing() {
    let mut tabs = vec![PanelTab {
        id: "t1".to_string(),
        name: "T".to_string(),
        order: 0,
        height: None,
        layout: TabLayout::FlexWrap,
        widgets: vec![MacroKey::Button(make_button("w1", "A", "a"))],
    }];
    assert!(!remove_widget_in_place(&mut tabs, "t1", "wnope"));
    assert!(!remove_widget_in_place(&mut tabs, "tnope", "w1"));
    assert_eq!(tabs[0].widgets.len(), 1);
}

#[test]
fn find_widget_by_shortcut_finds_buttons_and_skips_unknown_widgets() {
    let panels = PanelsState {
        schema_version: daruda_store::panels::SCHEMA_VERSION,
        active_tab_id: None,
        tabs: vec![PanelTab {
            id: "t1".to_string(),
            name: "AI".to_string(),
            order: 0,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets: vec![
                MacroKey::Button(ButtonWidget {
                    id: "w1".to_string(),
                    label: "A".to_string(),
                    send: "a".to_string(),
                    auto_enter: true,
                    display: daruda_store::panels::ButtonDisplay::Text,
                    icon: None,
                    shortcut: Some("cmd-shift-1".to_string()),
                    style: None,
                    builtin: false,
                }),
                MacroKey::Button(ButtonWidget {
                    id: "w2".to_string(),
                    label: "B".to_string(),
                    send: "b".to_string(),
                    auto_enter: true,
                    display: daruda_store::panels::ButtonDisplay::Text,
                    icon: None,
                    shortcut: Some("cmd-shift-2".to_string()),
                    style: None,
                    builtin: false,
                }),
            ],
        }],
    };
    let hit = find_widget_by_shortcut(&panels, "cmd-shift-2").unwrap();
    assert_eq!(hit, ("t1".to_string(), "w2".to_string()));
    assert_eq!(find_widget_by_shortcut(&panels, "cmd-shift-9"), None);

    let panels = PanelsState {
        schema_version: daruda_store::panels::SCHEMA_VERSION,
        active_tab_id: None,
        tabs: vec![PanelTab {
            id: "t1".to_string(),
            name: "Mix".to_string(),
            order: 0,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets: vec![
                MacroKey::Unknown(serde_json::json!({
                    "type": "future",
                    "shortcut": "cmd-shift-1"
                })),
                MacroKey::Button(make_button("w2", "Real", "real")),
            ],
        }],
    };
    // Unknown widgets are skipped even if their JSON has a
    // "shortcut" field.
    assert_eq!(find_widget_by_shortcut(&panels, "cmd-shift-1"), None);
}

#[test]
fn is_valid_shortcut_accepts_simple_and_rejects_blank() {
    assert!(is_valid_shortcut("cmd-shift-1"));
    assert!(is_valid_shortcut("ctrl-a"));
    assert!(is_valid_shortcut("f5"));
    assert!(!is_valid_shortcut(""));
    assert!(!is_valid_shortcut("   "));
}

#[test]
fn reorder_in_place_covers_moves_noops_and_order_field_sorting() {
    let mut tabs = vec![
        make_tab("a", "A", 0),
        make_tab("b", "B", 1),
        make_tab("c", "C", 2),
    ];
    // Drag A onto C: A claims C's slot, B and C shift left. Symmetric
    // with the right-to-left case below.
    assert!(reorder_in_place(&mut tabs, "a", "c"));
    let order_after: Vec<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(order_after, vec!["b", "c", "a"]);
    assert_eq!(tabs[0].order, 0);
    assert_eq!(tabs[1].order, 1);
    assert_eq!(tabs[2].order, 2);

    let mut tabs = vec![
        make_tab("a", "A", 0),
        make_tab("b", "B", 1),
        make_tab("c", "C", 2),
    ];
    // Drag A onto B: A and B swap. Guards against an off-by-one that
    // would leave the list unchanged on adjacent rightward drops.
    assert!(reorder_in_place(&mut tabs, "a", "b"));
    let order_after: Vec<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(order_after, vec!["b", "a", "c"]);

    let mut tabs = vec![
        make_tab("a", "A", 0),
        make_tab("b", "B", 1),
        make_tab("c", "C", 2),
    ];
    // Drag C onto A: C lands at A's position, A and B shift right.
    assert!(reorder_in_place(&mut tabs, "c", "a"));
    let order_after: Vec<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(order_after, vec!["c", "a", "b"]);
    assert_eq!(tabs[0].order, 0);
    assert_eq!(tabs[1].order, 1);
    assert_eq!(tabs[2].order, 2);

    let mut tabs = vec![make_tab("a", "A", 0), make_tab("b", "B", 1)];
    assert!(!reorder_in_place(&mut tabs, "a", "a"));

    let mut tabs = vec![make_tab("a", "A", 0), make_tab("b", "B", 1)];
    assert!(!reorder_in_place(&mut tabs, "nope", "a"));
    assert!(!reorder_in_place(&mut tabs, "a", "nope"));
    // List unchanged.
    assert_eq!(tabs[0].id, "a");
    assert_eq!(tabs[1].id, "b");

    // Vec is in insertion order [c, a, b] but order field says [a, b, c].
    // After reorder a→c, the result should respect the order-field
    // semantics, not the Vec insertion order.
    let mut tabs = vec![
        make_tab("c", "C", 2),
        make_tab("a", "A", 0),
        make_tab("b", "B", 1),
    ];
    assert!(reorder_in_place(&mut tabs, "a", "c"));
    // Sorted by order: [a(0), b(1), c(2)]; after a→c: [b, c, a].
    let order_after: Vec<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(order_after, vec!["b", "c", "a"]);
}

#[test]
fn remove_tab_returns_removed_or_none_for_missing_id() {
    let mut tabs = vec![make_tab("t1", "A", 0), make_tab("t2", "B", 1)];
    let removed = remove_tab(&mut tabs, "t1").unwrap();
    assert_eq!(removed.id, "t1");
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].id, "t2");

    let mut tabs = vec![make_tab("t1", "A", 0)];
    assert!(remove_tab(&mut tabs, "t999").is_none());
    assert_eq!(tabs.len(), 1);
}

#[test]
fn pick_fallback_active_prefers_next_then_previous_then_none() {
    let tabs = vec![
        make_tab("a", "A", 0),
        make_tab("b", "B", 5),
        make_tab("c", "C", 10),
    ];
    // Delete order 5 → next higher is 10 → "c".
    assert_eq!(pick_fallback_active(&tabs, 5), Some("c".to_string()));

    let tabs = vec![make_tab("a", "A", 0), make_tab("b", "B", 5)];
    // Removed order 10 → no higher, fall back to lower → "b" (order 5).
    assert_eq!(pick_fallback_active(&tabs, 10), Some("b".to_string()));

    let tabs: Vec<PanelTab> = Vec::new();
    assert_eq!(pick_fallback_active(&tabs, 0), None);
}

#[test]
fn rename_in_place_changes_trims_rejects_empty_and_handles_noops() {
    let mut tabs = vec![make_tab("t1", "Old", 0)];
    assert!(rename_in_place(&mut tabs, "t1", "New"));
    assert_eq!(tabs[0].name, "New");

    let mut tabs = vec![make_tab("t1", "Old", 0)];
    assert!(rename_in_place(&mut tabs, "t1", "  Build  "));
    assert_eq!(tabs[0].name, "Build");

    let mut tabs = vec![make_tab("t1", "Old", 0)];
    assert!(!rename_in_place(&mut tabs, "t1", ""));
    assert!(!rename_in_place(&mut tabs, "t1", "   "));
    assert_eq!(tabs[0].name, "Old");

    let mut tabs = vec![make_tab("t1", "AI", 0)];
    // Trim normalizes — "AI " trimmed equals "AI", so no-op.
    assert!(!rename_in_place(&mut tabs, "t1", "AI"));
    assert!(!rename_in_place(&mut tabs, "t1", "  AI  "));

    let mut tabs = vec![make_tab("t1", "AI", 0)];
    assert!(!rename_in_place(&mut tabs, "t999", "X"));
    assert_eq!(tabs[0].name, "AI");
}

#[test]
fn build_new_tab_validates_name_order_identity_and_defaults() {
    assert!(build_new_tab("", &[]).is_none());
    assert!(build_new_tab("   ", &[]).is_none());
    assert!(build_new_tab("\t\n", &[]).is_none());

    let tab = build_new_tab("  AI  ", &[]).unwrap();
    assert_eq!(tab.name, "AI");

    let tab = build_new_tab("First", &[]).unwrap();
    assert_eq!(tab.order, 0);

    let existing = vec![
        PanelTab {
            id: "a".to_string(),
            name: "A".to_string(),
            order: 0,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets: Vec::new(),
        },
        PanelTab {
            id: "b".to_string(),
            name: "B".to_string(),
            order: 5,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets: Vec::new(),
        },
        PanelTab {
            id: "c".to_string(),
            name: "C".to_string(),
            order: 2,
            height: None,
            layout: TabLayout::FlexWrap,
            widgets: Vec::new(),
        },
    ];
    let new_tab = build_new_tab("D", &existing).unwrap();
    assert_eq!(new_tab.order, 6);

    let a = build_new_tab("X", &[]).unwrap();
    let b = build_new_tab("X", &[]).unwrap();
    assert_ne!(a.id, b.id);

    let tab = build_new_tab("Empty", &[]).unwrap();
    assert!(tab.widgets.is_empty());
    assert!(tab.height.is_none());
    assert_eq!(tab.layout, TabLayout::FlexWrap);
}

#[test]
fn panels_state_eq_detects_changes() {
    let mut a = PanelsState::default();
    let mut b = PanelsState::default();
    assert_eq!(a, b);

    a.tabs.push(daruda_store::panels::PanelTab {
        id: "t1".to_string(),
        name: "X".to_string(),
        order: 0,
        height: None,
        layout: daruda_store::panels::TabLayout::FlexWrap,
        widgets: Vec::new(),
    });
    assert_ne!(a, b);

    b.tabs.push(daruda_store::panels::PanelTab {
        id: "t1".to_string(),
        name: "X".to_string(),
        order: 0,
        height: None,
        layout: daruda_store::panels::TabLayout::FlexWrap,
        widgets: Vec::new(),
    });
    assert_eq!(a, b);
}

#[test]
fn load_or_seed_returns_existing_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let mut custom = daruda_store::panels::PanelsState::default();
    let tab_id = daruda_store::panels::new_tab_id();
    custom.tabs.push(daruda_store::panels::PanelTab {
        id: tab_id.clone(),
        name: "Custom".to_string(),
        order: 0,
        height: None,
        layout: daruda_store::panels::TabLayout::FlexWrap,
        widgets: Vec::new(),
    });
    custom.active_tab_id = Some(tab_id.clone());
    daruda_store::panels::save_panels_in(dir.path(), &custom).unwrap();

    let loaded = load_or_seed_panels(dir.path());
    assert_eq!(loaded.tabs.len(), 1);
    assert_eq!(loaded.tabs[0].name, "Custom");
    assert_eq!(loaded.tabs[0].id, tab_id);
}
