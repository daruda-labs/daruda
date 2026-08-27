//! One `[[agents]]` catalog row as it is persisted: either a reference to a
//! built-in preset (plus per-field overrides) or a self-contained definition.
//!
//! Storing a reference rather than a copy is what lets an upstream preset
//! change — a new adapter command, a renamed agent — reach the user without a
//! config edit. A field the user did override stays theirs.

use serde::{Deserialize, Serialize};

use super::{AgentDefinition, AgentDefinitionRepr, AgentLaunch, DockerLaunchRepr, SshLaunchRepr};

/// A persisted catalog entry. The `preset` key decides the shape, so the two
/// are mutually exclusive by construction: a reference cannot also carry its
/// own `id`, and a custom entry has no preset to track.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "AgentEntryRepr", into = "AgentEntryRepr")]
pub enum AgentEntry {
    /// References the built-in preset `preset`; every field not in `overrides`
    /// resolves from it. An entry naming a preset daruda cannot launch is kept
    /// as it is and simply skipped by [`crate::Config::resolved_agents`].
    Preset {
        preset: String,
        overrides: PresetOverrides,
    },
    /// Carries the whole definition — a hand-written entry, or one whose fields
    /// no longer match any preset.
    Custom(AgentDefinition),
}

/// Per-field overrides on a preset reference. `None` means "follow the preset".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetOverrides {
    pub name: Option<String>,
    /// Replaces the preset's launch command. A plain command line, never a
    /// remote transport: every preset is [`AgentLaunch::Raw`], and reaching an
    /// adapter over ssh/docker describes a different agent rather than an
    /// override of this one — that is a [`AgentEntry::Custom`] entry.
    pub command: Option<String>,
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
}

impl AgentEntry {
    /// The runnable definition this entry stands for, or `None` when the preset
    /// it references carries no launch command — an id daruda no longer knows,
    /// or one that still needs a manual install.
    pub fn resolve(&self) -> Option<AgentDefinition> {
        match self {
            AgentEntry::Custom(definition) => Some(definition.clone()),
            AgentEntry::Preset { preset, overrides } => {
                let mut definition = AgentDefinition::registry_preset(preset)?;
                if let Some(name) = &overrides.name {
                    definition.name = name.clone();
                }
                if let Some(command) = &overrides.command {
                    definition.launch = AgentLaunch::Raw(command.clone());
                }
                if let Some(default_mode) = &overrides.default_mode {
                    definition.default_mode = Some(default_mode.clone());
                }
                if let Some(default_model) = &overrides.default_model {
                    definition.default_model = Some(default_model.clone());
                }
                Some(definition)
            }
        }
    }

    /// The preset this entry references, or `None` for a custom entry.
    pub fn preset_id(&self) -> Option<&str> {
        match self {
            AgentEntry::Preset { preset, .. } => Some(preset),
            AgentEntry::Custom(_) => None,
        }
    }

    /// The entry that persists `definition`.
    ///
    /// `preset` is the preset the definition was resolved from, when the caller
    /// knows it (an editor working on an already-referencing row): the entry
    /// keeps referencing it and each edited field becomes an override.
    ///
    /// With `None` — a hand-written config row, or a definition of unknown
    /// origin — the entry becomes a reference only on an *exact* match: same id
    /// and same launch command. A resembling-but-different command stays a
    /// custom copy, since silently retargeting it at the preset would swap the
    /// command the user is running.
    pub fn for_definition(definition: AgentDefinition, preset: Option<&str>) -> Self {
        if let Some(entry) = preset.and_then(|preset| Self::reference(preset, &definition)) {
            return entry;
        }
        // Promotion of an unattributed definition: `reference` already pins the
        // id, and rejecting a command override pins the command — the two
        // conditions that make the promotion invisible to everything
        // downstream, including the per-pane persisted `agent_id`.
        match Self::reference(&definition.id, &definition) {
            Some(entry) if !entry.overrides_command() => entry,
            _ => Self::Custom(definition),
        }
    }

    /// Whether this entry replaces its preset's launch command.
    fn overrides_command(&self) -> bool {
        matches!(self, AgentEntry::Preset { overrides, .. } if overrides.command.is_some())
    }

