use super::*;
use theme_presets::colors_for_preset;

#[test]
fn default_config_is_valid() {
    let cfg = Config::default();
    assert_eq!(cfg.font.size, 13.0);
    assert_eq!(cfg.cursor.style, CursorStyle::Block);
    assert!(cfg.cursor.blinking);
    assert_eq!(cfg.window.opacity, 1.0);
    assert!(!cfg.window.blur);
    assert_eq!(cfg.scrollback.lines, 10_000);
    assert!(cfg.keybindings.bindings.is_empty());
    assert_eq!(cfg.left_dock.left_default_width, 220.0);
    assert!(cfg.left_dock.left_collapsed_by_default);
}

#[test]
fn left_dock_parses_from_toml() {
    let input = "\
[left_dock]\n\
left_default_width = 260.0\n\
left_collapsed_by_default = false\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.left_dock.left_default_width, 260.0);
    assert!(!cfg.left_dock.left_collapsed_by_default);
}

#[test]
fn panels_grid_columns_default_is_five() {
    let cfg = Config::default();
    assert_eq!(cfg.panels.grid_columns, 5);
}

#[test]
fn panels_grid_columns_clamps_to_range() {
    let input = "[panels]\ngrid_columns = 0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.panels.grid_columns, 1);

    let input = "[panels]\ngrid_columns = 99\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.panels.grid_columns, 16);
}

#[test]
fn left_dock_width_clamps_to_range() {
    let input = "[left_dock]\nleft_default_width = 9999.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.left_dock.left_default_width, 400.0);

    let input = "[left_dock]\nleft_default_width = 10.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.left_dock.left_default_width, 150.0);
}

#[test]
fn empty_toml_produces_defaults() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(cfg.font.size, 13.0);
    assert_eq!(cfg.colors.foreground, HexColor::new(0xD4, 0xD4, 0xD4));
}

#[test]
fn missing_agents_seeds_single_claude_default() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(
        cfg.agents,
        vec![AgentEntry::Custom(AgentDefinition::claude_default())]
    );
    assert_eq!(cfg.resolved_agents()[0].id, "claude");
}

#[test]
fn explicit_agents_wholesale_replace_the_default() {
    let input = "\
[[agents]]\n\
id = \"codex\"\n\
name = \"Codex\"\n\
command = \"codex acp\"\n\
\n\
[[agents]]\n\
id = \"custom\"\n\
name = \"My Agent\"\n\
command = \"my-agent --acp\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    let resolved = cfg.resolved_agents();
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].id, "codex");
    assert_eq!(resolved[1].id, "custom");
    // The Claude default is gone — a provided array replaces, not merges.
    assert!(resolved.iter().all(|a| a.id != "claude"));
}

#[test]
fn agents_round_trip_through_toml() {
    let cfg = Config {
        agents: vec![
            AgentEntry::Custom(AgentDefinition {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                launch: AgentLaunch::Raw("codex acp".to_string()),
                default_mode: None,
                default_model: None,
            }),
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides {
                    name: Some("Codex (renamed)".to_string()),
                    ..PresetOverrides::default()
                },
            },
            AgentEntry::Custom(AgentDefinition::claude_default()),
        ],
        ..Config::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    let back: Config = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back.agents, cfg.agents);
}

#[test]
fn a_preset_reference_resolves_but_an_unknown_one_only_stays_persisted() {
    let input = "\
[[agents]]\n\
preset = \"codex-acp\"\n\
\n\
[[agents]]\n\
preset = \"retired-agent\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    // Both entries persist; only the resolvable one reaches the runtime.
    assert_eq!(cfg.agents.len(), 2);
    let resolved = cfg.resolved_agents();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0], AgentDefinition::codex_default());
}

