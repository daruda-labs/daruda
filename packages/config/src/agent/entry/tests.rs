use super::*;

/// The `codex-acp` preset — a registry entry daruda can launch as-is.
fn codex_preset() -> AgentDefinition {
    AgentDefinition::registry_preset("codex-acp").expect("codex-acp is runnable")
}

fn custom(id: &str, command: &str) -> AgentDefinition {
    AgentDefinition::new(
        id.to_string(),
        id.to_string(),
        AgentLaunch::Raw(command.to_string()),
    )
}

#[test]
fn a_bare_preset_reference_resolves_to_the_preset() {
    let entry: AgentEntry = toml::from_str("preset = \"codex-acp\"").expect("deserialize");
    assert_eq!(entry, AgentEntry::preset("codex-acp".to_string()));
    assert_eq!(entry.resolve(), Some(codex_preset()));
}

#[test]
fn overrides_replace_preset_fields_and_leave_the_rest_tracking() {
    let entry: AgentEntry = toml::from_str(
        "preset = \"gemini\"\n\
         command = \"npx -y @google/gemini-cli@0.9.0 --acp\"\n\
         default_mode = \"plan\"\n\
         default_model = \"gemini-2.5-pro\"\n",
    )
    .expect("deserialize");
    let resolved = entry.resolve().expect("gemini is a known preset");
    let preset = AgentDefinition::registry_preset("gemini").expect("gemini is runnable");
    assert_eq!(
        resolved.launch,
        AgentLaunch::Raw("npx -y @google/gemini-cli@0.9.0 --acp".to_string())
    );
    assert_eq!(resolved.default_mode.as_deref(), Some("plan"));
    assert_eq!(resolved.default_model.as_deref(), Some("gemini-2.5-pro"));
    // Untouched fields still come from the preset.
    assert_eq!(resolved.id, preset.id);
    assert_eq!(resolved.name, preset.name);
}

#[test]
fn a_reference_and_its_overrides_round_trip_through_toml() {
    for entry in [
        AgentEntry::preset("codex-acp".to_string()),
        AgentEntry::preset_with(
            "gemini".to_string(),
            PresetOverrides {
                name: Some("Gemini (pinned)".to_string()),
                command: Some("npx -y @google/gemini-cli@0.9.0 --acp".to_string()),
                default_mode: Some("plan".to_string()),
                default_model: Some("gemini-2.5-pro".to_string()),
                ..PresetOverrides::default()
            },
        ),
        AgentEntry::custom(custom("hermes", "hermes acp")),
        AgentEntry::custom(AgentDefinition {
            default_mode: Some("plan".to_string()),
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
    ] {
        let toml_str = toml::to_string(&entry).expect("serialize");
        let back: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, entry, "{toml_str}");
    }
}

#[test]
fn a_bare_reference_writes_only_the_preset_key() {
    let toml_str =
        toml::to_string(&AgentEntry::preset("codex-acp".to_string())).expect("serialize");
    assert_eq!(toml_str, "preset = \"codex-acp\"\n");
}

#[test]
fn a_flat_legacy_entry_stays_a_custom_definition() {
    let entry: AgentEntry =
        toml::from_str("id = \"hermes\"\nname = \"Hermes Agent\"\ncommand = \"hermes acp\"\n")
            .expect("deserialize");
    let AgentSource::Custom(definition) = &entry.source else {
        panic!("no preset carries the id `hermes`, so it cannot be a reference: {entry:?}");
    };
    assert_eq!(definition.id, "hermes");
    assert_eq!(
        definition.launch,
        AgentLaunch::Raw("hermes acp".to_string())
    );
}

#[test]
fn an_exact_copy_of_a_preset_is_promoted_to_a_reference() {
    // A flat copy of a preset — its own id, name and command — as older
    // configs hold it. Promoting it on load is what makes the reference
    // model apply to existing configs without a migration.
    let preset = codex_preset();
    let AgentLaunch::Raw(command) = &preset.launch else {
        panic!("presets launch Raw");
    };
    let toml_str = format!(
        "id = \"{}\"\nname = \"{}\"\ncommand = \"{}\"\n",
        preset.id, preset.name, command
    );
    let entry: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(entry, AgentEntry::preset("codex-acp".to_string()));
    assert_eq!(entry.resolve(), Some(preset));
}

#[test]
fn a_renamed_copy_is_promoted_with_the_rename_kept_as_an_override() {
    let preset = codex_preset();
    let AgentLaunch::Raw(command) = &preset.launch else {
        panic!("presets launch Raw");
    };
    let toml_str = format!(
        "id = \"{}\"\nname = \"My Codex\"\ncommand = \"{}\"\ndefault_mode = \"plan\"\n",
        preset.id, command
    );
    let entry: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "codex-acp".to_string(),
            PresetOverrides {
                name: Some("My Codex".to_string()),
                default_mode: Some("plan".to_string()),
                ..PresetOverrides::default()
            }
        )
    );
}