    /// A reference to `preset` carrying whatever `definition` states
    /// differently, or `None` when that reference could not stand in for
    /// `definition`: no such preset, a different id (an entry's id *is* its
    /// preset's id — overriding it would change the identity panes persist), or
    /// a difference no override can express.
    fn reference(preset: &str, definition: &AgentDefinition) -> Option<Self> {
        let base = AgentDefinition::registry_preset(preset)?;
        if base.id != definition.id {
            return None;
        }
        let entry = Self::Preset {
            preset: preset.to_string(),
            overrides: PresetOverrides {
                name: differing(&base.name, &definition.name),
                command: match (&base.launch, &definition.launch) {
                    (AgentLaunch::Raw(base), AgentLaunch::Raw(command)) => differing(base, command),
                    // A remote transport is not expressible as an override.
                    _ => return None,
                },
                default_mode: definition.default_mode.clone(),
                default_model: definition.default_model.clone(),
            },
        };
        // The reference stands in for `definition` only if it resolves back to
        // it exactly — the one check that keeps every promotion lossless as
        // `PresetOverrides` and the preset table evolve independently.
        (entry.resolve().as_ref() == Some(definition)).then_some(entry)
    }
}

/// `Some(value)` when it differs from `base`, else `None` (follow `base`).
fn differing(base: &str, value: &str) -> Option<String> {
    (base != value).then(|| value.to_string())
}

/// Private wire representation for [`AgentEntry`]. Every key is optional and
/// `preset` alone selects the variant — a plain struct rather than
/// `#[serde(untagged)]`, whose failure message names none of the keys the user
/// actually got wrong.
///
/// Scalars precede the `ssh` / `docker` sub-tables because TOML forbids a
/// value after a table within the same entry.
#[derive(Serialize, Deserialize)]
struct AgentEntryRepr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh: Option<SshLaunchRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    docker: Option<DockerLaunchRepr>,
}

impl TryFrom<AgentEntryRepr> for AgentEntry {
    type Error = String;

    fn try_from(v: AgentEntryRepr) -> Result<Self, Self::Error> {
        if let Some(preset) = v.preset {
            return Ok(Self::Preset {
                preset,
                overrides: PresetOverrides {
                    name: v.name,
                    command: v.command,
                    default_mode: v.default_mode,
                    default_model: v.default_model,
                },
            });
        }
        let (Some(id), Some(name)) = (v.id, v.name) else {
            return Err(
                "an [[agents]] entry needs either `preset`, or both `id` and `name`".to_string(),
            );
        };
        let definition = AgentDefinition::from(AgentDefinitionRepr {
            id,
            name,
            command: v.command,
            ssh: v.ssh,
            docker: v.docker,
            default_mode: v.default_mode,
            default_model: v.default_model,
        });
        Ok(Self::for_definition(definition, None))
    }
}

impl From<AgentEntry> for AgentEntryRepr {
    fn from(v: AgentEntry) -> Self {
        match v {
            AgentEntry::Preset { preset, overrides } => Self {
                preset: Some(preset),
                id: None,
                name: overrides.name,
                command: overrides.command,
                default_mode: overrides.default_mode,
                default_model: overrides.default_model,
                ssh: None,
                docker: None,
            },
            AgentEntry::Custom(definition) => {
                let repr = AgentDefinitionRepr::from(definition);
                Self {
                    preset: None,
                    id: Some(repr.id),
                    name: Some(repr.name),
                    command: repr.command,
                    default_mode: repr.default_mode,
                    default_model: repr.default_model,
                    ssh: repr.ssh,
                    docker: repr.docker,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `codex-acp` preset — a registry entry daruda can launch as-is.
    fn codex_preset() -> AgentDefinition {
        AgentDefinition::registry_preset("codex-acp").expect("codex-acp is runnable")
    }

    fn custom(id: &str, command: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            name: id.to_string(),
            launch: AgentLaunch::Raw(command.to_string()),
            default_mode: None,
            default_model: None,
        }
    }

    #[test]
    fn a_bare_preset_reference_resolves_to_the_preset() {
        let entry: AgentEntry = toml::from_str("preset = \"codex-acp\"").expect("deserialize");
        assert_eq!(
            entry,
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides::default(),
            }
        );
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
            AgentEntry::Custom(custom("hermes", "hermes acp")),
            AgentEntry::Custom(AgentDefinition {
                id: "remote".to_string(),
                name: "Remote".to_string(),
                launch: AgentLaunch::Ssh {
                    adapter_command: "npx -y some-acp".to_string(),
                    host: "vm-work".to_string(),
                },
                default_mode: Some("plan".to_string()),
                default_model: Some("claude-opus-4".to_string()),
            }),
        ] {
            let toml_str = toml::to_string(&entry).expect("serialize");
            let back: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
            assert_eq!(back, entry, "{toml_str}");
        }
    }

    #[test]
    fn a_bare_reference_writes_only_the_preset_key() {
        let toml_str = toml::to_string(&AgentEntry::Preset {
            preset: "codex-acp".to_string(),
            overrides: PresetOverrides::default(),
        })
        .expect("serialize");
        assert_eq!(toml_str, "preset = \"codex-acp\"\n");
    }

    #[test]
    fn a_flat_legacy_entry_stays_a_custom_definition() {
        let entry: AgentEntry =
            toml::from_str("id = \"hermes\"\nname = \"Hermes Agent\"\ncommand = \"hermes acp\"\n")
                .expect("deserialize");
        let AgentEntry::Custom(definition) = &entry else {
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
        // What `Add Preset` used to write into config.toml: the preset's own id,
        // name and command, copied flat. Promoting it on load is what makes the
        // reference model apply to existing configs without a migration.
        let preset = codex_preset();
        let AgentLaunch::Raw(command) = &preset.launch else {
            panic!("presets launch Raw");
        };
        let toml_str = format!(
            "id = \"{}\"\nname = \"{}\"\ncommand = \"{}\"\n",
            preset.id, preset.name, command
        );
        let entry: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(
            entry,
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides::default(),
            }
        );
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
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides {
                    name: Some("My Codex".to_string()),
                    command: None,
                    default_mode: Some("plan".to_string()),
                    default_model: None,
                },
            }
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
            AgentEntry::Preset {
                preset: "codex-acp".to_string(),
                overrides: PresetOverrides {
                    name: None,
                    command: None,
                    default_mode: None,
                    default_model: Some("gpt-5-codex".to_string()),
                },
            }
        );
        let resolved = entry.resolve().expect("codex-acp is runnable");
        assert_eq!(resolved.default_model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(resolved.id, preset.id);
    }