#[test]
fn a_catalog_that_resolves_to_nothing_still_hands_out_the_built_in_default() {
    // Every consumer relies on `catalog[0]`; a config whose only entries are
    // unknown presets must not produce an empty runtime catalog.
    let mut cfg: Config = toml::from_str("[[agents]]\npreset = \"retired-agent\"\n").unwrap();
    cfg.clamp();
    assert_eq!(cfg.agents.len(), 1, "the unresolved entry is not pruned");
    assert_eq!(
        cfg.resolved_agents(),
        vec![AgentDefinition::claude_default()]
    );
}

#[test]
fn load_from_missing_path_normalizes_agents() {
    // The error branch returns Config::default(), which seeds the Claude default
    // directly (since a232e44), and still clamps — so the normalized non-empty
    // catalog holds regardless. Proves normalization runs on every load path.
    let cfg = Config::load_from(std::path::Path::new("/nonexistent/daruda/config.toml"));
    assert_eq!(
        cfg.agents,
        vec![AgentEntry::Custom(AgentDefinition::claude_default())]
    );
}

#[test]
fn explicitly_empty_agents_normalizes_to_claude_default() {
    let mut cfg: Config = toml::from_str("agents = []").unwrap();
    assert!(cfg.agents.is_empty());
    cfg.clamp();
    assert_eq!(
        cfg.agents,
        vec![AgentEntry::Custom(AgentDefinition::claude_default())]
    );
}

#[test]
fn font_inset_defaults_to_4_and_2() {
    let cfg = Config::default();
    assert_eq!(cfg.font.inset_x, 4.0);
    assert_eq!(cfg.font.inset_y, 2.0);
}

#[test]
fn font_inset_clamps_to_range() {
    let input = "[font]\ninset_x = 100.0\ninset_y = -5.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.inset_x, 32.0);
    assert_eq!(cfg.font.inset_y, 0.0);
}

#[test]
fn partial_toml_fills_missing_with_defaults() {
    let input = "\
[font]\n\
size = 16.0\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.font.size, 16.0);
    assert_eq!(cfg.font.family, FontConfig::default().family);
    assert_eq!(cfg.cursor.style, CursorStyle::Block);
}

#[test]
fn hex_color_parses_valid_input() {
    let input = "\
[colors]\n\
foreground = \"#ff8800\"\n\
background = \"#001122\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.colors.foreground, HexColor::new(0xFF, 0x88, 0x00));
    assert_eq!(cfg.colors.background, HexColor::new(0x00, 0x11, 0x22));
}

#[test]
fn hex_color_rejects_invalid_input() {
    let input = "\
[colors]\n\
foreground = \"not-a-color\"\n";
    let result: Result<Config, _> = toml::from_str(input);
    assert!(result.is_err());
}

#[test]
fn cursor_style_parses_all_variants() {
    for (style_str, expected) in [
        ("block", CursorStyle::Block),
        ("underline", CursorStyle::Underline),
        ("bar", CursorStyle::Bar),
    ] {
        let input = format!("[cursor]\nstyle = \"{style_str}\"\n");
        let cfg: Config = toml::from_str(&input).unwrap();
        assert_eq!(cfg.cursor.style, expected);
    }
}

#[test]
fn font_size_clamps_to_range() {
    let input = "[font]\nsize = 200.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.size, 72.0);

    let input = "[font]\nsize = 1.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.size, 6.0);
}

#[test]
fn editor_font_size_clamps_to_range() {
    let input = "[font]\neditor_size = 200.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.editor_size, 72.0);

    let input = "[font]\neditor_size = 1.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.editor_size, 6.0);
}

#[test]
fn opacity_clamps_to_range() {
    let input = "[window]\nopacity = 0.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.window.opacity, 0.1);
}

#[test]
fn keybindings_parse_flat_map() {
    let input = "\
[keybindings]\n\
\"cmd-t\" = \"new_tab\"\n\
\"cmd-d\" = \"split_right\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.keybindings.bindings.len(), 2);
    assert_eq!(cfg.keybindings.bindings["cmd-t"], "new_tab");
}

