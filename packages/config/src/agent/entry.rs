//! One `[[agents]]` catalog row as it is persisted: either a reference to a
//! built-in preset (plus per-field overrides) or a self-contained definition.
//!
//! Storing a reference rather than a copy is what lets an upstream preset
//! change — a new adapter command, a renamed agent — reach the user without a
//! config edit. A field the user did override stays theirs.

use serde::{Deserialize, Serialize};

use super::{
    AgentDefinition, AgentDefinitionRepr, AgentLaunch, DockerLaunchRepr, EnvRepr, SshLaunchRepr,
};

/// A persisted catalog entry: where its definition comes from, plus whether
/// the user has it switched on. The two are independent — any source can be
/// turned off, and turning one off keeps every field it states.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "AgentEntryRepr", into = "AgentEntryRepr")]
pub struct AgentEntry {
    /// `false` keeps the entry, and every override it carries, out of
    /// [`crate::Config::resolved_agents`].
    pub enabled: bool,
    pub source: AgentSource,
}

/// Where a catalog entry's definition comes from. The `preset` key decides
/// the shape: a reference follows its preset in every field it does not
/// override, and a custom entry has no preset to track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
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
    /// The agent id panes persist, when it is not the preset's own. Where an
    /// entry comes from and what it is called are separate facts: daruda's
    /// built-in Claude is the `claude-acp` preset under the id `claude`.
    pub id: Option<String>,
    pub name: Option<String>,
    /// Replaces the preset's launch command. A plain command line, never a
    /// remote transport: every preset is [`AgentLaunch::Raw`], and reaching an
    /// adapter over ssh/docker describes a different agent rather than an
    /// override of this one — that is a [`AgentSource::Custom`] entry.
    pub command: Option<String>,
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
    pub fold_mode: Option<Vec<String>>,
    pub tail_window: Option<u8>,
    pub tail_window_calls: Option<u8>,
    pub display_filter: Option<Vec<String>>,
    /// Replaces the preset's environment wholesale — an empty list is a real
    /// value (the preset's own environment, cleared), so it cannot collapse
    /// to "follow the preset". Same three states as
    /// [`AgentDefinition::env`], one level down.
    pub env: Option<Vec<(String, String)>>,
}

impl AgentEntry {
    /// A reference to preset `preset` that follows it in every field.
    pub fn preset(preset: impl Into<String>) -> Self {
        Self::preset_with(preset, PresetOverrides::default())
    }

    /// A reference to preset `preset` carrying `overrides`.
    pub fn preset_with(preset: impl Into<String>, overrides: PresetOverrides) -> Self {
        Self::enabled(AgentSource::Preset {
            preset: preset.into(),
            overrides,
        })
    }

    /// A self-contained entry for `definition`.
    pub fn custom(definition: AgentDefinition) -> Self {
        Self::enabled(AgentSource::Custom(definition))
    }

    fn enabled(source: AgentSource) -> Self {
        Self {
            enabled: true,
            source,
        }
    }

    /// The same entry, switched off.
    pub fn disabled(self) -> Self {
        Self {
            enabled: false,
            ..self
        }
    }

    /// The runnable definition this entry stands for, whether or not it is
    /// switched on — see [`AgentSource::resolve`].
    pub fn resolve(&self) -> Option<AgentDefinition> {
        self.source.resolve()
    }

    /// The preset this entry references, or `None` for a custom entry.
    pub fn preset_id(&self) -> Option<&str> {
        self.source.preset_id()
    }

    /// The entry that persists `definition`, switched on — see
    /// [`AgentSource::for_definition`].
    pub fn for_definition(definition: AgentDefinition, preset: Option<&str>) -> Self {
        Self::enabled(AgentSource::for_definition(definition, preset))
    }
}

