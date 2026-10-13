use super::*;

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
fn font_inset_clamps_to_range() {
    let input = "[font]\ninset_x = 100.0\ninset_y = -5.0\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();
    assert_eq!(cfg.font.terminal.inset_x, 32.0);
    assert_eq!(cfg.font.terminal.inset_y, 0.0);
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