#[test]
fn ansi_palette_as_array_returns_8_entries() {
    let palette = AnsiPalette::default();
    let arr = palette.as_array();
    assert_eq!(arr.len(), 8);
    assert_eq!(arr[0], palette.black);
    assert_eq!(arr[7], palette.white);
}

#[test]
fn load_from_missing_file_returns_defaults() {
    let cfg = Config::load_from(std::path::Path::new("/nonexistent/path/config.toml"));
    assert_eq!(cfg.font.size, 13.0);
}

#[test]
fn hex_color_display_roundtrips() {
    let c = HexColor::new(0xAB, 0xCD, 0xEF);
    assert_eq!(format!("{c}"), "#abcdef");
}

#[test]
fn full_config_parses_all_sections() {
    let input = "\
[font]\n\
family = \"JetBrains Mono\"\n\
size = 14.0\n\
vertical_spacing = 1.2\n\
horizontal_spacing = 0.9\n\
\n\
[cursor]\n\
style = \"underline\"\n\
blinking = false\n\
\n\
[window]\n\
opacity = 0.85\n\
blur = true\n\
\n\
[colors]\n\
foreground = \"#e0e0e0\"\n\
background = \"#282828\"\n\
\n\
[colors.normal]\n\
black = \"#111111\"\n\
red = \"#ff0000\"\n\
green = \"#00ff00\"\n\
yellow = \"#ffff00\"\n\
blue = \"#0000ff\"\n\
magenta = \"#ff00ff\"\n\
cyan = \"#00ffff\"\n\
white = \"#ffffff\"\n\
\n\
[colors.bright]\n\
black = \"#333333\"\n\
red = \"#ff3333\"\n\
green = \"#33ff33\"\n\
yellow = \"#ffff33\"\n\
blue = \"#3333ff\"\n\
magenta = \"#ff33ff\"\n\
cyan = \"#33ffff\"\n\
white = \"#ffffff\"\n\
\n\
[scrollback]\n\
lines = 50000\n\
\n\
[keybindings]\n\
\"cmd-t\" = \"new_tab\"\n";

    let cfg: Config = toml::from_str(input).unwrap();

    assert_eq!(cfg.font.family, "JetBrains Mono");
    assert_eq!(cfg.font.size, 14.0);
    assert_eq!(cfg.cursor.style, CursorStyle::Underline);
    assert!(!cfg.cursor.blinking);
    assert_eq!(cfg.window.opacity, 0.85);
    assert!(cfg.window.blur);
    assert_eq!(cfg.colors.foreground, HexColor::new(0xE0, 0xE0, 0xE0));
    assert_eq!(cfg.colors.normal.red, HexColor::new(0xFF, 0x00, 0x00));
    assert_eq!(cfg.colors.bright.cyan, HexColor::new(0x33, 0xFF, 0xFF));
    assert_eq!(cfg.scrollback.lines, 50_000);
    assert_eq!(cfg.keybindings.bindings["cmd-t"], "new_tab");
}

#[test]
fn theme_config_defaults_to_default_preset() {
    let cfg = Config::default();
    assert_eq!(cfg.theme.terminal_preset, "default");
    assert_eq!(cfg.theme.ui_preset, "daruda_dark");
}

#[test]
fn theme_config_parses_from_toml() {
    let input = "[theme]\nterminal_preset = \"dracula\"\nui_preset = \"daruda_dark\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.theme.terminal_preset, "dracula");
    assert_eq!(cfg.theme.ui_preset, "daruda_dark");
}

#[test]
fn theme_config_accepts_legacy_preset_alias() {
    // Pre-Phase-2 configs used the unqualified `preset` key. Serde
    // alias should keep those configs loading without an opt-in
    // migration step.
    let input = "[theme]\npreset = \"dracula\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.theme.terminal_preset, "dracula");
}