impl AgentSource {
    /// The runnable definition this source stands for, or `None` when the
    /// preset it references carries no launch command — an id daruda no longer
    /// knows, or one that still needs a manual install.
    pub fn resolve(&self) -> Option<AgentDefinition> {
        match self {
            AgentSource::Custom(definition) => Some(definition.clone()),
            AgentSource::Preset { preset, overrides } => {
                let mut definition = AgentDefinition::registry_preset(preset)?;
                if let Some(id) = &overrides.id {
                    definition.id = id.clone();
                }
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
                if let Some(fold_mode) = &overrides.fold_mode {
                    definition.fold_mode = Some(fold_mode.clone());
                }
                if let Some(tail_window) = overrides.tail_window {
                    definition.tail_window = Some(tail_window);
                }
                if let Some(tail_window_calls) = overrides.tail_window_calls {
                    definition.tail_window_calls = Some(tail_window_calls);
                }
                if let Some(display_filter) = &overrides.display_filter {
                    definition.display_filter = Some(display_filter.clone());
                }
                if let Some(env) = &overrides.env {
                    definition.env = Some(env.clone());
                }
                Some(definition)
            }
        }
    }

    /// The preset this source references, or `None` for a custom one.
    pub fn preset_id(&self) -> Option<&str> {
        match self {
            AgentSource::Preset { preset, .. } => Some(preset),
            AgentSource::Custom(_) => None,
        }
    }

    /// The source that persists `definition`.
    ///
    /// `preset` is the preset the definition was resolved from, when the caller
    /// knows it (an editor working on an already-referencing row): the entry
    /// keeps referencing it and each edited field becomes an override.
    ///
    /// With `None` — a hand-written config row, or a definition of unknown
    /// origin — the source becomes a reference only on an *exact* match: a
    /// preset whose launch command it runs verbatim, its id first. A
    /// resembling-but-different command stays a custom copy, since silently
    /// retargeting it at the preset would swap the command the user is running.
    pub fn for_definition(definition: AgentDefinition, preset: Option<&str>) -> Self {
        if let Some(entry) = preset.and_then(|preset| Self::reference(preset, &definition)) {
            return entry;
        }
        // Promotion of an unattributed definition: `reference` keeps the id as
        // an override, and rejecting a command override pins the command — the
        // two conditions that make the promotion invisible to everything
        // downstream, including the per-pane persisted `agent_id`. The preset
        // the id names is tried first, so an id that is a preset id stays one.
        let named = std::iter::once(definition.id.clone());
        let others = super::preset::presets().map(|preset| preset.id.to_string());
        named
            .chain(others)
            .find_map(|preset| {
                Self::reference(&preset, &definition).filter(|source| !source.overrides_command())
            })
            .unwrap_or(Self::Custom(definition))
    }

    /// Whether this entry replaces its preset's launch command.
    fn overrides_command(&self) -> bool {
        matches!(self, AgentSource::Preset { overrides, .. } if overrides.command.is_some())
    }

    /// A reference to `preset` carrying whatever `definition` states
    /// differently, or `None` when that reference could not stand in for
    /// `definition`: no such preset, or a difference no override can express.
    fn reference(preset: &str, definition: &AgentDefinition) -> Option<Self> {
        let base = AgentDefinition::registry_preset(preset)?;
        // A definition that states no environment adopts the preset's — that
        // is what `None` means, and it is what lets a row written before its
        // preset gained an env default pick that default up. Resolved once,
        // here, so every field below is diffed and compared the same way.
        let definition = &AgentDefinition {
            env: definition.env.clone().or_else(|| base.env.clone()),
            ..definition.clone()
        };
        let entry = Self::Preset {
            preset: preset.to_string(),
            overrides: PresetOverrides {
                id: differing(&base.id, &definition.id),
                name: differing(&base.name, &definition.name),
                command: match (&base.launch, &definition.launch) {
                    (AgentLaunch::Raw(base), AgentLaunch::Raw(command)) => differing(base, command),
                    // A remote transport is not expressible as an override.
                    _ => return None,
                },
                // Copied whole rather than diffed against `base` like `name`
                // and `command`: every registry preset leaves these `None`, so
                // the value *is* the override.
                default_mode: definition.default_mode.clone(),
                default_model: definition.default_model.clone(),
                fold_mode: definition.fold_mode.clone(),
                tail_window: definition.tail_window,
                tail_window_calls: definition.tail_window_calls,
                display_filter: definition.display_filter.clone(),
                // Diffed like `name` and `command`, not copied whole: a
                // preset may state an environment of its own, and a row that
                // agrees with it has to keep following it. An empty one is a
                // difference like any other — the preset's environment,
                // cleared.
                env: definition
                    .env
                    .clone()
                    .filter(|env| base.env.as_ref() != Some(env)),
            },
        };
        // The reference stands in for `definition` only if it resolves back to
        // it — the check that keeps every promotion lossless as
        // `PresetOverrides` and the preset table evolve independently.
        let equivalent = entry.resolve().as_ref() == Some(definition);
        equivalent.then_some(entry)
    }
}