#[test]
fn a_default_model_override_is_promoted_and_resolves_back() {
    let preset = codex_preset();
    let AgentLaunch::Raw(command) = &preset.launch else {
        panic!("presets launch Raw");
    };
    let toml_str = format!(
        "id = \"{}\"\nname = \"{}\"\ncommand = \"{}\"\ndefault_model = \"gpt-5-codex\"\n",
        preset.id, preset.name, command
    );
    let entry: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "codex-acp".to_string(),
            PresetOverrides {
                default_model: Some("gpt-5-codex".to_string()),
                ..PresetOverrides::default()
            }
        )
    );
    let resolved = entry.resolve().expect("codex-acp is runnable");
    assert_eq!(resolved.default_model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(resolved.id, preset.id);
}

#[test]
fn a_differing_command_blocks_promotion_and_a_differing_id_is_kept() {
    let preset = codex_preset();
    // Same id, pinned command: the user is running a specific build, so the
    // entry must not start tracking the preset's `@latest`.
    let pinned = AgentDefinition {
        launch: AgentLaunch::Raw("npx -y @agentclientprotocol/codex-acp@1.1.0".to_string()),
        ..preset.clone()
    };
    assert_eq!(
        AgentEntry::for_definition(pinned.clone(), None),
        AgentEntry::custom(pinned)
    );
    // Same command, different id: still that preset, and the id — what
    // panes persist — survives as an override.
    let renamed_id = AgentDefinition {
        id: "my-codex".to_string(),
        ..preset
    };
    let entry = AgentEntry::for_definition(renamed_id.clone(), None);
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "codex-acp".to_string(),
            PresetOverrides {
                id: Some("my-codex".to_string()),
                ..PresetOverrides::default()
            }
        )
    );
    assert_eq!(entry.resolve(), Some(renamed_id));
}

/// Regression: `claude_default()` shares its command with the `claude-acp`
/// preset but keeps the id `claude`, which every AgentChat pane persists in
/// `SerializedAgentChat.agent_id`. It becomes a reference to that preset
/// and must still resolve under `claude`, or every existing pane's restore
/// breaks.
#[test]
fn the_built_in_claude_default_references_its_preset_under_its_own_id() {
    let claude = AgentDefinition::claude_default();
    let preset = AgentDefinition::registry_preset("claude-acp").expect("claude-acp is runnable");
    assert_eq!(claude.launch, preset.launch, "same adapter command");
    assert_ne!(claude.id, preset.id, "different stable id");

    let entry = AgentEntry::for_definition(claude.clone(), None);
    assert_eq!(entry.preset_id(), Some("claude-acp"));
    assert_eq!(entry.resolve(), Some(claude));
}

#[test]
fn a_known_origin_keeps_the_reference_when_a_field_was_edited() {
    // The Settings catalog editing a row that already referenced `gemini`:
    // the edited command becomes an override instead of detaching the row
    // into a frozen copy.
    let mut edited = AgentDefinition::registry_preset("gemini").expect("gemini is runnable");
    edited.launch = AgentLaunch::Raw("npx -y @google/gemini-cli@0.9.0 --acp".to_string());
    let entry = AgentEntry::for_definition(edited.clone(), Some("gemini"));
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "gemini".to_string(),
            PresetOverrides {
                command: Some("npx -y @google/gemini-cli@0.9.0 --acp".to_string()),
                ..PresetOverrides::default()
            }
        )
    );
    assert_eq!(entry.resolve(), Some(edited));
}