#[test]
fn effective_colors_returns_preset_when_named() {
    let mut cfg = Config::default();
    cfg.theme.terminal_preset = "dracula".to_owned();
    let effective = cfg.effective_colors();
    let preset = colors_for_preset("dracula").unwrap();
    assert_eq!(effective.background, preset.background);
    assert_eq!(effective.foreground, preset.foreground);
}

#[test]
fn effective_colors_falls_back_to_colors_section_for_custom() {
    let input = "[theme]\nterminal_preset = \"custom\"\n[colors]\nforeground = \"#aabbcc\"\n";
    let cfg: Config = toml::from_str(input).unwrap();
    let effective = cfg.effective_colors();
    assert_eq!(effective.foreground, HexColor::new(0xAA, 0xBB, 0xCC));
}

#[test]
fn effective_colors_falls_back_to_colors_section_for_unknown_preset() {
    let mut cfg = Config::default();
    cfg.theme.terminal_preset = "__unknown_preset__".to_owned();
    let effective = cfg.effective_colors();
    assert_eq!(effective.foreground, cfg.colors.foreground);
}

#[test]
fn to_ansi_palette_maps_normal_to_0_7_and_bright_to_8_15() {
    let colors = ColorConfig::default();
    let pal = colors.to_ansi_palette();
    assert_eq!(
        pal[0],
        [
            colors.normal.black.r,
            colors.normal.black.g,
            colors.normal.black.b
        ]
    );
    assert_eq!(
        pal[7],
        [
            colors.normal.white.r,
            colors.normal.white.g,
            colors.normal.white.b
        ]
    );
    assert_eq!(
        pal[8],
        [
            colors.bright.black.r,
            colors.bright.black.g,
            colors.bright.black.b
        ]
    );
    assert_eq!(
        pal[15],
        [
            colors.bright.white.r,
            colors.bright.white.g,
            colors.bright.white.b
        ]
    );
}

#[test]
fn patch_config_file_preserves_unmanaged_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // Write an initial file with a [keybindings] section that the Settings UI
    // does not touch.
    std::fs::write(
        &path,
        "[keybindings]\n\"cmd-t\" = \"new_tab\"\n\n[font]\nsize = 14.0\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);

    // Simulate Settings saving a font size change.
    let mut updated = cfg.clone();
    updated.font.size = 16.0;

    // Call a path-aware variant so we don't write to the real config location.
    patch_config_file_to(&updated, &path).unwrap();

    let reloaded = Config::load_from(&path);
    // The font size change must be applied.
    assert_eq!(reloaded.font.size, 16.0);
    // The keybinding must be preserved.
    assert_eq!(reloaded.keybindings.bindings["cmd-t"], "new_tab");
}

#[test]
fn settings_patch_only_rewrites_the_addressed_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# keep this comment\n[font]\nsize = 14.0\neditor_size = 17.0 # external value\n",
    )
    .unwrap();

    let written = crate::apply_settings_patch_to(&crate::SettingsPatch::FontSize(16.0), &path)
        .expect("field patch");

    assert_eq!(written.font.size, 16.0);
    assert_eq!(written.font.editor_size, 17.0);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("# keep this comment"), "{text}");
    assert!(
        text.contains("editor_size = 17.0 # external value"),
        "{text}"
    );
}

#[test]
fn settings_patch_updates_an_inline_table_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "font = { size = 14.0, editor_size = 17.0 } # keep inline\n",
    )
    .unwrap();

    let written = crate::apply_settings_patch_to(&crate::SettingsPatch::FontSize(16.0), &path)
        .expect("inline field patch");

    assert_eq!(written.font.size, 16.0);
    assert_eq!(written.font.editor_size, 17.0);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("font = {"), "{text}");
    assert!(text.contains("# keep inline"), "{text}");
}