/// `Some(value)` when it differs from `base`, else `None` (follow `base`).
fn differing(base: &str, value: &str) -> Option<String> {
    (base != value).then(|| value.to_string())
}

/// Private wire representation for [`AgentEntry`]. Every key is optional and
/// `preset` alone selects the [`AgentSource`] variant — a plain struct rather than
/// `#[serde(untagged)]`, whose failure message names none of the keys the user
/// actually got wrong.
///
/// Scalars precede the `ssh` / `docker` sub-tables because TOML forbids a
/// value after a table within the same entry.
#[derive(Serialize, Deserialize)]
struct AgentEntryRepr {
    /// Written only as `false`: an entry that states nothing is switched on,
    /// which keeps every file written before the key existed meaning the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
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
    fold_mode: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tail_window: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tail_window_calls: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_filter: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env: Option<EnvRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh: Option<SshLaunchRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    docker: Option<DockerLaunchRepr>,
}

impl TryFrom<AgentEntryRepr> for AgentEntry {
    type Error = String;

    fn try_from(v: AgentEntryRepr) -> Result<Self, Self::Error> {
        let enabled = v.enabled.unwrap_or(true);
        let source = AgentSource::try_from(v)?;
        Ok(AgentEntry { enabled, source })
    }
}

impl TryFrom<AgentEntryRepr> for AgentSource {
    type Error = String;

    fn try_from(v: AgentEntryRepr) -> Result<Self, Self::Error> {
        if let Some(preset) = v.preset {
            return Ok(Self::Preset {
                preset,
                overrides: PresetOverrides {
                    id: v.id,
                    name: v.name,
                    command: v.command,
                    default_mode: v.default_mode,
                    default_model: v.default_model,
                    fold_mode: v.fold_mode,
                    tail_window: v.tail_window,
                    tail_window_calls: v.tail_window_calls,
                    display_filter: v.display_filter,
                    // Sanitized like a definition's own `env` (see
                    // `super::sanitized_env`): an override reaches the very
                    // same launch assembly, so it cannot be the one env entry
                    // point that skips the name check.
                    env: v.env.map(super::sanitized_env),
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
            fold_mode: v.fold_mode,
            tail_window: v.tail_window,
            tail_window_calls: v.tail_window_calls,
            display_filter: v.display_filter,
            env: v.env,
        });
        Ok(Self::for_definition(definition, None))
    }
}

impl From<AgentEntry> for AgentEntryRepr {
    fn from(v: AgentEntry) -> Self {
        Self {
            enabled: (!v.enabled).then_some(false),
            ..Self::from(v.source)
        }
    }
}

impl From<AgentSource> for AgentEntryRepr {
    fn from(v: AgentSource) -> Self {
        match v {
            AgentSource::Preset { preset, overrides } => Self {
                enabled: None,
                preset: Some(preset),
                id: overrides.id,
                name: overrides.name,
                command: overrides.command,
                default_mode: overrides.default_mode,
                default_model: overrides.default_model,
                fold_mode: overrides.fold_mode,
                tail_window: overrides.tail_window,
                tail_window_calls: overrides.tail_window_calls,
                display_filter: overrides.display_filter,
                env: overrides.env.map(|env| env.into_iter().collect()),
                ssh: None,
                docker: None,
            },
            AgentSource::Custom(definition) => {
                let repr = AgentDefinitionRepr::from(definition);
                Self {
                    enabled: None,
                    preset: None,
                    id: Some(repr.id),
                    name: Some(repr.name),
                    command: repr.command,
                    default_mode: repr.default_mode,
                    default_model: repr.default_model,
                    fold_mode: repr.fold_mode,
                    tail_window: repr.tail_window,
                    tail_window_calls: repr.tail_window_calls,
                    display_filter: repr.display_filter,
                    env: repr.env,
                    ssh: repr.ssh,
                    docker: repr.docker,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
