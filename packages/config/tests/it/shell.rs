//! Public API tests for shell.

use daruda_config::shell::*;

#[test]
fn default_closes_on_exit() {
    assert!(ShellConfig::default().close_pane_on_exit);
}

#[test]
fn parses_disabled() {
    let toml = "close_pane_on_exit = false";
    let cfg: ShellConfig = toml::from_str(toml).unwrap();
    assert!(!cfg.close_pane_on_exit);
}

#[test]
fn parses_empty_to_default() {
    let cfg: ShellConfig = toml::from_str("").unwrap();
    assert!(cfg.close_pane_on_exit);
    assert!(cfg.program.is_none());
}

#[test]
fn natural_text_editing_defaults_on() {
    assert!(ShellConfig::default().natural_text_editing);
}

#[test]
fn parses_natural_text_editing_disabled() {
    let toml = "natural_text_editing = false";
    let cfg: ShellConfig = toml::from_str(toml).unwrap();
    assert!(!cfg.natural_text_editing);
    // Other fields fall back to defaults.
    assert!(cfg.close_pane_on_exit);
}

#[test]
fn parses_program_override() {
    let toml = "program = \"/usr/local/bin/zsh\"";
    let cfg: ShellConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.program.as_deref(), Some("/usr/local/bin/zsh"));
    // Other fields fall back to defaults.
    assert!(cfg.close_pane_on_exit);
}