#[test]
fn checked_settings_patch_rejects_a_same_field_disk_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[font]\nsize = 14.0\n").unwrap();
    let expected = Config::load_from(&path);
    std::fs::write(&path, "[font]\nsize = 20.0\n").unwrap();

    let error = crate::apply_settings_patch_to_if_unchanged(
        &crate::SettingsPatch::FontSize(16.0),
        &expected,
        &path,
    )
    .expect_err("same-field external edit must conflict");

    assert_eq!(
        error,
        crate::SettingsPatchApplyError::Conflict(crate::SettingsFieldId::FontSize)
    );
    assert_eq!(Config::load_from(&path).font.size, 20.0);
}

#[test]
fn checked_settings_patch_keeps_an_unrelated_disk_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[font]\nsize = 14.0\neditor_size = 13.0\n").unwrap();
    let expected = Config::load_from(&path);
    std::fs::write(&path, "[font]\nsize = 14.0\neditor_size = 17.0\n").unwrap();

    let written = crate::apply_settings_patch_to_if_unchanged(
        &crate::SettingsPatch::FontSize(16.0),
        &expected,
        &path,
    )
    .expect("unrelated external edit");

    assert_eq!(written.font.size, 16.0);
    assert_eq!(written.font.editor_size, 17.0);
}

#[cfg(unix)]
#[test]
fn settings_patch_preserves_a_config_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("managed-config.toml");
    let path = dir.path().join("config.toml");
    std::fs::write(&target, "[font]\nsize = 14.0\n").unwrap();
    symlink(&target, &path).unwrap();

    crate::apply_settings_patch_to(&crate::SettingsPatch::FontSize(16.0), &path)
        .expect("symlinked config patch");

    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(Config::load_from(&target).font.size, 16.0);
}

#[test]
fn settings_patch_refuses_to_replace_a_corrupt_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let corrupt = "[font\nsize = 14";
    std::fs::write(&path, corrupt).unwrap();

    let error = crate::apply_settings_patch_to(&crate::SettingsPatch::FontSize(16.0), &path)
        .expect_err("corrupt config must block the write");

    assert!(error.contains("parse error"), "{error}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), corrupt);
}

#[test]
fn settings_patch_round_trips_structural_session_hosts() {
    use daruda_store::project::SessionHostId;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let host = SessionHostEntry {
        id: SessionHostId::new(),
        label: "Build box".to_string(),
        kind: SessionHostKind::Ssh {
            target: "vm-work".to_string(),
        },
    };
    let tombstone = SessionHostTombstone {
        old_id: SessionHostId::new(),
        kind: SessionHostKind::Docker {
            container: "old-dev".to_string(),
        },
        value: "old-dev".to_string(),
        removed_at: 1_700_000_000,
        redirected_to: Some(host.id),
    };

    let written = crate::apply_settings_patch_to(
        &crate::SettingsPatch::SessionHosts {
            entries: vec![host.clone()],
            tombstones: vec![tombstone.clone()],
        },
        &path,
    )
    .expect("catalog patch");

    assert_eq!(written.session_hosts, vec![host]);
    assert_eq!(written.session_host_tombstones, vec![tombstone]);
}

#[test]
fn settings_patch_writes_render_max_fps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    crate::apply_settings_patch_to(&crate::SettingsPatch::RenderMaxFps(60), &path)
        .expect("render patch");

    assert_eq!(Config::load_from(&path).render.max_fps, 60);
}

#[test]
fn patch_config_file_creates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.font.size = 18.0;
    patch_config_file_to(&cfg, &path).unwrap();

    assert!(path.exists());
    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.font.size, 18.0);
}

#[test]
fn patch_config_file_writes_agent_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let cfg = Config {
        agent: AgentConfig {
            use_modifier_to_send: true,
            ..AgentConfig::default()
        },
        ..Config::default()
    };
    patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert!(reloaded.agent.use_modifier_to_send);
}