#[test]
fn a_known_origin_keeps_an_edited_id_and_detaches_for_a_remote_transport() {
    let preset = AgentDefinition::registry_preset("gemini").expect("gemini is runnable");
    // Id edited away from the preset's: the origin still holds, and the
    // new id is carried as an override.
    let renamed = AgentDefinition {
        id: "my-gemini".to_string(),
        ..preset.clone()
    };
    let entry = AgentEntry::for_definition(renamed.clone(), Some("gemini"));
    assert_eq!(entry.preset_id(), Some("gemini"));
    assert_eq!(entry.resolve(), Some(renamed));
    // Switched to a remote transport: no override can express that.
    let remote = AgentDefinition {
        launch: AgentLaunch::Ssh {
            adapter_command: "npx -y @google/gemini-cli@latest --acp".to_string(),
            host: "vm".to_string(),
        },
        ..preset
    };
    assert_eq!(
        AgentEntry::for_definition(remote.clone(), Some("gemini")),
        AgentEntry::custom(remote)
    );
}

#[test]
fn an_unknown_preset_stays_in_place_and_resolves_to_nothing() {
    let entry: AgentEntry = toml::from_str("preset = \"retired-agent\"").expect("deserialize");
    assert_eq!(entry.preset_id(), Some("retired-agent"));
    assert_eq!(entry.resolve(), None);
    // And it survives a write: nothing prunes an entry daruda cannot resolve.
    let toml_str = toml::to_string(&entry).expect("serialize");
    assert_eq!(
        toml::from_str::<AgentEntry>(&toml_str).expect("deserialize"),
        entry
    );
}

#[test]
fn a_preset_row_that_also_needs_a_manual_install_resolves_to_nothing() {
    // `cursor` is in the preset table but ships only binary archives, so it
    // has no launch command — same outcome as an unknown id.
    let entry = AgentEntry::preset("cursor".to_string());
    assert_eq!(entry.resolve(), None);
}

/// A row follows its preset's `env` until it states one of its own, and
/// the override survives the promotion path Settings saves through.
#[test]
fn a_row_follows_its_presets_env_until_it_states_one() {
    let preset = codex_preset();
    let follower = AgentEntry::preset("codex-acp".to_string());
    assert_eq!(
        follower.resolve().expect("codex-acp is runnable").env,
        preset.env,
        "a bare reference follows the preset"
    );

    // Deliberately differs from the preset's own env default (which turns
    // `multi_agent_v2` on) — a user explicitly opting back out.
    let stated = AgentDefinition {
        env: Some(vec![(
            "CODEX_CONFIG".to_string(),
            r#"{"features":{"multi_agent_v2":false}}"#.to_string(),
        )]),
        ..preset.clone()
    };
    let entry = AgentEntry::for_definition(stated.clone(), Some("codex-acp"));
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "codex-acp".to_string(),
            PresetOverrides {
                env: stated.env.clone(),
                ..PresetOverrides::default()
            }
        )
    );
    assert_eq!(entry.resolve(), Some(stated));
}

/// A pre-`env`, flat `[[agents]]` row for `codex-acp` (`id`/`name`/`command`
/// only, no `preset` key, no `env` key) must still promote to a bare
/// preset reference — not a `Custom` copy that pins the empty env it
/// happened to be read with. Pinning it would freeze the row out of the
/// preset's own `multi_agent_v2` default forever, exactly what the `env`
/// diff in `reference` is meant to prevent.
#[test]
fn a_legacy_flat_codex_row_picks_up_the_presets_env_default() {
    let preset = codex_preset();
    let legacy = AgentDefinition {
        env: None,
        ..preset.clone()
    };
    let entry = AgentEntry::for_definition(legacy, None);
    assert_eq!(
        entry,
        AgentEntry::preset("codex-acp".to_string()),
        "a flat row with no env key must promote to a bare reference"
    );
    assert_eq!(
        entry.resolve().expect("codex-acp is runnable").env,
        preset.env,
        "the promoted row must carry the preset's env, not stay empty"
    );
    assert!(
        entry
            .resolve()
            .unwrap()
            .env
            .is_some_and(|env| !env.is_empty())
    );
}

