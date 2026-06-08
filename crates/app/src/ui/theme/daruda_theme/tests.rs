use super::*;

#[test]
fn default_clones_compile_time_palette() {
    // Every field must equal the underlying `pub const` so a
    // fresh `DarudaTheme::default()` is observationally identical
    // to reading `palette::FOO` directly — the invariant Phase 3-B
    // relies on when it switches call sites over.
    let t = DarudaTheme::default();
    assert_eq!(t.title_bar_bg, palette::BG_BASE);
    assert_eq!(t.tab_inactive_bg, palette::BG_PANEL);
    assert_eq!(t.tab_inactive_hover_bg, palette::BG_HOVER);
    assert_eq!(t.tab_inactive_text, palette::TEXT_MUTE);
    assert_eq!(t.status_bar_bg, palette::BG_BASE);
    assert_eq!(t.status_bar_border, palette::BORDER);
    assert_eq!(t.status_bar_project_dot, palette::STATUS_BAR_PROJECT_DOT);
    assert_eq!(t.status_bar_detached_bg, palette::STATUS_BAR_DETACHED_BG);
    assert_eq!(
        t.status_bar_detached_text,
        palette::STATUS_BAR_DETACHED_TEXT
    );
    assert_eq!(
        t.lane_drop_target_rejected_bg,
        palette::LANE_DROP_TARGET_REJECTED_BG,
    );
    assert_eq!(t.dock_bg, palette::BG_PANEL);
    assert_eq!(t.dock_border, palette::BORDER);
    assert_eq!(t.dock_header_text, palette::TEXT_MUTE);
    assert_eq!(t.dock_placeholder_text, palette::TEXT_SUBTLE);
    assert_eq!(t.button_widget_bg, palette::BG_HOVER);
    assert_eq!(t.button_widget_bg_hover, palette::BG_ACTIVE);
    assert_eq!(t.button_widget_text, palette::TEXT_BODY);
    assert_eq!(t.panel_tab_drop_target_bg, palette::BG_ACTIVE);
    assert_eq!(t.muted_text, palette::TEXT_MUTE);
    assert_eq!(t.faint_text, palette::TEXT_SUBTLE);
    assert_eq!(t.disabled_item_bg, palette::BG_HOVER);
    assert_eq!(t.disabled_item_text, palette::TEXT_SUBTLE);
    assert_eq!(t.dock_icon_inactive, palette::TEXT_MUTE);
    assert_eq!(t.dock_icon_hover, palette::TEXT_BODY);
    assert_eq!(t.dock_icon_active_bg, palette::BG_HOVER);
    assert_eq!(t.dock_view_tab_inactive, palette::TEXT_MUTE);
    assert_eq!(t.dock_view_tab_hover_bg, palette::BG_HOVER);
    assert_eq!(t.lane_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.lane_drop_target_bg, palette::LANE_DROP_TARGET_BG);
    assert_eq!(t.modal_panel_bg, palette::BG_RAISED);
    assert_eq!(t.modal_panel_border, palette::BORDER);
    assert_eq!(t.modal_input_bg, palette::BG_BASE);
    assert_eq!(t.modal_input_border, palette::BORDER);
    assert_eq!(t.modal_secondary_text, palette::TEXT_BODY);
    assert_eq!(t.textarea_scrollbar_thumb, palette::SCROLLBAR_THUMB);
    assert_eq!(t.banner_error_bg, palette::BANNER_ERROR_BG);
    assert_eq!(t.banner_error_text, palette::BANNER_ERROR_TEXT);
    assert_eq!(t.banner_warning_bg, palette::BANNER_WARNING_BG);
    assert_eq!(t.banner_warning_text, palette::BANNER_WARNING_TEXT);
    assert_eq!(t.banner_info_bg, palette::BANNER_INFO_BG);
    assert_eq!(t.banner_info_text, palette::BANNER_INFO_TEXT);
    assert_eq!(t.banner_success_bg, palette::BANNER_SUCCESS_BG);
    assert_eq!(t.banner_success_text, palette::BANNER_SUCCESS_TEXT);
    assert_eq!(t.settings_scrollbar_thumb, palette::SCROLLBAR_THUMB);
    assert_eq!(
        t.settings_scrollbar_thumb_hover,
        palette::SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.right_panel_scrollbar_thumb, palette::SCROLLBAR_THUMB);
    assert_eq!(
        t.right_panel_scrollbar_thumb_hover,
        palette::SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.settings_sidebar_bg, palette::SETTINGS_SIDEBAR_BG);
    assert_eq!(t.palette_bg, palette::BG_RAISED);
    assert_eq!(t.palette_border, palette::BORDER);
    assert_eq!(t.palette_input_border, palette::BORDER);
    assert_eq!(t.palette_focused_bg, palette::BG_ACTIVE);
    assert_eq!(t.palette_entry_text, palette::TEXT_BODY);
    assert_eq!(t.palette_shortcut_text, palette::TEXT_SUBTLE);
    assert_eq!(t.palette_empty_text, palette::TEXT_SUBTLE);
    assert_eq!(t.git_file_row_selected_bg, palette::BG_HOVER);
    assert_eq!(t.git_file_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.git_diff_panel_bg, palette::BG_BASE);
    assert_eq!(t.git_diff_border, palette::BORDER);
    assert_eq!(t.git_diff_context_text, palette::TEXT_SUBTLE);
    assert_eq!(t.git_diff_hunk_bg, palette::BG_RAISED);
    assert_eq!(t.git_stage_checkbox_border, palette::BORDER);
    assert_eq!(
        t.git_stage_checkbox_checked_bg,
        palette::GIT_STAGE_CHECKBOX_CHECKED_BG
    );
    assert_eq!(t.git_stage_checkbox_unchecked_bg, palette::BG_RAISED);
    assert_eq!(t.git_commit_border, palette::BORDER);
    assert_eq!(
        t.input_panel_drop_target_bg,
        palette::INPUT_PANEL_DROP_TARGET_BG
    );
    assert_eq!(t.terminal_drop_target_bg, palette::TERMINAL_DROP_TARGET_BG);
    assert_eq!(t.dock_scrollbar_thumb, palette::SCROLLBAR_THUMB);
    assert_eq!(t.dock_scrollbar_thumb_hover, palette::SCROLLBAR_THUMB_HOVER);
    assert_eq!(t.pane_header_focused_bg, palette::BG_HOVER);
    assert_eq!(t.pane_header_unfocused_bg, palette::BG_PANEL);
    assert_eq!(t.pane_header_cwd_text, palette::TEXT_MUTE);
    assert_eq!(t.pane_divider_bg, palette::BORDER);
    assert_eq!(t.welcome_bg, palette::BG_BASE);
    assert_eq!(t.welcome_button_bg, palette::BG_HOVER);
    assert_eq!(t.welcome_button_hover_bg, palette::BG_ACTIVE);
    assert_eq!(t.welcome_button_border, palette::BORDER);
    assert_eq!(t.welcome_recent_hover_bg, palette::BG_HOVER);
    assert_eq!(t.file_viewer_bg, palette::BG_BASE);
    assert_eq!(t.file_viewer_header_bg, palette::BG_PANEL);
    assert_eq!(t.file_viewer_header_border, palette::BORDER);
    assert_eq!(t.file_viewer_header_text, palette::TEXT_BODY);
    assert_eq!(t.file_viewer_text, palette::TEXT_BODY);
    assert_eq!(t.file_viewer_line_no_text, palette::TEXT_SUBTLE);
    assert_eq!(t.file_viewer_tab_active_bg, palette::BG_ACTIVE);
    assert_eq!(t.file_viewer_tab_text, palette::TEXT_MUTE);
    assert_eq!(t.file_diff_add_bg, palette::FILE_DIFF_ADD_BG);
    assert_eq!(t.file_diff_del_bg, palette::FILE_DIFF_DEL_BG);
    assert_eq!(t.file_diff_add_text, palette::FILE_DIFF_ADD_TEXT);
    assert_eq!(t.file_diff_del_text, palette::FILE_DIFF_DEL_TEXT);
    assert_eq!(t.file_diff_hunk_bg, palette::BG_RAISED);
    assert_eq!(t.file_diff_hunk_text, palette::FILE_DIFF_HUNK_TEXT);
    assert_eq!(t.file_diff_hunk_border, palette::BORDER);
    assert_eq!(t.file_diff_ctx_text, palette::TEXT_MUTE);
    assert_eq!(t.file_viewer_divider, palette::BORDER);
    assert_eq!(t.file_viewer_scrollbar_thumb, palette::SCROLLBAR_THUMB);
    assert_eq!(
        t.file_viewer_scrollbar_thumb_hover,
        palette::SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.file_diff_hunk_ctx_text, palette::FILE_DIFF_HUNK_CTX_TEXT);
    assert_eq!(t.file_diff_stat_add, palette::FILE_DIFF_STAT_ADD);
    assert_eq!(t.file_diff_stat_del, palette::FILE_DIFF_STAT_DEL);
    assert_eq!(t.file_diff_word_add_bg, palette::FILE_DIFF_WORD_ADD_BG);
    assert_eq!(t.file_diff_word_del_bg, palette::FILE_DIFF_WORD_DEL_BG);
    assert_eq!(t.file_viewer_search_text, palette::TEXT_BODY);
    assert_eq!(t.file_viewer_search_count, palette::TEXT_MUTE);
    assert_eq!(
        t.file_viewer_search_empty,
        palette::FILE_VIEWER_SEARCH_EMPTY
    );
    assert_eq!(
        t.file_viewer_search_match_bg,
        palette::FILE_VIEWER_SEARCH_MATCH_BG
    );
    assert_eq!(
        t.file_viewer_search_focused_bg,
        palette::FILE_VIEWER_SEARCH_FOCUSED_BG
    );
    assert_eq!(t.md_h1_color, palette::INK);
    assert_eq!(t.md_h2_color, palette::MD_H2_COLOR);
    assert_eq!(t.md_h3_color, palette::TEXT_BODY);
    assert_eq!(t.md_h4_color, palette::TEXT_BODY);
    assert_eq!(t.md_code_inline_bg, palette::BG_RAISED);
    assert_eq!(t.md_code_block_bg, palette::BG_PANEL);
    assert_eq!(t.md_code_block_border, palette::BORDER);
    assert_eq!(t.md_blockquote_border, palette::BORDER);
    assert_eq!(t.md_blockquote_text, palette::TEXT_MUTE);
    assert_eq!(t.md_rule_color, palette::BORDER);
    assert_eq!(t.md_footnote_color, palette::MD_FOOTNOTE_COLOR);
    assert_eq!(t.md_table_border, palette::BORDER);
    assert_eq!(t.md_table_header_bg, palette::BG_RAISED);
    assert_eq!(t.md_table_row_bg_even, palette::BG_PANEL);
    assert_eq!(t.md_table_row_bg_odd, palette::BG_PANEL);
    assert_eq!(t.toast_border, palette::BORDER);
    assert_eq!(t.toast_text, palette::TEXT_BODY);
    assert_eq!(t.toast_action_text, palette::PRIMARY);
    assert_eq!(t.toast_repeat_bg, palette::BG_ACTIVE);
    assert_eq!(t.error_modal_body_bg, palette::BG_BASE);
    assert_eq!(t.error_modal_body_border, palette::BORDER);
    assert_eq!(t.terminal_bg, palette::TERMINAL_BG);
    assert_eq!(t.terminal_scrollbar_track_bg, palette::SCROLLBAR_TRACK_BG);
    assert_eq!(t.keystroke_input_bg, palette::BG_PANEL);
    assert_eq!(t.keystroke_input_border, palette::BORDER);
    assert_eq!(t.keystroke_input_border_active, palette::PRIMARY);
    assert_eq!(t.keystroke_badge_bg, palette::BG_ACTIVE);
    assert_eq!(t.keystroke_badge_border, palette::BORDER);
    assert_eq!(t.keystroke_badge_text, palette::TEXT_BODY);
    assert_eq!(t.keystroke_hint_text, palette::TEXT_SUBTLE);
    assert_eq!(t.popover_border, palette::BORDER);
    assert_eq!(t.status_working_light, palette::STATUS_WORKING_LIGHT);
    assert_eq!(
        t.status_executing_tool_light,
        palette::STATUS_EXECUTING_TOOL_LIGHT
    );
    assert_eq!(
        t.status_needs_attention_light,
        palette::STATUS_NEEDS_ATTENTION_LIGHT
    );
    assert_eq!(t.status_idle_light, palette::STATUS_IDLE_LIGHT);
    assert_eq!(t.status_connecting_light, palette::STATUS_CONNECTING_LIGHT);
    assert_eq!(t.status_working_dark, palette::STATUS_WORKING_DARK);
    assert_eq!(
        t.status_executing_tool_dark,
        palette::STATUS_EXECUTING_TOOL_DARK
    );
    assert_eq!(
        t.status_needs_attention_dark,
        palette::STATUS_NEEDS_ATTENTION_DARK
    );
    assert_eq!(t.status_idle_dark, palette::STATUS_IDLE_DARK);
    assert_eq!(t.status_connecting_dark, palette::STATUS_CONNECTING_DARK);
    assert_eq!(
        t.status_badge_active_outline,
        palette::STATUS_BADGE_ACTIVE_OUTLINE
    );
    assert_eq!(t.claude_banner_bg, palette::CLAUDE_BANNER_BG);
    assert_eq!(t.claude_banner_border, palette::CLAUDE_BANNER_BORDER);
    assert_eq!(t.claude_banner_hover_bg, palette::CLAUDE_BANNER_HOVER_BG);
    assert_eq!(t.claude_banner_icon, palette::CLAUDE_BANNER_ICON);
    assert_eq!(t.right_panel_task_running_color, palette::SIGNAL_GREEN);
    assert_eq!(t.task_edit_branch_invalid_border, palette::ERROR);
    assert_eq!(t.badge_bg, palette::BG_HOVER);
    assert_eq!(t.badge_border, palette::BORDER);
    assert_eq!(t.badge_text, palette::TEXT_BODY);
    assert_eq!(t.divider_default_color, palette::BORDER);
    assert_eq!(t.gauge_track_bg, palette::BG_HOVER);
    assert_eq!(t.skill_aux_chip_bg, palette::SKILL_AUX_CHIP_BG);
    assert_eq!(t.skill_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.mcp_indicator_malformed, palette::MCP_INDICATOR_MALFORMED);
    assert_eq!(t.mcp_malformed_badge_bg, palette::MCP_MALFORMED_BADGE_BG);
    assert_eq!(
        t.mcp_malformed_badge_text,
        palette::MCP_MALFORMED_BADGE_TEXT
    );
}