#[test]
fn patch_config_file_clears_a_stale_permission_mode_key() {
    // The global permission-mode axis was removed; a config.toml written by
    // an older daruda build can still carry the key. Load must migrate it into
    // compatible catalog rows, and a full save must clear the stale key.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[agent]\ndefault_permission_mode = \"plan\"\nuse_modifier_to_send = true\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    assert_eq!(
        cfg.resolved_agents()[0].default_mode.as_deref(),
        Some("plan"),
        "the legacy global default still reaches the built-in Claude row"
    );
    patch_config_file_to(&cfg, &path).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("default_permission_mode"), "{text}");
    assert!(text.contains("default_mode = \"plan\""), "{text}");
}

#[test]
fn legacy_permission_mode_migration_skips_known_incompatible_agents() {
    let input = "\
[agent]\n\
default_permission_mode = \"plan\"\n\
[[agents]]\n\
id = \"claude\"\n\
name = \"Claude Code\"\n\
command = \"npx -y @agentclientprotocol/claude-agent-acp@latest\"\n\
[[agents]]\n\
preset = \"codex-acp\"\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();

    let resolved = cfg.resolved_agents();
    assert_eq!(resolved[0].default_mode.as_deref(), Some("plan"));
    assert_eq!(
        resolved[1].default_mode, None,
        "Codex advertises its own mode vocabulary, so a Claude-only legacy id must not be pinned"
    );
}

#[test]
fn settings_patch_migrates_and_removes_a_stale_permission_mode_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[agent]\ndefault_permission_mode = \"acceptEdits\"\nuse_modifier_to_send = false\n",
    )
    .unwrap();

    let reloaded =
        apply_settings_patch_to(&SettingsPatch::FontSize(15.0), &path).expect("patch applies");
    assert_eq!(reloaded.font.size, 15.0);
    assert_eq!(
        reloaded.resolved_agents()[0].default_mode.as_deref(),
        Some("acceptEdits")
    );

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("default_permission_mode"), "{text}");
    assert!(text.contains("default_mode = \"acceptEdits\""), "{text}");
}

#[test]
fn patch_config_file_writes_non_default_agents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let cfg = Config {
        agents: vec![
            AgentEntry::Custom(AgentDefinition::claude_default()),
            AgentEntry::Custom(AgentDefinition::codex_default()),
        ],
        ..Config::default()
    };
    patch_config_file_to(&cfg, &path).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("[[agents]]"));
    assert!(on_disk.contains("@agentclientprotocol/codex-acp"));
    // The Codex row was written flat but matches its preset exactly, so the
    // reload promotes it to a reference — same resolved catalog either way.
    let reloaded = Config::load_from(&path);
    assert_eq!(
        reloaded.agents,
        vec![
            AgentEntry::Custom(AgentDefinition::claude_default()),
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides::default(),
            },
        ]
    );
    assert_eq!(reloaded.resolved_agents(), cfg.resolved_agents());
}

