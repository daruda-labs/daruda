//! Public API tests for config agent catalog.

use daruda_config::*;

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

/// The axis's two levels are independent keys: an entry can state either, both
/// or neither, and each survives a write-and-reload on its own.
#[test]
fn the_two_tail_levels_round_trip_as_separate_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[[agents]]
         id = \"split\"\n         name = \"Split\"\n         command = \"npx -y some-acp\"\n         tail_window = 3\n         tail_window_calls = 10\n         [[agents]]
         id = \"calls-only\"\n         name = \"Calls Only\"\n         command = \"npx -y some-acp\"\n         tail_window_calls = 1\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    let agents = cfg.resolved_agents();
    assert_eq!(agents[0].tail_window, Some(3));
    assert_eq!(agents[0].tail_window_calls, Some(10));
    assert_eq!(
        agents[1].tail_window, None,
        "a row may state the call level alone"
    );
    assert_eq!(agents[1].tail_window_calls, Some(1));

    patch_config_file_to(&cfg, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("tail_window = 3"), "{text}");
    assert!(text.contains("tail_window_calls = 10"), "{text}");
    assert!(text.contains("tail_window_calls = 1"), "{text}");

    let reloaded = Config::load_from(&path).resolved_agents();
    assert_eq!(reloaded[0].tail_window, Some(3));
    assert_eq!(reloaded[0].tail_window_calls, Some(10));
    assert_eq!(reloaded[1].tail_window, None);
    assert_eq!(reloaded[1].tail_window_calls, Some(1));
}

/// The lifted keys are never written back under `[agent]`, so the migration
/// settles rather than re-running against a value the catalog now owns.
#[test]
fn legacy_app_wide_transcript_keys_are_not_written_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[agent]\nfold_mode = [\"expanded\"]\ntail_window = 3\n",
    )
    .unwrap();

    let cfg = Config::load_from(&path);
    assert_eq!(cfg.resolved_agents()[0].tail_window, Some(3));
    patch_config_file_to(&cfg, &path).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let agent_section = text
        .split("[[agents]]")
        .next()
        .expect("the [agent] section comes first");
    assert!(
        !agent_section.contains("fold_mode"),
        "the app-wide key must not be rewritten: {text}"
    );
    assert!(text.contains("tail_window = 3"), "{text}");
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

    let reloaded = apply_settings_patch_to(&SettingsPatch::TerminalFontSize(15.0), &path)
        .expect("patch applies");
    assert_eq!(reloaded.font.terminal.size, 15.0);
    assert_eq!(
        reloaded.resolved_agents()[0].default_mode.as_deref(),
        Some("acceptEdits")
    );

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("default_permission_mode"), "{text}");
    assert!(text.contains("default_mode = \"acceptEdits\""), "{text}");
}

/// The Settings save path patches in place, so it has to clear the legacy
/// transcript keys the same way it clears the legacy permission mode. Left
/// behind, they re-migrate on the next load and undo the edit that was just
/// saved — a reset to the built-in would appear to fail, every time.
#[test]
fn settings_patch_migrates_and_removes_stale_transcript_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[agent]\nfold_mode = [\"expanded\"]\ntail_window = 3\ndisplay_filter = [\"prose\"]\n",
    )
    .unwrap();

    let reloaded = apply_settings_patch_to(&SettingsPatch::TerminalFontSize(15.0), &path)
        .expect("patch applies");
    let agent = &reloaded.resolved_agents()[0];
    assert_eq!(
        agent.fold_mode.as_deref(),
        Some(["expanded".to_string()].as_slice())
    );
    assert_eq!(agent.tail_window, Some(3));

    let text = std::fs::read_to_string(&path).unwrap();
    let agent_section = text
        .split("[[agents]]")
        .next()
        .expect("[agent] comes first");
    for key in ["fold_mode", "tail_window", "display_filter"] {
        assert!(
            !agent_section.contains(key),
            "the app-wide `{key}` must not survive the patch: {text}"
        );
    }
    assert!(
        text.contains("tail_window = 3"),
        "lifted onto the entry: {text}"
    );
}