/// The hand-written form of the same row: an `[env]` table under a
/// `preset` key overrides only that field.
#[test]
fn an_env_table_on_a_preset_row_overrides_only_env() {
    let entry: AgentEntry =
        toml::from_str("preset = \"codex-acp\"\n[env]\nCODEX_CONFIG = \"{}\"\n")
            .expect("deserialize");
    let resolved = entry.resolve().expect("codex-acp is runnable");
    assert_eq!(
        resolved.env,
        Some(vec![("CODEX_CONFIG".to_string(), "{}".to_string())])
    );
    let preset = codex_preset();
    assert_eq!(resolved.id, preset.id);
    assert_eq!(resolved.launch, preset.launch);
}

/// Clearing a preset-shipping environment must survive the save path:
/// the row states an environment of its own — an empty one — and must not
/// fall back to following the preset again. This is the state the plain
/// `Vec` could not express.
#[test]
fn an_env_cleared_against_a_shipping_preset_stays_cleared() {
    let cleared = AgentDefinition {
        env: Some(Vec::new()),
        ..codex_preset()
    };
    let entry = AgentEntry::for_definition(cleared.clone(), Some("codex-acp"));
    assert_eq!(
        entry,
        AgentEntry::preset_with(
            "codex-acp".to_string(),
            PresetOverrides {
                env: Some(Vec::new()),
                ..PresetOverrides::default()
            }
        ),
        "clearing a shipping env is an override, not a return to the preset"
    );
    assert_eq!(entry.resolve(), Some(cleared));
}

/// The three `env` states, end to end through TOML: no key at all, an
/// empty table, and a stated one. The middle case is the whole point of
/// the distinction — on disk, the empty table is all that separates it
/// from the first.
#[test]
fn every_env_state_round_trips_through_toml() {
    let preset = codex_preset();
    for (entry, expected) in [
        (
            AgentEntry::preset("codex-acp".to_string()),
            preset.env.clone(),
        ),
        (
            AgentEntry::preset_with(
                "codex-acp".to_string(),
                PresetOverrides {
                    env: Some(Vec::new()),
                    ..PresetOverrides::default()
                },
            ),
            Some(Vec::new()),
        ),
        (
            AgentEntry::preset_with(
                "codex-acp".to_string(),
                PresetOverrides {
                    env: Some(vec![("K".to_string(), "v".to_string())]),
                    ..PresetOverrides::default()
                },
            ),
            Some(vec![("K".to_string(), "v".to_string())]),
        ),
        (
            AgentEntry::custom(AgentDefinition {
                env: Some(Vec::new()),
                ..custom("hermes", "hermes acp")
            }),
            Some(Vec::new()),
        ),
    ] {
        let toml_str = toml::to_string(&entry).expect("serialize");
        let back: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, entry, "{toml_str}");
        assert_eq!(
            back.resolve().expect("runnable").env,
            expected,
            "{toml_str}"
        );
    }
}

/// A bare reference writes no `env` key at all, so a preset that gains an
/// env default later still reaches the row. The explicit-empty override
/// above writes the table that opts out of exactly that.
#[test]
fn an_unstated_env_writes_no_key_and_an_empty_one_writes_a_table() {
    let bare = toml::to_string(&AgentEntry::preset("codex-acp".to_string())).expect("serialize");
    assert!(!bare.contains("env"), "{bare}");

    let cleared = toml::to_string(&AgentEntry::preset_with(
        "codex-acp".to_string(),
        PresetOverrides {
            env: Some(Vec::new()),
            ..PresetOverrides::default()
        },
    ))
    .expect("serialize");
    assert!(cleared.contains("[env]"), "{cleared}");
}

#[test]
fn an_entry_with_neither_preset_nor_id_is_rejected_by_name() {
    let err = toml::from_str::<AgentEntry>("name = \"Nameless\"\n")
        .expect_err("an entry with no identity is not loadable");
    let message = err.to_string();
    assert!(message.contains("preset"), "{message}");
    assert!(message.contains("id"), "{message}");
}