/// The catalog's real persistence boundary is the hand-built `toml_edit`
/// writer, not serde — a reference that came back as a flat `id`/`name`/
/// `command` copy would put the copy model back on disk, and an unresolved
/// entry the writer skipped would delete part of the user's config.
#[test]
fn patch_config_file_round_trips_every_agent_entry_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let cfg = Config {
        agents: vec![
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides::default(),
            },
            AgentEntry::Preset {
                preset: "gemini".to_string(),
                overrides: PresetOverrides {
                    name: Some("Gemini (pinned)".to_string()),
                    command: Some("npx -y @google/gemini-cli@0.9.0 --acp".to_string()),
                    default_mode: Some("plan".to_string()),
                    default_model: Some("gemini-2.5-pro".to_string()),
                },
            },
            AgentEntry::Preset {
                preset: "retired-agent".to_string(),
                overrides: PresetOverrides::default(),
            },
            AgentEntry::Custom(AgentDefinition {
                id: "hermes".to_string(),
                name: "Hermes Agent".to_string(),
                launch: AgentLaunch::Raw("hermes acp".to_string()),
                default_mode: Some("yolo".to_string()),
                default_model: None,
            }),
            AgentEntry::Custom(AgentDefinition {
                id: "remote".to_string(),
                name: "Remote".to_string(),
                launch: AgentLaunch::Ssh {
                    adapter_command: "npx -y some-acp".to_string(),
                    host: "vm-work".to_string(),
                },
                default_mode: None,
                default_model: Some("claude-opus-4".to_string()),
            }),
        ],
        ..Config::default()
    };
    patch_config_file_to(&cfg, &path).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("preset = \"codex-acp\""), "{on_disk}");
    // A bare reference carries no copied command.
    assert!(
        !on_disk.contains("@agentclientprotocol/codex-acp"),
        "{on_disk}"
    );
    assert!(on_disk.contains("preset = \"retired-agent\""), "{on_disk}");

    assert_eq!(Config::load_from(&path).agents, cfg.agents);
}

#[test]
fn patch_config_file_preserves_implicit_default_agents_when_unmanaged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    patch_config_file_to(&Config::default(), &path).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("[[agents]]"));
    let reloaded = Config::load_from(&path);
    assert_eq!(
        reloaded.agents,
        vec![AgentEntry::Custom(AgentDefinition::claude_default())]
    );
}

#[test]
fn patch_config_file_clamps_out_of_range_values() {
    // The Settings UI may construct a Config with values outside the
    // valid range (e.g., a slider bug). patch_config_file_to clamps
    // before writing so the on-disk file never holds invalid state.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.font.size = 999.0; // way over the 72.0 ceiling
    cfg.window.opacity = 5.0; // over 1.0
    patch_config_file_to(&cfg, &path).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        !on_disk.contains("999"),
        "expected clamped font size on disk; got: {on_disk}"
    );

    let reloaded = Config::load_from(&path);
    assert!(reloaded.font.size <= 72.0);
    assert!(reloaded.window.opacity <= 1.0);
}

// ---- Project layer (resolve / round-trip) ----

#[test]
fn resolve_with_empty_project_returns_user_unchanged() {
    let user = Config::default();
    let project = ProjectConfig::default();
    let effective = user.clone().resolve(&project);
    assert_eq!(effective.shell.program, user.shell.program);
    assert_eq!(
        effective.shell.close_pane_on_exit,
        user.shell.close_pane_on_exit
    );
    assert_eq!(effective.font.size, user.font.size);
}

#[test]
fn resolve_overrides_shell_section_only() {
    let mut user = Config::default();
    user.font.size = 17.0;
    user.shell.program = Some("/bin/bash".into());

    let project_shell = ShellConfig {
        program: Some("/usr/local/bin/zsh".into()),
        close_pane_on_exit: false,
        natural_text_editing: true,
    };
    let project = ProjectConfig {
        shell: Some(project_shell),
    };

    let effective = user.clone().resolve(&project);
    // Shell entirely replaced (section-level override).
    assert_eq!(
        effective.shell.program.as_deref(),
        Some("/usr/local/bin/zsh")
    );
    assert!(!effective.shell.close_pane_on_exit);
    // Other sections unchanged.
    assert_eq!(effective.font.size, 17.0);
}

#[test]
fn resolve_section_replace_drops_user_subkeys() {
    // `resolve` is wholesale section replacement — when project sets
    // `[shell]`, every field in the user shell section is dropped,
    // including ones the project file omits. The user opts in to this
    // by adding `[shell]` to the project config at all.
    let mut user = Config::default();
    user.shell.program = Some("/bin/bash".into());
    user.shell.close_pane_on_exit = false;

    let project = ProjectConfig {
        shell: Some(ShellConfig::default()),
    };
    let effective = user.resolve(&project);
    assert!(effective.shell.program.is_none());
    assert!(effective.shell.close_pane_on_exit);
}