    #[test]
    fn a_differing_command_or_id_blocks_promotion() {
        let preset = codex_preset();
        // Same id, pinned command: the user is running a specific build, so the
        // entry must not start tracking the preset's `@latest`.
        let pinned = AgentDefinition {
            launch: AgentLaunch::Raw("npx -y @agentclientprotocol/codex-acp@1.1.0".to_string()),
            ..preset.clone()
        };
        assert_eq!(
            AgentEntry::for_definition(pinned.clone(), None),
            AgentEntry::Custom(pinned)
        );
        // Same command, different id: the id is what panes persist, so it wins.
        let renamed_id = AgentDefinition {
            id: "my-codex".to_string(),
            ..preset
        };
        assert_eq!(
            AgentEntry::for_definition(renamed_id.clone(), None),
            AgentEntry::Custom(renamed_id)
        );
    }

    /// Regression: `claude_default()` shares its command with the `claude-acp`
    /// preset but keeps the id `claude`, which every AgentChat pane persists in
    /// `SerializedAgentChat.agent_id`. Promoting on the command alone would
    /// resolve that entry as `claude-acp` and break the restore of every
    /// existing pane.
    #[test]
    fn the_built_in_claude_default_is_never_promoted() {
        let claude = AgentDefinition::claude_default();
        let preset =
            AgentDefinition::registry_preset("claude-acp").expect("claude-acp is runnable");
        assert_eq!(claude.launch, preset.launch, "same adapter command");
        assert_ne!(claude.id, preset.id, "different stable id");

        let entry = AgentEntry::for_definition(claude.clone(), None);
        assert_eq!(entry, AgentEntry::Custom(claude.clone()));
        assert_eq!(entry.resolve().map(|d| d.id), Some(claude.id));
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
            AgentEntry::Preset {
                preset: "gemini".to_string(),
                overrides: PresetOverrides {
                    name: None,
                    command: Some("npx -y @google/gemini-cli@0.9.0 --acp".to_string()),
                    default_mode: None,
                    default_model: None,
                },
            }
        );
        assert_eq!(entry.resolve(), Some(edited));
    }

    #[test]
    fn a_known_origin_detaches_when_the_id_or_transport_no_longer_fits() {
        let preset = AgentDefinition::registry_preset("gemini").expect("gemini is runnable");
        // Id edited away from the preset's: identity beats origin.
        let renamed = AgentDefinition {
            id: "my-gemini".to_string(),
            ..preset.clone()
        };
        assert_eq!(
            AgentEntry::for_definition(renamed.clone(), Some("gemini")),
            AgentEntry::Custom(renamed)
        );
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
            AgentEntry::Custom(remote)
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
        let entry = AgentEntry::Preset {
            preset: "cursor".to_string(),
            overrides: PresetOverrides::default(),
        };
        assert_eq!(entry.resolve(), None);
    }

    #[test]
    fn an_entry_with_neither_preset_nor_id_is_rejected_by_name() {
        let err = toml::from_str::<AgentEntry>("name = \"Nameless\"\n")
            .expect_err("an entry with no identity is not loadable");
        let message = err.to_string();
        assert!(message.contains("preset"), "{message}");
        assert!(message.contains("id"), "{message}");
    }
}