#[test]
fn json_round_trip_preserves_default_within_8bit_tolerance() {
    // `gpui::Hsla` serialises through its `Rgba` representation,
    // which is 8-bit per channel (`#rrggbbaa`). A pure-Hsla value
    // like `hsla(_, _, 0.1, 1.0)` rounds through Rgba as 26/255 =
    // 0.101960786 on the way back — visually indistinguishable
    // but not bit-equal. The round-trip is therefore lossy by
    // design; the contract we *can* assert is "every channel
    // stays within one 8-bit step (1/255 ≈ 0.0039) of the input."
    //
    // Probing one slot per major slot group is enough to catch a
    // future field type change that breaks the contract entirely
    // (e.g. accidentally typing a slot as `f32` and rounding
    // through some other serializer).
    let original = DarudaTheme::default();
    let json = original.to_json_pretty().expect("serialize");
    let parsed = DarudaTheme::from_json(&json).expect("deserialize");

    // One 8-bit step plus a slack epsilon for floating-point noise
    // in the HSL→RGB→HSL conversion path.
    const TOL: f32 = 1.5 / 255.0;
    fn close(a: gpui::Hsla, b: gpui::Hsla, tol: f32) -> bool {
        (a.h - b.h).abs() <= tol
            && (a.s - b.s).abs() <= tol
            && (a.l - b.l).abs() <= tol
            && (a.a - b.a).abs() <= tol
    }
    for (name, a, b) in [
        ("title_bar_bg", parsed.title_bar_bg, original.title_bar_bg),
        (
            "modal_panel_bg",
            parsed.modal_panel_bg,
            original.modal_panel_bg,
        ),
        (
            "banner_error_text",
            parsed.banner_error_text,
            original.banner_error_text,
        ),
        (
            "mcp_malformed_badge_bg",
            parsed.mcp_malformed_badge_bg,
            original.mcp_malformed_badge_bg,
        ),
    ] {
        assert!(
            close(a, b, TOL),
            "{name}: round-trip diverged beyond 8-bit tolerance: {a:?} vs {b:?}"
        );
    }
}