// ---- telegram round-trip ----

#[test]
fn patch_config_file_round_trips_telegram_enabled_and_chat_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.telegram.enabled = true;
    cfg.telegram.authorized_chat_id = Some(999888777);
    cfg.telegram.defer_while_active = false;
    cfg.telegram.active_idle_secs = 5;
    crate::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert!(reloaded.telegram.enabled);
    assert_eq!(reloaded.telegram.authorized_chat_id, Some(999888777));
    assert!(!reloaded.telegram.defer_while_active);
    assert_eq!(reloaded.telegram.active_idle_secs, 5);
}

#[test]
fn patch_config_file_clears_telegram_chat_id_when_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.telegram.enabled = true;
    cfg.telegram.authorized_chat_id = Some(111);
    crate::patch_config_file_to(&cfg, &path).unwrap();

    // Unpair: authorized_chat_id goes back to None.
    cfg.telegram.authorized_chat_id = None;
    crate::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.telegram.authorized_chat_id, None);
}

// ---- general.language round-trip ----

#[test]
fn patch_config_file_round_trips_language() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.general.language = "ko".to_owned();
    crate::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.general.language, "ko");
}

#[test]
fn patch_config_file_round_trips_preferred_editor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let mut cfg = Config::default();
    cfg.editor.preferred = "vscode".to_owned();
    crate::patch_config_file_to(&cfg, &path).unwrap();

    let reloaded = Config::load_from(&path);
    assert_eq!(reloaded.editor.preferred, "vscode");
}

#[test]
fn config_default_language_is_auto() {
    assert_eq!(Config::default().general.language, "auto");
}

#[test]
fn config_load_missing_general_defaults_to_auto() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    // Write a config that has no [general] section.
    std::fs::write(&path, "[font]\nsize = 14.0\n").unwrap();
    let cfg = Config::load_from(&path);
    assert_eq!(cfg.general.language, "auto");
}

// ---- session host registry ----

/// Unlike `agents`, an absent/empty catalog is a valid steady state — no
/// non-empty seed. Covers both a totally empty document and a config that
/// sets unrelated sections but never mentions `session_hosts`.
#[test]
fn missing_session_hosts_defaults_to_an_empty_catalog() {
    let cfg: Config = toml::from_str("").unwrap();
    assert!(cfg.session_hosts.is_empty());
    assert!(cfg.session_host_tombstones.is_empty());

    let cfg: Config = toml::from_str("[font]\nsize = 14.0\n").unwrap();
    assert!(cfg.session_hosts.is_empty());
    assert!(cfg.session_host_tombstones.is_empty());

    assert!(Config::default().session_hosts.is_empty());
    assert!(Config::default().session_host_tombstones.is_empty());
}

#[test]
fn session_hosts_and_tombstones_round_trip_through_config_toml() {
    use daruda_store::project::SessionHostId;

    let cfg = Config {
        session_hosts: vec![
            SessionHostEntry {
                id: SessionHostId::new(),
                label: "Build box".to_string(),
                kind: SessionHostKind::Ssh {
                    target: "vm-work".to_string(),
                },
            },
            SessionHostEntry {
                id: SessionHostId::new(),
                label: "Dev container".to_string(),
                kind: SessionHostKind::Docker {
                    container: "dev-1".to_string(),
                },
            },
        ],
        session_host_tombstones: vec![SessionHostTombstone {
            old_id: SessionHostId::new(),
            kind: SessionHostKind::Ssh {
                target: "old-box".to_string(),
            },
            value: "old-box".to_string(),
            removed_at: 1_700_000_000,
            redirected_to: None,
        }],
        ..Config::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    let back: Config = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back.session_hosts, cfg.session_hosts, "{toml_str}");
    assert_eq!(
        back.session_host_tombstones, cfg.session_host_tombstones,
        "{toml_str}"
    );
}
