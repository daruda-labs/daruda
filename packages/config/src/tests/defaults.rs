use super::*;

#[test]
fn default_config_is_valid() {
    let cfg = Config::default();
    assert_eq!(cfg.font.terminal.size, 13.0);
    assert_eq!(cfg.cursor.style, CursorStyle::Block);
    assert!(cfg.cursor.blinking);
    assert_eq!(cfg.window.opacity, 1.0);
    assert!(!cfg.window.blur);
    assert!(cfg.keybindings.bindings.is_empty());
    assert_eq!(cfg.left_dock.left_default_width, 220.0);
    assert!(!cfg.left_dock.left_collapsed_by_default);
}

#[test]
fn left_dock_parses_from_toml() {
    let input = "\
[left_dock]\n\
left_default_width = 260.0\n\
left_collapsed_by_default = true\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.left_dock.left_default_width, 260.0);
    assert!(cfg.left_dock.left_collapsed_by_default);
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
    assert_eq!(cfg.font.terminal.size, 13.0);
    assert_eq!(cfg.colors.foreground, HexColor::new(0xD4, 0xD4, 0xD4));
}

#[test]
fn missing_agents_seeds_single_claude_default() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(
        cfg.agents,
        vec![AgentEntry::for_definition(
            AgentDefinition::claude_default(),
            None
        )]
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
            AgentEntry::custom(AgentDefinition::new(
                "codex".to_string(),
                "Codex".to_string(),
                AgentLaunch::Raw("codex acp".to_string()),
            )),
            AgentEntry::preset_with(
                "codex-acp".to_string(),
                PresetOverrides {
                    name: Some("Codex (renamed)".to_string()),
                    ..PresetOverrides::default()
                },
            ),
            AgentEntry::for_definition(AgentDefinition::claude_default(), None),
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
        vec![AgentEntry::for_definition(
            AgentDefinition::claude_default(),
            None
        )]
    );
}

#[test]
fn explicitly_empty_agents_normalizes_to_claude_default() {
    let mut cfg: Config = toml::from_str("agents = []").unwrap();
    assert!(cfg.agents.is_empty());
    cfg.clamp();
    assert_eq!(
        cfg.agents,
        vec![AgentEntry::for_definition(
            AgentDefinition::claude_default(),
            None
        )]
    );
}

#[test]
fn font_inset_defaults_to_4_and_2() {
    let cfg = Config::default();
    assert_eq!(cfg.font.terminal.inset_x, 4.0);
    assert_eq!(cfg.font.terminal.inset_y, 2.0);
}

#[test]
fn font_inset_clamps_to_range() {
    let input = "[font]\ninset_x = 100.0\ninset_y = -5.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.terminal.inset_x, 32.0);
    assert_eq!(cfg.font.terminal.inset_y, 0.0);
}

#[test]
fn partial_toml_fills_missing_with_defaults() {
    let input = "\
[font]\n\
size = 16.0\n";
    let cfg: Config = toml::from_str(input).unwrap();
    assert_eq!(cfg.font.terminal.size, 16.0);
    assert_eq!(
        cfg.font.terminal.family,
        FontConfig::default().terminal.family
    );
    assert_eq!(cfg.cursor.style, CursorStyle::Block);
}