/// The consequence the removal above exists to prevent: an axis handed back to
/// the built-in stays handed back across a reload.
#[test]
fn a_transcript_axis_reset_after_the_legacy_lift_stays_reset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[agent]\nfold_mode = [\"expanded\"]\n").unwrap();

    let lifted =
        apply_settings_patch_to(&SettingsPatch::TerminalFontSize(15.0), &path).expect("patch");
    assert!(
        lifted.resolved_agents()[0].fold_mode.is_some(),
        "the legacy value reached the catalog"
    );

    // The user hands the axis back: the catalog states nothing on it.
    let mut agents = lifted.agents.clone();
    for entry in &mut agents {
        match &mut entry.source {
            AgentSource::Custom(definition) => definition.fold_mode = None,
            AgentSource::Preset { overrides, .. } => overrides.fold_mode = None,
        }
    }
    apply_settings_patch_to(&SettingsPatch::AgentCatalog(agents), &path).expect("catalog patch");

    let reloaded = Config::load_from(&path);
    assert_eq!(
        reloaded.resolved_agents()[0].fold_mode,
        None,
        "a stale [agent] key must not migrate the value back in"
    );
}

#[test]
fn patch_config_file_writes_non_default_agents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let cfg = Config {
        agents: vec![
            AgentEntry::for_definition(AgentDefinition::claude_default(), None),
            AgentEntry::custom(AgentDefinition::codex_default()),
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
            AgentEntry::for_definition(AgentDefinition::claude_default(), None),
            AgentEntry::preset("codex-acp".to_string()),
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
            AgentEntry::preset("codex-acp".to_string()),
            AgentEntry::preset_with(
                "gemini".to_string(),
                PresetOverrides {
                    env: Some(vec![("RUST_LOG".to_string(), "debug".to_string())]),
                    name: Some("Gemini (pinned)".to_string()),
                    command: Some("npx -y @google/gemini-cli@0.9.0 --acp".to_string()),
                    default_mode: Some("plan".to_string()),
                    default_model: Some("gemini-2.5-pro".to_string()),
                    fold_mode: Some(vec!["summary".to_string()]),
                    tail_window: Some(3),
                    // An empty visible set, not an absent key.
                    display_filter: Some(Vec::new()),
                    ..PresetOverrides::default()
                },
            ),
            AgentEntry::preset("retired-agent".to_string()),
            AgentEntry::custom(AgentDefinition {
                default_mode: Some("yolo".to_string()),
                fold_mode: Some(vec!["expanded".to_string()]),
                tail_window: Some(10),
                display_filter: Some(vec!["prose".to_string(), "tools".to_string()]),
                ..AgentDefinition::new(
                    "hermes".to_string(),
                    "Hermes Agent".to_string(),
                    AgentLaunch::Raw("hermes acp".to_string()),
                )
            }),
            AgentEntry::custom(AgentDefinition {
                // A table key, like the `ssh` sub-table below it — TOML
                // forbids a value after a table, so the writer has to emit
                // both after every scalar.
                env: Some(vec![(
                    "CODEX_CONFIG".to_string(),
                    r#"{"features":{"multi_agent_v2":true}}"#.to_string(),
                )]),
                default_model: Some("claude-opus-4".to_string()),
                ..AgentDefinition::new(
                    "remote".to_string(),
                    "Remote".to_string(),
                    AgentLaunch::Ssh {
                        adapter_command: "npx -y some-acp".to_string(),
                        host: "vm-work".to_string(),
                    },
                )
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
    assert!(on_disk.contains("display_filter = []"), "{on_disk}");
    assert!(on_disk.contains("RUST_LOG = \"debug\""), "{on_disk}");

    assert_eq!(Config::load_from(&path).agents, cfg.agents);
}

/// A row that clears its preset's environment states an empty table, which is
/// a different answer from stating nothing — the latter follows the preset.
/// The writer has to keep them apart on disk, or the clear is undone by the
/// next reload.
#[test]
fn patch_config_file_keeps_a_cleared_env_override_apart_from_an_absent_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let cleared = AgentEntry::preset_with(
        "codex-acp".to_string(),
        PresetOverrides {
            env: Some(Vec::new()),
            ..PresetOverrides::default()
        },
    );
    let cfg = Config {
        agents: vec![cleared.clone()],
        ..Config::default()
    };
    patch_config_file_to(&cfg, &path).unwrap();
    assert_eq!(Config::load_from(&path).agents, vec![cleared]);
}

/// Switching an entry off keeps every field it states, on both write paths —
/// and only the switched-off one says so on disk, so a file written before
/// the key existed reads back with every entry on.
#[test]
fn a_switched_off_entry_round_trips_with_its_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let off = AgentEntry::preset_with(
        "gemini".to_string(),
        PresetOverrides {
            default_mode: Some("plan".to_string()),
            ..PresetOverrides::default()
        },
    )
    .disabled();
    let cfg = Config {
        agents: vec![AgentEntry::preset("codex-acp".to_string()), off.clone()],
        ..Config::default()
    };
    patch_config_file_to(&cfg, &path).unwrap();

    let on_disk: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let written: Vec<Option<&toml::Value>> = on_disk["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.get("enabled"))
        .collect();
    assert_eq!(written, [None, Some(&toml::Value::Boolean(false))]);
    assert_eq!(Config::load_from(&path).agents, cfg.agents);

    let serde_round_trip: AgentEntry = toml::from_str(&toml::to_string(&off).unwrap()).unwrap();
    assert_eq!(serde_round_trip, off);
    let legacy: AgentEntry = toml::from_str("preset = \"gemini\"").unwrap();
    assert!(legacy.enabled);
}

#[test]
fn resolved_agents_skips_a_switched_off_entry() {
    let cfg = Config {
        agents: vec![
            AgentEntry::preset("gemini".to_string()).disabled(),
            AgentEntry::preset("codex-acp".to_string()),
        ],
        ..Config::default()
    };
    let ids: Vec<String> = cfg.resolved_agents().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, ["codex-acp"]);
}

/// The catalog shape a pre-reference install carries: the built-in Claude
/// written out as a custom entry, next to a preset reference. Loading has to
/// recognise that entry as the `claude-acp` preset without touching the id
/// every pane persists, and a save has to keep it that way on disk.
#[test]
fn a_custom_entry_running_a_presets_command_loads_as_that_preset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[[agents]]\n\
         id = \"claude\"\n\
         name = \"Claude Code\"\n\
         command = \"npx -y @agentclientprotocol/claude-agent-acp@latest\"\n\
         default_mode = \"bypassPermissions\"\n\
         tail_window = 5\n\
         \n\
         [[agents]]\n\
         preset = \"codex-acp\"\n",
    )
    .unwrap();

    let loaded = Config::load_from(&path);
    assert_eq!(
        loaded.agents[0],
        AgentEntry::preset_with(
            "claude-acp",
            PresetOverrides {
                id: Some("claude".to_string()),
                name: Some("Claude Code".to_string()),
                default_mode: Some("bypassPermissions".to_string()),
                tail_window: Some(5),
                ..PresetOverrides::default()
            }
        )
    );
    let ids: Vec<String> = loaded.resolved_agents().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, ["claude", "codex-acp"]);

    patch_config_file_to(&loaded, &path).unwrap();
    let on_disk: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let first = &on_disk["agents"][0];
    assert_eq!(first["preset"].as_str(), Some("claude-acp"));
    assert_eq!(first["id"].as_str(), Some("claude"));
    assert!(
        first.get("command").is_none(),
        "a reference copies no command"
    );
    assert_eq!(Config::load_from(&path).agents, loaded.agents);
}

