use super::*;

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

/// The app-wide `[agent]` transcript keys used to apply to every entry that
/// stated nothing. Dropping that layer without lifting them would have silently
/// stopped a hand-set value from working.
#[test]
fn legacy_app_wide_transcript_keys_lift_onto_entries_that_state_nothing() {
    let input = "\
[agent]\n\
fold_mode = [\"expanded\"]\n\
tail_window = 3\n\
display_filter = [\"prose\"]\n\
[[agents]]\n\
id = \"states-nothing\"\n\
name = \"States Nothing\"\n\
command = \"npx -y some-acp\"\n\
[[agents]]\n\
id = \"states-fold\"\n\
name = \"States Fold\"\n\
command = \"npx -y some-acp\"\n\
fold_mode = [\"summary\"]\n";
    let mut cfg: Config = toml::from_str(input).unwrap();
    cfg.clamp();

    let resolved = cfg.resolved_agents();
    assert_eq!(
        resolved[0].fold_mode.as_deref(),
        Some(["expanded".to_string()].as_slice())
    );
    assert_eq!(resolved[0].tail_window, Some(3));
    assert_eq!(
        resolved[0].display_filter.as_deref(),
        Some(["prose".to_string()].as_slice())
    );

    // Per axis: the entry that already states fold keeps its own, and still
    // takes the two it says nothing about.
    assert_eq!(
        resolved[1].fold_mode.as_deref(),
        Some(["summary".to_string()].as_slice())
    );
    assert_eq!(resolved[1].tail_window, Some(3));
    // The legacy section never had a call-level key, so the lift cannot invent
    // one: the level the old value meant is the step level, and the call level
    // stays unstated and follows the built-in.
    assert!(resolved.iter().all(|a| a.tail_window_calls.is_none()));
}