#[test]
fn font_domains_parse_independently() {
    let input = r#"
[font.terminal]
family = "JetBrains Mono"
size = 14.0
line_height = 1.1
cell_width = 0.95

[font.editor]
family = "Iosevka"
size = 16.0
line_height = 1.8

[font.agent_chat]
family = ".SystemUIFont"
size = 15.0
line_height = 1.5
"#;
    let cfg: Config = toml::from_str(input).unwrap();

    assert_eq!(cfg.font.terminal.family, "JetBrains Mono");
    assert_eq!(cfg.font.terminal.size, 14.0);
    assert_eq!(cfg.font.terminal.line_height, 1.1);
    assert_eq!(cfg.font.terminal.cell_width, 0.95);
    assert_eq!(cfg.font.editor.family, "Iosevka");
    assert_eq!(cfg.font.editor.size, 16.0);
    assert_eq!(cfg.font.editor.line_height, 1.8);
    assert_eq!(cfg.font.agent_chat.family, ".SystemUIFont");
    assert_eq!(cfg.font.agent_chat.size, 15.0);
    assert_eq!(cfg.font.agent_chat.line_height, 1.5);
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_default_family_is_monaco_on_macos() {
    let cfg = Config::default();
    assert_eq!(cfg.font.terminal.family, "Monaco");
    assert_eq!(cfg.font.editor.family, "Monaco");
    assert_eq!(cfg.font.agent_chat.family, crate::SYSTEM_UI_FONT_FAMILY);
    assert_ne!(cfg.font.terminal.family, cfg.font.agent_chat.family);
}

#[test]
fn partially_migrated_font_config_falls_back_to_legacy_fields() {
    let input = r#"
[font]
family = "Legacy Mono"
size = 14.0
editor_size = 17.0
agent_chat_size = 15.0
vertical_spacing = 1.2

[font.editor]
line_height = 1.9
"#;
    let cfg: Config = toml::from_str(input).unwrap();

    assert_eq!(cfg.font.terminal.family, "Legacy Mono");
    assert_eq!(cfg.font.terminal.size, 14.0);
    assert_eq!(cfg.font.terminal.line_height, 1.2);
    assert_eq!(cfg.font.editor.family, "Legacy Mono");
    assert_eq!(cfg.font.editor.size, 17.0);
    assert_eq!(cfg.font.editor.line_height, 1.9);
    assert_eq!(cfg.font.agent_chat.size, 15.0);
    assert_eq!(
        cfg.font.agent_chat.family,
        FontConfig::default().agent_chat.family
    );
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
    assert_eq!(cfg.font.terminal.size, 72.0);

    let input = "[font]\nsize = 1.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.terminal.size, 6.0);
}

#[test]
fn editor_font_size_clamps_to_range() {
    let input = "[font]\neditor_size = 200.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.editor.size, 72.0);

    let input = "[font]\neditor_size = 1.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.editor.size, 6.0);
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
    assert_eq!(cfg.font.terminal.size, 13.0);
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

    assert_eq!(cfg.font.terminal.family, "JetBrains Mono");
    assert_eq!(cfg.font.terminal.size, 14.0);
    assert_eq!(cfg.cursor.style, CursorStyle::Underline);
    assert!(!cfg.cursor.blinking);
    assert_eq!(cfg.window.opacity, 0.85);
    assert!(cfg.window.blur);
    assert_eq!(cfg.colors.foreground, HexColor::new(0xE0, 0xE0, 0xE0));
    assert_eq!(cfg.colors.normal.red, HexColor::new(0xFF, 0x00, 0x00));
    assert_eq!(cfg.colors.bright.cyan, HexColor::new(0x33, 0xFF, 0xFF));
    // `[scrollback] lines` above is a retired key: it must still parse.
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
fn system_ui_preset_resolves_by_appearance() {
    use ui_theme_presets::{SYSTEM, resolve};
    assert_eq!(resolve(SYSTEM, true), "daruda_dark");
    assert_eq!(resolve(SYSTEM, false), "daruda_light");
    assert!(ui_theme_presets::is_known(resolve(SYSTEM, true)));
    assert!(ui_theme_presets::is_known(resolve(SYSTEM, false)));
    // An explicit preset ignores the OS appearance.
    assert_eq!(resolve("daruda_dark", false), "daruda_dark");
    assert_eq!(resolve("daruda_light", true), "daruda_light");
}

#[test]
fn theme_config_accepts_legacy_preset_alias() {
    // Legacy configs used the unqualified `preset` key. Serde alias
    // should keep those configs loading without an opt-in
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
fn nan_numbers_fall_back_to_their_defaults_instead_of_passing_through() {
    let input = "[font.editor]\nsize = nan\n[font.agent_chat]\nline_height = nan\n\
                 [window]\nopacity = nan\n[agent]\nreading_width = nan\n\
                 [left_dock]\nleft_default_width = nan\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    assert!(cfg.font.editor.size.is_nan(), "TOML really reads `nan`");
    cfg.clamp();
    let d = Config::default();
    assert_eq!(cfg.font.editor.size, d.font.editor.size);
    assert_eq!(
        cfg.font.agent_chat.line_height,
        d.font.agent_chat.line_height
    );
    assert_eq!(cfg.window.opacity, d.window.opacity);
    assert_eq!(cfg.agent.reading_width, d.agent.reading_width);
    assert_eq!(
        cfg.left_dock.left_default_width,
        d.left_dock.left_default_width
    );
    // An out-of-range real number still clamps rather than resetting.
    let mut big: Config = toml::from_str("[font.editor]\nsize = 500.0\n").unwrap();
    big.clamp();
    assert_eq!(big.font.editor.size, 72.0);
}