/// A custom entry that states no environment and is promoted to a preset
/// reference adopts that preset's environment — `None` means "follow the
/// preset", which is how codex rows pick up the native-subagent overlay.
#[test]
fn a_promoted_entry_without_an_environment_takes_the_presets() {
    let entry: AgentEntry = toml::from_str(
        "id = \"my-codex\"\nname = \"My Codex\"\n\
         command = \"npx -y @agentclientprotocol/codex-acp@latest\"\n",
    )
    .unwrap();
    assert_eq!(entry.preset_id(), Some("codex-acp"));
    let env = entry.resolve().and_then(|d| d.env).unwrap_or_default();
    assert!(
        env.iter().any(|(name, _)| name == CODEX_CONFIG_ENV),
        "{env:?}"
    );
}

/// A pinned build or a remote launch is not the preset, whatever its id.
#[test]
fn a_custom_entry_that_differs_from_every_preset_stays_custom() {
    let pinned: AgentEntry = toml::from_str(
        "id = \"claude\"\nname = \"Claude Code\"\n\
         command = \"npx -y @agentclientprotocol/claude-agent-acp@0.84.0\"\n",
    )
    .unwrap();
    assert_eq!(pinned.preset_id(), None);
    let remote: AgentEntry = toml::from_str(
        "id = \"claude\"\nname = \"Claude Code\"\n\
         [ssh]\nhost = \"vm\"\n\
         adapter_command = \"npx -y @agentclientprotocol/claude-agent-acp@latest\"\n",
    )
    .unwrap();
    assert_eq!(remote.preset_id(), None);
}

/// Settings keeps one entry on; a hand-edited file that switches every entry
/// off still gets the non-empty catalog every consumer relies on.
#[test]
fn a_catalog_with_every_entry_off_falls_back_to_the_default() {
    let cfg = Config {
        agents: vec![AgentEntry::preset("codex-acp".to_string()).disabled()],
        ..Config::default()
    };
    assert_eq!(
        cfg.resolved_agents(),
        vec![AgentDefinition::claude_default()]
    );
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
        vec![AgentEntry::for_definition(
            AgentDefinition::claude_default(),
            None
        )]
    );
}