#[test]
fn from_json_empty_object_fills_defaults() {
    // `{}` is the minimal valid theme — every slot falls back to
    // the compile-time `palette` const. Phase 3-C relies on this
    // when a user-authored JSON omits slots they don't care to
    // override.
    let parsed = DarudaTheme::from_json("{}").expect("empty object parses");
    let defaults = DarudaTheme::default();
    assert_eq!(parsed.tab_inactive_bg, defaults.tab_inactive_bg);
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
    assert_eq!(parsed.dock_bg, defaults.dock_bg);
}

#[test]
fn from_json_partial_overrides_only_listed_keys() {
    // A user-authored JSON that lists only one key must override
    // exactly that key; every other slot keeps the dark default.
    // Double-hash delimiter because the JSON body itself contains
    // `"#ff00ff"` — single-hash raw strings would terminate early.
    let parsed = DarudaTheme::from_json(r##"{ "tab_inactive_bg": "#ff00ff" }"##)
        .expect("partial JSON parses");
    let defaults = DarudaTheme::default();

    // Overridden slot reflects the JSON value (magenta).
    assert_ne!(parsed.tab_inactive_bg, defaults.tab_inactive_bg);
    // Untouched slot keeps the default.
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
}

#[test]
fn bundled_daruda_dark_json_matches_default() {
    // `assets/themes/daruda_dark.json` is committed alongside the
    // crate so the runtime loader (Phase 3-C) can read a real
    // file. This test asserts the bundled JSON has not drifted
    // from the compile-time defaults — if a future palette tweak
    // changes a default colour, regenerate the asset via
    //   cargo run --example dump_default_theme  # or the build script
    // and re-commit. Catching drift in CI is far cheaper than
    // chasing a visual mismatch in production.
    let bundled = include_str!("../../../../../../assets/themes/daruda_dark.json");
    let parsed =
        DarudaTheme::from_json(bundled).expect("bundled daruda_dark.json must be valid JSON");
    let defaults = DarudaTheme::default();

    // Spot-check a representative slice across the major slot
    // groups. The full-field equality is enforced by the
    // generator (the same `to_json_pretty` we test in the
    // round-trip case above), so we only need a handful of
    // probes here to detect "file was hand-edited and the
    // generator was not re-run."
    assert_eq!(parsed.title_bar_bg, defaults.title_bar_bg);
    assert_eq!(parsed.tab_inactive_bg, defaults.tab_inactive_bg);
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
    assert_eq!(parsed.modal_panel_bg, defaults.modal_panel_bg);
    assert_eq!(parsed.banner_error_text, defaults.banner_error_text);
    assert_eq!(parsed.lane_row_hover_bg, defaults.lane_row_hover_bg);
}

#[test]
fn json_schema_lists_every_theme_slot() {
    // `DarudaTheme::json_schema()` drives the bundled
    // `assets/themes/theme.schema.json`. If a new slot is added to
    // the `daruda_theme_fields!` macro but the schema regenerator
    // is not re-run, editors that wired `$schema` to the bundled
    // file would miss autocomplete for the new key. Guard the
    // invariant in CI: every `pub` field on `DarudaTheme` must be
    // listed in the schema's `properties` object.
    let schema = DarudaTheme::json_schema();
    let value = serde_json::to_value(&schema).expect("schema serialises");
    let properties = value
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema has a properties object");

    // Probe a representative slice — the macro guarantees the
    // struct and `Default::default()` stay in lockstep, so testing
    // every property here would just re-test the macro. Sample
    // across groups to catch a missing derive on `Hsla` or a
    // schemars version bump that breaks field emission.
    for slot in [
        "title_bar_bg",
        "tab_inactive_bg",
        "status_bar_bg",
        "modal_panel_bg",
        "lane_row_hover_bg",
        "right_panel_task_running_color",
    ] {
        assert!(
            properties.contains_key(slot),
            "json schema missing slot `{slot}` — did `daruda_theme_fields!` lose a row?"
        );
    }
}

/// Regeneration helper for `assets/themes/theme.schema.json`. Run
/// manually after a `daruda_theme_fields!` change:
///
/// ```bash
/// cargo test -p daruda \
///   ui::theme::daruda_theme::tests::regenerate_theme_schema_json \
///   -- --ignored --nocapture
/// ```
///
/// Marked `#[ignore]` so the regular test run never overwrites
/// disk — only an explicit invocation can mutate the bundled
/// schema. Committing the regenerated file is what makes
/// `bundled_theme_schema_json_matches_generated` (below) pass on
/// the next run.
#[test]
#[ignore = "regenerator — writes to assets/themes/theme.schema.json"]
fn regenerate_theme_schema_json() {
    let schema = serde_json::to_value(DarudaTheme::json_schema())
        .expect("DarudaTheme::json_schema() must serialise");
    let pretty =
        serde_json::to_string_pretty(&schema).expect("schema must serialise to pretty JSON");
    let dest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/themes/theme.schema.json");
    std::fs::write(&dest, pretty).expect("write theme.schema.json");
    println!("wrote {}", dest.display());
}

#[test]
fn bundled_theme_schema_json_matches_generated() {
    // Same "lockstep with disk" invariant as
    // `bundled_daruda_dark_json_matches_default`, but for the
    // companion JSON Schema. Editors that point `$schema` at the
    // bundled file would otherwise show stale autocomplete after
    // a slot rename / addition. Regenerate via
    //   cargo run --example dump_theme_schema  # or the build script
    // (see `assets/themes/README.md`).
    let bundled = include_str!("../../../../../../assets/themes/theme.schema.json");
    let bundled_value: serde_json::Value =
        serde_json::from_str(bundled).expect("bundled theme.schema.json must be valid JSON");
    let generated = serde_json::to_value(DarudaTheme::json_schema())
        .expect("DarudaTheme::json_schema() must serialise to JSON");
    assert_eq!(
        bundled_value, generated,
        "bundled theme.schema.json has drifted from DarudaTheme::json_schema()"
    );
}

#[test]
fn is_dark_reflects_base_background_lightness() {
    // The default theme is the dark palette.
    assert!(DarudaTheme::default().is_dark());

    // A light base background flips the verdict.
    let light = DarudaTheme {
        title_bar_bg: gpui::hsla(0.0, 0.0, 0.96, 1.0),
        ..Default::default()
    };
    assert!(!light.is_dark());
}
